//! The seam this paintable actually drives: bootstrap, the play/pause/visibility
//! reconciliation, the tick callback, and the decode round trip. Split out of
//! `mod.rs` at POLICY's 500-line soft limit — the struct/`GdkPaintable` vfuncs and
//! the public `AnimatedPaintable` API stay there; every method below is
//! `impl imp::AnimatedPaintable` (the same type, a different file, exactly like
//! `gtk_tests` splits the tests for the same reason).

use super::imp;
use super::{application_of, open_from_frame0, policy, schedule, visibility, worker};
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::{ObjectSubclassExt, ObjectSubclassIsExt};

impl imp::AnimatedPaintable {
    /// Resolve `host`'s `gtk::Application`, subscribe to [`policy::watch`] and
    /// [`visibility::watch`], and start ticking if both currently allow it.
    /// Idempotent — a no-op once already bootstrapped — because `notify::root`
    /// can fire more than once over a widget's life (e.g. a reparent), and
    /// because [`super::AnimatedPaintable::new`] calls this eagerly before ever
    /// knowing whether it will need the fallback `connect_root_notify` retry.
    pub(super) fn try_bootstrap(&self) {
        if self.policy_watch.borrow().is_some() {
            return;
        }
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let Some(app) = application_of(&host) else {
            return;
        };
        self.app.set(Some(&app));

        let obj = self.obj();
        let watch = policy::watch(
            &app,
            glib::clone!(
                #[weak]
                obj,
                move |_effective| obj.imp().recompute()
            ),
        );
        self.policy_watch.replace(Some(watch));

        // A picture arranges its own visibility watching from its own
        // `host` — `AnimatedPaintable::set_should_play` is this module's one
        // seam for it, so wiring lives here rather than at any picture-
        // construction call site (`renderer::start` never mentions
        // `animation::visibility` at all).
        let visibility_watch = visibility::watch(
            &host,
            glib::clone!(
                #[weak]
                obj,
                move |visible| obj.set_should_play(visible)
            ),
        );
        self.visibility_watch.replace(Some(visibility_watch));

        // Seed with the CURRENT visibility rather than leaving `play_wanted`
        // at its construction default of `true` until the next signal fires —
        // a picture built already scrolled away, backgrounded, or minimized
        // must not autoplay (and must not hold a decoder) for one recompute
        // before anything else says otherwise. `set_should_play` itself calls
        // `recompute`, so this is also what performs the first recompute.
        obj.set_should_play(visibility::current(&host));
    }

    /// The single point that reconciles "is this picture visible" with
    /// "does policy currently allow playing" and acts on the result — called
    /// on bootstrap, on every `policy::watch`/`visibility::watch` firing, and
    /// by [`super::AnimatedPaintable::set_should_play`].
    ///
    /// Visibility is checked FIRST and is unconditional: an invisible picture
    /// drops its decoder and canvas regardless of what policy says, because
    /// there is nothing to preserve visually for a picture nobody can see —
    /// dropping everything the paintable owns beyond the shared bytes is
    /// not qualified by whether Play Animations
    /// happened to be off at the time. A policy-only pause (visible, but
    /// paused) is the FREEZE case ([`Self::maybe_reset_for_reduce_animations`]
    /// only actually resets for the "reduce animations" system setting, never
    /// for the reader's own toggle) — the two are visually different states
    /// and this is the one function that tells them apart.
    ///
    /// `pub(super)`, not private: `AnimatedPaintable::set_should_play` (in
    /// `mod.rs`, `drive`'s PARENT module) calls this directly.
    pub(super) fn recompute(&self) {
        if !self.play_wanted.get() {
            self.stop_ticking();
            self.drop_decoder_state();
            // Nothing is painted for an invisible picture (see this
            // function's own doc comment) — not "paused" in TDD 27.8's
            // sense, so the badge flag must not carry over from a spell
            // where it WAS paused-by-policy before going off screen.
            self.paused_by_policy.set(false);
            return;
        }
        self.ensure_decoded();
        let playing = self.app.upgrade().is_some_and(|app| policy::current(&app));
        // TDD 27.8's badge must appear/disappear the INSTANT policy flips,
        // not whenever the next frame swap happens to invalidate the
        // paintable next — `maybe_reset_for_reduce_animations` below only
        // invalidates for ITS OWN sub-case (the system setting), so an
        // ordinary reader toggle-off would otherwise sit un-repainted until
        // some unrelated invalidation came along. Compared against the
        // PREVIOUS value so an unchanged re-fire (two policy signals in one
        // turn, TDD 27.4's own scenario) costs nothing extra.
        let was_paused = self.paused_by_policy.get();
        if playing {
            self.paused_by_policy.set(false);
            self.start_ticking();
        } else {
            self.stop_ticking();
            // `policy::current` merges BOTH triggers TDD 27.8 names — the
            // reader's own Play Animations toggle and the system's "reduce
            // animations" setting — so "not playing, but visible and
            // decoded" is exactly the badge's condition; a schedule that
            // stopped on its own (a finished finite loop count, TDD 27.1)
            // never reaches this branch at all, since nothing here re-calls
            // `recompute` for that — see `on_tick`/`on_decoded`.
            self.paused_by_policy.set(true);
            self.maybe_reset_for_reduce_animations();
        }
        if was_paused != self.paused_by_policy.get() {
            self.obj().invalidate_contents();
        }
    }

    /// Everything this paintable owns beyond the shared encoded bytes,
    /// dropped — the decoder AND the canvas (both live inside `animation`;
    /// each on-screen animation owns one decoder and one working canvas),
    /// plus the currently-displayed and frame-0
    /// textures. `bytes` stays (the one thing kept), and so does `schedule` —
    /// it holds only a due-time and small counters, no decoded pixels, and
    /// [`Self::ensure_decoded`] resets it with `Schedule::restart` rather than
    /// rebuilding it, so a `LoopCount::Finite` animation's remaining budget is
    /// not silently replenished every time a picture leaves and returns to
    /// view. `width`/`height` also stay — see their field doc comment: the
    /// paintable's declared intrinsic size must not change while this is
    /// dropped, or a picture scrolled out and back would reflow the document
    /// on every round trip.
    fn drop_decoder_state(&self) {
        // Any decode still in flight was started for the decoder being torn down here;
        // its result must not be applied to whatever replaces it (F-R2-2).
        self.bump_decoder_generation();
        self.animation.take();
        self.texture.take();
        self.frame0_texture.take();
    }

    /// Rebuild the decoder, canvas and schedule from the shared encoded bytes
    /// if [`Self::drop_decoder_state`] cleared them — "coming back into
    /// view restarts from frame 0", never resuming mid-loop. A no-op if a
    /// decoder is already held (the ordinary case: visibility was never lost,
    /// or this already ran for the current visible spell).
    fn ensure_decoded(&self) {
        if self.animation.borrow().is_some() {
            return;
        }
        // A NEW decoder incarnation begins below. Bumping here as well as in
        // `drop_decoder_state` means a rebuild is fenced off even on any path that
        // reaches it without a matching teardown.
        self.bump_decoder_generation();
        let Some(bytes) = self.bytes.borrow().clone() else {
            // Should not happen post-construction (`bytes` is set once in
            // `AnimatedPaintable::new` and never cleared before `dispose`) —
            // nothing to rebuild from, so leave the picture showing nothing
            // rather than panicking.
            return;
        };
        let Some((animation, texture, delay)) = open_from_frame0(&bytes) else {
            log::warn!(
                "animated image: re-decode after a visibility round trip failed; leaving the \
                 picture blank rather than retrying every recompute"
            );
            return;
        };
        self.width.set(texture.width());
        self.height.set(texture.height());
        self.texture.replace(Some(texture.clone()));
        self.frame0_texture.replace(Some(texture));
        self.frame0_delay.set(delay);
        self.animation.replace(Some(animation));
        {
            let mut schedule = self.schedule.borrow_mut();
            match schedule.as_mut() {
                Some(schedule) => schedule.restart(glib::monotonic_time(), delay),
                None => {
                    // Should not happen (the schedule survives a visibility
                    // drop — see `drop_decoder_state`) — build a fresh one
                    // in place, through the SAME borrow, so playback can
                    // still start rather than silently never ticking.
                    *schedule = Some(schedule::Schedule::start(
                        glib::monotonic_time(),
                        delay,
                        richimg::LoopCount::Infinite,
                    ));
                }
            }
        }
        self.obj().invalidate_contents();
    }

    /// Install the tick callback, if it is not already installed. A no-op
    /// otherwise — [`Self::recompute`] can run more than once while playing
    /// (e.g. two policy signals firing in the same turn) and must not install
    /// a second callback each time.
    fn start_ticking(&self) {
        if self.tick_installed.get() {
            return;
        }
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let obj = self.obj();
        let id = host.add_tick_callback(glib::clone!(
            #[weak]
            obj,
            #[upgrade_or]
            glib::ControlFlow::Break,
            move |_widget, frame_clock| obj.imp().on_tick(frame_clock)
        ));
        self.tick
            .replace(Some(crate::animation::tick::TickHandle::new(id)));
        self.tick_installed.set(true);
    }

    /// Remove the tick callback if one is installed — the ONLY path that
    /// calls [`gtk::TickCallbackId::remove`] explicitly. The other path a
    /// callback stops on (returning [`glib::ControlFlow::Break`] from
    /// [`Self::on_tick`] once the schedule reports
    /// [`schedule::Action::Stopped`]) already self-unregisters with GTK, so it
    /// clears the bookkeeping directly instead of calling this — calling
    /// `remove` on an id GTK already forgot would be a second removal of the
    /// same registration.
    fn stop_ticking(&self) {
        // `TickHandle`'s own Drop calls `remove()`, so dropping the handle IS the
        // removal — see `animation::tick` for why the contract is a type rather than a
        // rule every site has to remember.
        let _ = self.tick.take();
        self.tick_installed.set(false);
    }

    /// TDD 27.6/27.7: when the reason playback stopped is specifically GTK's
    /// own "reduce animations" system setting (`gtk-enable-animations` is
    /// `false`) — as opposed to an ordinary reader toggle-off, which freezes
    /// on whatever frame was showing — reset the DISPLAY to frame 0 and the
    /// underlying decoder's position to match, so that if reduce-animations
    /// is later turned back on playback resumes cleanly from the start
    /// rather than from an arbitrary mid-sequence position with no visible
    /// indication of where it is.
    ///
    /// Reads `gtk::Settings` directly rather than through `policy::current` —
    /// that function collapses "the reader's own choice" and "the system
    /// setting" into one effective boolean, and this decision needs to tell
    /// the two apart (`policy.rs`'s private helper doing the equivalent read
    /// is not `pub(crate)`, and widening it buys nothing here —
    /// this reads the same public GTK property directly instead).
    ///
    /// Only reachable while VISIBLE (`recompute` calls this only after
    /// `ensure_decoded`, on the visible-but-not-playing branch) — an
    /// invisible picture has already had its decoder dropped entirely by
    /// [`Self::drop_decoder_state`], which is the stronger of the two resets.
    fn maybe_reset_for_reduce_animations(&self) {
        let reduced = gtk::Settings::default().is_none_or(|s| !s.is_gtk_enable_animations());
        if !reduced {
            return;
        }
        if let Some(animation) = self.animation.borrow_mut().as_mut() {
            animation.rewind();
        }
        if let Some(texture) = self.frame0_texture.borrow().clone() {
            self.texture.replace(Some(texture));
            self.obj().invalidate_contents();
        }
        if let Some(schedule) = self.schedule.borrow_mut().as_mut() {
            schedule.restart(glib::monotonic_time(), self.frame0_delay.get());
        }
    }

    /// Start a new decoder incarnation, invalidating any decode already in flight.
    /// Saturating rather than wrapping: at one bump per visibility round trip, `u64`
    /// does not realistically wrap, and saturating removes the question entirely rather
    /// than leaving a reviewer to reason about an ABA at the boundary.
    fn bump_decoder_generation(&self) {
        self.decoder_generation
            .set(self.decoder_generation.get().saturating_add(1));
    }

    /// Retire the in-flight frame request on the schedule, if there is still a schedule.
    /// See [`schedule::Schedule::abandon`] for why this is not `restart`.
    fn abandon_pending_frame(&self) {
        if let Some(schedule) = self.schedule.borrow_mut().as_mut() {
            schedule.abandon();
        }
    }

    /// Retire this paintable's tick registration from INSIDE the tick callback, and
    /// return the `ControlFlow::Break` that does the unregistering.
    ///
    /// GTK has already unregistered the callback by the time a `Break` takes effect, so
    /// the handle is **forgotten rather than removed** — dropping it normally would
    /// remove one registration twice. Every `Break` path goes through here so that the
    /// two pieces of state and the return value can never be updated in three different
    /// combinations by three different arms (they were, and one arm updated none of
    /// them).
    fn retire_tick(&self) -> glib::ControlFlow {
        if let Some(handle) = self.tick.take() {
            handle.forget();
        }
        self.tick_installed.set(false);
        glib::ControlFlow::Break
    }

    /// One frame clock tick: ask the schedule what to do, and act on it.
    /// `ControlFlow::Continue` keeps the callback installed for the next
    /// tick; `ControlFlow::Break` self-unregisters (paired with clearing
    /// `tick`/`tick_installed` directly — see [`Self::stop_ticking`]'s doc
    /// comment on why that path never calls `remove`).
    fn on_tick(&self, frame_clock: &gtk::gdk::FrameClock) -> glib::ControlFlow {
        let now = frame_clock.frame_time();
        let action = {
            let mut schedule = self.schedule.borrow_mut();
            let Some(schedule) = schedule.as_mut() else {
                // EVERY `Break` out of this function must retire the registration
                // state, not just the expected one. This arm used to return bare,
                // leaving `tick` holding a `TickHandle` for a callback GTK had
                // already unregistered (so its `Drop` would remove the registration a
                // second time) and `tick_installed` stuck `true` (so nothing could
                // ever re-arm the clock for this paintable again).
                return self.retire_tick();
            };
            schedule.poll(now)
        };
        match action {
            schedule::Action::Hold => glib::ControlFlow::Continue,
            schedule::Action::Stopped => self.retire_tick(),
            schedule::Action::NeedNextFrame => {
                self.start_decode();
                glib::ControlFlow::Continue
            }
        }
    }

    /// Hand the animation to [`worker::decode_next_frame`] and resume on the
    /// main context once it completes. A no-op if a decode is somehow already
    /// in flight (`self.animation` already `None`) — should not happen, since
    /// [`schedule::Schedule::poll`] only ever answers
    /// [`schedule::Action::NeedNextFrame`] once per outstanding request, but
    /// this is the same defensive shape `worker::decode_next_frame` itself
    /// uses (taking `Animation` by value rather than trusting a flag).
    fn start_decode(&self) {
        let Some(animation) = self.animation.take() else {
            return;
        };
        let generation = self.decoder_generation.get();
        // A manual weak upgrade rather than `glib::clone!` around the async
        // block: this closure needs no `#[upgrade_or]` default because it
        // simply does nothing when the paintable is gone (the picture, and
        // therefore this paintable, has been dropped mid-decode — the decode
        // itself still runs to completion on the pool thread per
        // `worker::decode_next_frame`'s own contract, with its result
        // discarded here).
        let weak = self.obj().downgrade();
        glib::MainContext::default().spawn_local(async move {
            let (animation, result) = worker::decode_next_frame(animation).await;
            if let Some(obj) = weak.upgrade() {
                obj.imp().on_decoded(generation, animation, result);
            }
        });
    }

    /// A decode completed: hand the animation back (always — a poisoned or
    /// exhausted one is still this paintable's to hold, per
    /// `worker::decode_next_frame`'s own contract), then act on the result.
    ///
    /// **If visibility was lost while this decode was in flight, `animation`
    /// is DISCARDED here rather than stored.** `stop_ticking` already ran
    /// when `visibility::watch` fired invisible (`recompute`'s
    /// `!self.play_wanted.get()` branch), and — unlike a policy-only pause —
    /// nothing fires again while the picture stays off screen, so there is
    /// no later `recompute` waiting to drop a resurrected decoder: a
    /// previous version of this comment claimed one, and QA (2026-09-12)
    /// found that claim false. Storing the late arrival would also poison
    /// [`Self::ensure_decoded`]'s "already decoded" early return
    /// (`self.animation.borrow().is_some()`), so the next return to view
    /// would resume mid-sequence instead of restarting at frame 0 (TDD
    /// 27.3). This is where `worker::decode_next_frame`'s own contract — a
    /// dropped decode still runs to completion and hands its result back —
    /// meets the discard: the animation (and, with it, its decoder) is
    /// simply let go instead of being replaced into `self.animation`.
    ///
    /// **Two guards, asking two different questions, and both are needed.**
    ///
    /// The first is IDENTITY: does this frame belong to the decoder that is current?
    /// `generation` is captured at dispatch and compared here, so a decode outlived by
    /// its own decoder is dropped whatever the picture's visibility now says. This
    /// catches lose-then-REGAIN, where a visibility test cannot: by the time the stale
    /// frame lands the picture is on screen again, with a fresh decoder and a restarted
    /// schedule that the stale frame would corrupt (F-R2-2).
    ///
    /// The second is VISIBILITY: `play_wanted`, which is set only by
    /// [`super::AnimatedPaintable::set_should_play`], itself driven only by
    /// `visibility::watch`'s callback (see [`Self::try_bootstrap`]) — never a second
    /// notion of "visible"; `animation::visibility` owns the decision and this only
    /// reads its last answer. It catches the lose-and-stay-lost case, where the
    /// generation still matches because no rebuild has happened yet.
    fn on_decoded(
        &self,
        generation: u64,
        animation: richimg::Animation,
        result: Result<richimg::Frame, richimg::Error>,
    ) {
        if generation != self.decoder_generation.get() {
            // This frame belongs to a decoder incarnation that has since been torn
            // down. Let `animation` — the STALE decoder — drop here. Nothing is
            // abandoned on the schedule: the schedule was restarted by
            // `ensure_decoded` when the current incarnation was built, so it holds no
            // request of this decode's (F-R2-2).
            return;
        }
        if !self.play_wanted.get() {
            // Let `animation` (and its decoder) drop right here. `recompute`
            // already cleared `texture`/`frame0_texture` and stopped the
            // tick the instant visibility was lost — there is nothing left
            // for this late arrival to undo.
            //
            // Except the schedule's in-flight latch: the frame this driver asked for
            // is never going to be presented, so the request has to be retired or the
            // schedule holds `awaiting` forever and the next re-arm can only ever
            // answer `Hold` (F-R2-1).
            self.abandon_pending_frame();
            return;
        }
        self.animation.replace(Some(animation));
        let frame = match result {
            Ok(frame) => frame,
            Err(err) => {
                log::warn!(
                    "animated image: frame decode failed, freezing on the last frame shown: {err}"
                );
                // Retire the request before stopping: `stop_ticking` removes the
                // registration but leaves the schedule's latch set, so a later re-arm
                // would find it pinned in `Hold` (F-R2-1).
                self.abandon_pending_frame();
                self.stop_ticking();
                return;
            }
        };
        let now = glib::monotonic_time();
        let index = frame.index;
        let delay = frame.delay;
        let show = {
            let mut schedule = self.schedule.borrow_mut();
            let Some(schedule) = schedule.as_mut() else {
                return;
            };
            schedule.present(now, index, delay)
        };
        if show {
            if let Some(texture) = super::memory_texture_from_frame(frame, "animation") {
                self.texture.replace(Some(texture));
                self.obj().invalidate_contents();
            }
        }
        // Re-poll at the same `now` purely to learn whether THIS `present`
        // was the one that just stopped the schedule (the last play of a
        // finite loop count) — `poll` is documented idempotent between
        // `present` calls, so this reads current state rather than
        // requesting a frame.
        let stopped = {
            let mut schedule = self.schedule.borrow_mut();
            schedule
                .as_mut()
                .is_some_and(|s| matches!(s.poll(now), schedule::Action::Stopped))
        };
        if stopped {
            self.stop_ticking();
        }
    }
}
