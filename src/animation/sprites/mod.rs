//! Animated theme sprites (TDD 27.9, WP10 of `sdd/PLAN.memory-gates.md`).
//!
//! A theme's sprite is painted DIRECTLY by the preview's paint plan
//! (`crate::decorplan`, via `codeview::bandpaint::paint_band`), not by a
//! `GdkPaintable`-backed picture widget the way a document image is
//! (`crate::animation::paintable`). It therefore has no PER-SPRITE geometry of
//! its own to watch — no independent notion of "this one decoration is on
//! screen" is invented here; that stays `decorplan`'s own viewport gate (see
//! [`frame_for`] below). The VIEW that hosts every sprite it currently draws
//! is a different question, and it does get an `animation::visibility` watch
//! (`CodePreviewView::ensure_sprite_visibility_watch`, wired from `frame_for`)
//! — the same reuse of `animation::visibility` a document image's
//! `AnimatedPaintable` uses for its own host, applied once per view rather
//! than once per sprite, because a background tab or a hidden pane is a
//! property of the WHOLE view, not of any one decoration in it (QA finding,
//! 2026-09-12: the paint-driven pruning below never sees that case at all,
//! since an unmapped widget is never painted).
//!
//! **[`frame_for`] is the one seam an animated-sprite painting site calls, and it is
//! called only from the exact point in that site's own paint where `decorplan`'s
//! viewport gate has ALREADY said "this decoration is on screen right now"
//! (`bandpaint::paint_band`'s `span.is_outside` check, for the two callers wired so
//! far — a heading's band and a disclosure summary's). That call, once per
//! `snapshot_layer` pass, IS this sprite's SCROLL-visibility signal: nothing else
//! answers "can anyone see this sprite, given the view is itself visible" here,
//! exactly per the plan's instruction to reuse the paint plan's own gates rather
//! than invent a second one for that question.
//!
//! # How a pass proves absence, not just presence
//!
//! A decoration that IS on screen calls `frame_for` and that call marks its
//! [`SpriteAnim`] SEEN for this pass. But a decoration that has scrolled OFF screen
//! calls nothing at all — `paint_band` returns before ever reaching `frame_for` — so
//! there is no direct "you are now invisible" event to react to. The pass boundary is
//! what turns silence into a verdict: `CodePreviewView::reset_sprite_anim_seen` clears
//! every entry's flag at the start of the `BelowText` layer (the first of the two
//! `snapshot_layer` runs GTK always makes for one real repaint —
//! `decorplan::PAINT_ORDER`'s own doc comment), and
//! `CodePreviewView::drop_unseen_sprite_anims` prunes whatever is STILL unmarked at the
//! end of `AboveText` (the last). An entry that survives a whole pass unmarked was, by
//! construction, never asked about — decorplan's gate refused it every time it was
//! tried — and dropping it is what releases its decoder and stops its tick callback:
//! [`SpriteAnim`]'s own `Drop` does that (via [`Self::stop_ticking`]'s `TickCallbackId`
//! removal, run implicitly by dropping `tick`, and [`policy::PolicyWatch`]'s own
//! `Drop`).
//!
//! **This pass-boundary mechanism ONLY runs from inside a real paint.** An
//! unmapped widget (a background tab, a hidden pane) is never painted at
//! all, so `snapshot_layer` never runs, `reset_sprite_anim_seen` never clears
//! anything, and `drop_unseen_sprite_anims` never prunes anything — every
//! entry this view was driving keeps its decoder and its tick callback for
//! as long as the view stays off screen, however long that is. That is
//! exactly the gap `CodePreviewView::ensure_sprite_visibility_watch` closes:
//! `animation::visibility`'s `map`/`unmap` wiring fires independently of
//! painting, so `CodePreviewView::drop_all_sprite_anims` runs the instant the
//! view itself unmaps, regardless of whether it was ever asked to paint
//! again.
//!
//! # Reused, not reimplemented (POLICY, and this WP's own instructions)
//!
//! Frame timing, loop counts and the skip-ahead-when-late rule are
//! [`schedule::Schedule`], unmodified; decoding runs on [`worker::decode_next_frame`],
//! the SAME off-main-thread bridge with the SAME "at most one decode per animation"
//! cap a document image's `AnimatedPaintable` uses; the Play Animations / "reduce
//! animations" decision is [`policy::current`]. Nothing here re-derives any of the
//! three.
//!
//! # A still sprite costs nothing new
//!
//! [`crate::sprite::animated_bytes`] answers `None` for a sprite `texture()` decoded
//! and found to be a plain still image — a single already-cached `HashMap` lookup, no
//! disk I/O, no `richimg` call — and [`frame_for`] returns the caller's OWN `natural`
//! texture unchanged in that case. A theme with no animated sprite therefore does not
//! reach a single line below this point beyond that one lookup.

use gtk::glib;
use gtk::prelude::*;
use std::sync::Arc;

use super::{policy, schedule, worker};
use crate::codeview::CodePreviewView;
use crate::sprite::SpriteRef;

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_tests;

/// One view's playback state for one animated sprite reference — the sprite
/// equivalent of `animation::paintable::AnimatedPaintable`, minus everything that
/// exists only because a picture is a `GdkPaintable`: there is no `current_image`/
/// `snapshot` vfunc to implement, because this struct never paints anything itself —
/// [`Self::current_texture`] just hands back whatever `bandpaint::paint_band` is
/// about to draw with.
///
/// Deliberately simpler than `AnimatedPaintable` in one respect, stated rather than
/// silently diverged from: pausing for GTK's "reduce animations" setting here freezes
/// on whatever frame is currently showing, the SAME as an ordinary Play Animations
/// toggle-off, rather than resetting to frame 0 the way a picture's pause overlay
/// does. A theme sprite carries no pause badge (TDD 27.9 does not ask for one, unlike
/// 27.8's picture-specific badge), so there is no on-screen indication that would
/// disagree with staying on the current frame — see this WP's report for the
/// trade-off.
pub(crate) struct SpriteAnim {
    /// The frame currently painted. `None` only in the brief window between
    /// construction succeeding and the first frame actually being available, which
    /// cannot happen here — [`Self::new`] never returns `Ok` without one.
    texture: Option<gtk::gdk::Texture>,
    animation: Option<richimg::Animation>,
    schedule: Option<schedule::Schedule>,
    /// `Some` exactly while a tick callback is installed on the host view.
    tick: Option<super::tick::TickHandle>,
    app: glib::WeakRef<gtk::Application>,
    /// Kept for its `Drop`: disconnects the action/settings subscriptions that force
    /// a repaint (`CodePreviewView::queue_draw`) on a Play-Animations/reduce-animations
    /// change, so a paused-but-visible sprite still notices "resume" even though
    /// nothing else would otherwise repaint a static preview pane.
    policy_watch: Option<policy::PolicyWatch>,
    /// Reset to `false` by `CodePreviewView::reset_sprite_anim_seen`, set `true` by
    /// [`frame_for`]. See the module doc comment's "How a pass proves absence" section.
    seen_this_pass: bool,
    /// Process-unique identity for THIS instance, captured when a frame decode is
    /// dispatched and re-checked when it completes.
    ///
    /// **A key is not an identity, which is what made the sprite case the worse of the
    /// two.** A completing decode finds its way back through
    /// `CodePreviewView::with_sprite_anim(&r, …)` — a lookup by `SpriteRef`. If the
    /// entry under that key was dropped and a new one built for the same sprite while
    /// the decode was in flight, the lookup succeeds and hands the stale frame to a
    /// DIFFERENT `SpriteAnim` that merely shares the key: its texture is replaced and
    /// its schedule advanced by a decode it never asked for. Comparing instance
    /// identity rather than trusting the key closes it (QA round 2, F-R2-2).
    generation: u64,
}

/// Source of [`SpriteAnim::generation`]. Monotonic for the life of the process, so no
/// two instances ever share an identity and a stale decode can never be mistaken for a
/// current one.
static NEXT_SPRITE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl SpriteAnim {
    /// Open `bytes` as a fresh animation and decode frame 0, exactly as
    /// `animation::paintable::open_from_frame0` does for a document image (that
    /// function is private to `paintable` and this module owns nothing there to
    /// import — the handful of lines are duplicated rather than the module boundary
    /// widened for them). `None` on any failure to open or decode it; the caller
    /// falls back to the sprite's ordinary still texture.
    fn new(bytes: Arc<[u8]>) -> Option<Self> {
        let limits = crate::imagedecode::richimg_limits();
        let mut animation = richimg::Animation::new(bytes, &limits).ok()?;
        let frame0 = animation.next_frame().ok()?;
        let delay = frame0.delay;
        let loop_count = animation.info().loop_count;
        let texture = crate::imagedecode::memory_texture_from_frame(frame0, "theme sprite")?;
        Some(SpriteAnim {
            texture: Some(texture),
            animation: Some(animation),
            schedule: Some(schedule::Schedule::start(
                glib::monotonic_time(),
                delay,
                loop_count,
            )),
            tick: None,
            app: glib::WeakRef::new(),
            policy_watch: None,
            seen_this_pass: true,
            generation: NEXT_SPRITE_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        })
    }

    /// The texture `bandpaint::paint_band` should draw for this animation right now.
    pub(crate) fn current_texture(&self) -> Option<gtk::gdk::Texture> {
        self.texture.clone()
    }

    pub(crate) fn mark_unseen(&mut self) {
        self.seen_this_pass = false;
    }

    pub(crate) fn seen_this_pass(&self) -> bool {
        self.seen_this_pass
    }

    fn mark_seen(&mut self) {
        self.seen_this_pass = true;
    }

    /// Resolve `view`'s application and subscribe to Play Animations / "reduce
    /// animations" changes, once. The subscription's ONLY job is to force a repaint
    /// (`view.queue_draw()`) — the actual play/pause decision is re-taken from
    /// scratch, every real paint, by [`Self::recompute`], which is what `frame_for`
    /// calls right after this. Without the forced repaint a sprite that is paused
    /// while visible, on an otherwise-static preview pane, would never notice the
    /// reader turning Play Animations back on until something UNRELATED caused the
    /// next paint.
    fn ensure_bootstrapped(&mut self, view: &CodePreviewView) {
        if self.policy_watch.is_some() {
            return;
        }
        let Some(app) = application_of(view.upcast_ref()) else {
            return;
        };
        self.app.set(Some(&app));
        let weak = view.downgrade();
        let watch = policy::watch(&app, move |_effective| {
            if let Some(view) = weak.upgrade() {
                view.queue_draw();
            }
        });
        self.policy_watch = Some(watch);
    }

    /// Reconcile "policy currently allows playing" and act on it — called every time
    /// `frame_for` is invoked, i.e. on every real paint that finds this decoration on
    /// screen.
    fn recompute(&mut self, view: &CodePreviewView, r: &SpriteRef) {
        self.ensure_bootstrapped(view);
        let playing = self.app.upgrade().is_some_and(|app| policy::current(&app));
        if playing {
            self.start_ticking(view, r.clone());
        } else {
            self.stop_ticking();
        }
    }

    /// Install the tick callback if one is not already running.
    fn start_ticking(&mut self, view: &CodePreviewView, r: SpriteRef) {
        if self.tick.is_some() {
            return;
        }
        let weak = view.downgrade();
        let id = view.add_tick_callback(move |_widget, frame_clock| {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let now = frame_clock.frame_time();
            view.with_sprite_anim(&r, |anim| anim.on_tick(&view, &r, now))
                .unwrap_or(glib::ControlFlow::Break)
        });
        self.tick = Some(super::tick::TickHandle::new(id));
    }

    /// Drop the handle WITHOUT removing the registration, which is correct on exactly one
    /// path: returning `glib::ControlFlow::Break` from inside the callback, where GTK has
    /// already unregistered it. Named at length because it is the one place where not
    /// removing is right, and a reader who sees it anywhere else should be suspicious.
    fn forget_tick_gtk_already_removed(&mut self) {
        if let Some(handle) = self.tick.take() {
            handle.forget();
        }
    }

    /// Retire the in-flight frame request on the schedule, if this entry still has one.
    /// See [`super::schedule::Schedule::abandon`] for why this is not `restart` — and
    /// note that this driver never calls `restart` at all, which is what makes the
    /// latch permanent here rather than merely long-lived.
    fn abandon_pending_frame(&mut self) {
        if let Some(schedule) = self.schedule.as_mut() {
            schedule.abandon();
        }
    }

    /// Remove the tick callback — the path called from OUTSIDE the callback itself
    /// (`recompute`, when policy turns playing off while visible, and the decode-failure
    /// arm of [`Self::on_decoded`]).
    ///
    /// **Dropping the handle IS the removal**, because [`super::tick::TickHandle`] owns
    /// it: there is no longer a rule about which assignment spelling is safe here, which
    /// is the whole reason that type exists. An earlier version of this comment said the
    /// opposite — that `self.tick = None` was the dangerous spelling and `remove()` the
    /// correct one — and it survived the fix that inverted it, so it was prescribing
    /// exactly the double-removal `TickHandle::forget` was introduced to prevent
    /// (QA round 2, F-AP2-2). The one path that must NOT remove is a `Break` returned
    /// from inside the callback, where GTK has already unregistered it; that path calls
    /// [`Self::forget_tick_gtk_already_removed`] and is named at length there.
    fn stop_ticking(&mut self) {
        self.tick = None;
    }

    /// One frame-clock tick. Re-checks policy on EVERY tick — not only via
    /// `policy_watch`'s repaint-forcing subscription — so a reader turning Play
    /// Animations off mid-play is honoured within about one tick (~16 ms) rather than
    /// waiting for whatever due-time the schedule happens to be sitting on; a paused
    /// sprite needs no forced repaint of its own to freeze, since the LAST painted
    /// frame simply stays on screen until something repaints it again.
    fn on_tick(
        &mut self,
        view: &CodePreviewView,
        r: &SpriteRef,
        now: schedule::Micros,
    ) -> glib::ControlFlow {
        let playing = self.app.upgrade().is_some_and(|app| policy::current(&app));
        if !playing {
            self.forget_tick_gtk_already_removed();
            return glib::ControlFlow::Break;
        }
        let action = {
            let Some(schedule) = self.schedule.as_mut() else {
                self.forget_tick_gtk_already_removed();
                return glib::ControlFlow::Break;
            };
            schedule.poll(now)
        };
        match action {
            schedule::Action::Hold => glib::ControlFlow::Continue,
            schedule::Action::Stopped => {
                self.forget_tick_gtk_already_removed();
                glib::ControlFlow::Break
            }
            schedule::Action::NeedNextFrame => {
                self.start_decode(view, r.clone());
                glib::ControlFlow::Continue
            }
        }
    }

    /// Hand the animation to `worker::decode_next_frame` and resume on the main
    /// context. A no-op if a decode is somehow already in flight (`self.animation`
    /// already `None`) — mirrors `animation::paintable::drive::start_decode`'s own
    /// defensive shape.
    fn start_decode(&mut self, view: &CodePreviewView, r: SpriteRef) {
        let Some(animation) = self.animation.take() else {
            return;
        };
        let generation = self.generation;
        let weak = view.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let (animation, result) = worker::decode_next_frame(animation).await;
            if let Some(view) = weak.upgrade() {
                let repaint = view
                    .with_sprite_anim(&r, |anim| anim.on_decoded(generation, animation, result))
                    .unwrap_or(false);
                // `on_decoded` only DECIDES whether the new frame is visible; nothing
                // inside it may hold `view`'s own `RefCell` borrow open while calling
                // back into the view (`with_sprite_anim` has just released it here),
                // so the actual repaint request happens at this outer call site.
                // Unlike a `GdkPaintable`'s `invalidate_contents()`, a self-drawn
                // decoration has no signal GTK listens for on its own — nothing else
                // would ever tell GTK this view needs to repaint the new frame.
                if repaint {
                    view.queue_draw();
                }
            }
        });
    }

    /// A decode completed — hand the animation back (always, per
    /// `worker::decode_next_frame`'s own contract) and act on the result. Returns
    /// `true` when `self.texture` actually changed, which is the caller's cue to
    /// force a repaint: unlike a `GdkPaintable`'s `invalidate_contents()`, nothing
    /// GTK owns notices a self-drawn decoration's backing data changing on its own —
    /// see [`Self::start_decode`]'s own call site for why the ACTUAL `queue_draw()`
    /// happens there rather than in here. If this entry was dropped (scrolled away)
    /// while the decode was in flight, `weak` in [`Self::start_decode`] fails to
    /// upgrade and this is simply never called — there is nothing to discard here.
    ///
    /// **Re-checks policy before ever touching `self.texture`.** A decode dispatched
    /// while playing can resolve AFTER the reader has since turned Play Animations
    /// off — `stop_ticking`/`on_tick`'s own stop path only ever prevents a FUTURE
    /// decode from starting, neither cancels one already in flight (`worker`'s own
    /// module doc comment: a dropped decode still runs to completion, its result
    /// simply discarded by whoever would have received it) — so without this a
    /// pause can visibly "leak" one more frame after the reader asked it to freeze.
    /// `schedule.present()` still runs regardless, so the schedule's own due-time
    /// bookkeeping stays correct for a later resume; only the VISIBLE texture swap is
    /// skipped.
    fn on_decoded(
        &mut self,
        generation: u64,
        animation: richimg::Animation,
        result: Result<richimg::Frame, richimg::Error>,
    ) -> bool {
        if generation != self.generation {
            // A decode dispatched by a PREVIOUS `SpriteAnim` that happened to share this
            // sprite's key. Let its animation drop here; this instance asked for
            // nothing and must not be advanced by it (F-R2-2).
            return false;
        }
        self.animation = Some(animation);
        let frame = match result {
            Ok(frame) => frame,
            Err(err) => {
                log::warn!(
                    "theme sprite: frame decode failed, freezing on the last frame shown: {err}"
                );
                // TWO things leak on this path, and they have to be retired separately.
                //
                // 1. The SCHEDULE's in-flight latch. `schedule.present` is never reached
                //    here, so `awaiting` stays set and `poll` can only ever answer
                //    `Hold` — `NeedNextFrame` is unreachable while it is set. This
                //    driver never calls `restart`, so nothing would ever clear it again:
                //    the animation is pinned dead for the life of the entry, and the
                //    next re-arm silently animates nothing (QA round 2, F-R2-1 — an
                //    earlier version of this comment named this exact consequence and
                //    then fixed only item 2).
                self.abandon_pending_frame();
                // 2. The TICK REGISTRATION — and `stop_ticking`, NOT `self.tick = None`.
                //    `TickCallbackId` has no `Drop`, so dropping the handle leaves GTK's
                //    registration installed and throws away the only thing that could
                //    remove it — and this arm is reached from the decode COMPLETION, not
                //    from inside the callback, so nothing else unregisters it. The
                //    callback would then run forever at display rate for the whole
                //    toplevel, since `Hold` is `ControlFlow::Continue`.
                self.stop_ticking();
                return false;
            }
        };
        let playing = self.app.upgrade().is_some_and(|app| policy::current(&app));
        let now = glib::monotonic_time();
        let (index, delay) = (frame.index, frame.delay);
        let show = match self.schedule.as_mut() {
            Some(s) => s.present(now, index, delay),
            None => return false,
        };
        let mut repaint = false;
        if show && playing {
            if let Some(tex) = crate::imagedecode::memory_texture_from_frame(frame, "theme sprite")
            {
                self.texture = Some(tex);
                repaint = true;
            }
        }
        if !playing {
            // The tick that requested this decode already stopped ticking, or is
            // about to on its own next poll — either way this decode's result must
            // not be visible, and there is nothing further to advance while paused.
            // `stop_ticking` rather than a bare clear for the same reason the `Err` arm
            // above uses it: one rule, one implementation. This route self-heals on the
            // next tick (`on_tick` reaches `ControlFlow::Break`), so it cost a stale
            // handle and a wasted tick rather than a pin — but the difference between
            // the two is not something a reader should have to re-derive per site.
            self.stop_ticking();
            return false;
        }
        let stopped = self
            .schedule
            .as_mut()
            .is_some_and(|s| matches!(s.poll(now), schedule::Action::Stopped));
        if stopped {
            self.stop_ticking();
        }
        repaint
    }
}

/// `view`'s `gtk::Application`, via its root window — mirrors
/// `animation::paintable::application_of`, duplicated for the same reason
/// [`SpriteAnim::new`] duplicates `open_from_frame0`.
fn application_of(view: &gtk::Widget) -> Option<gtk::Application> {
    view.root()
        .and_then(|r| r.dynamic_cast::<gtk::Window>().ok())
        .and_then(|w| w.application())
}

/// The texture `bandpaint::paint_band` should actually draw for `r`, given that the
/// caller has ALREADY confirmed (via `decorplan`'s existing viewport gate) that this
/// decoration is on screen right now — see the module doc comment for why that
/// confirmation, and nothing else, is this sprite's visibility signal.
///
/// `natural` is `crate::sprite::texture(r)`'s own frame-0 result: returned VERBATIM,
/// with no new work performed at all, whenever `r` is not animated, is not currently
/// allowed to play, or fails to open as an animation — so a still sprite's paint is
/// byte-identical to the pre-WP10 path, and a broken animated one degrades to its own
/// first frame exactly like an animated document image degrades to a `None`
/// `AnimatedPaintable`.
pub(crate) fn frame_for(
    view: &CodePreviewView,
    r: &SpriteRef,
    natural: Option<gtk::gdk::Texture>,
) -> Option<gtk::gdk::Texture> {
    let Some(bytes) = crate::sprite::animated_bytes(r) else {
        return natural;
    };
    // QA finding (2026-09-12): the paint-driven pruning `with_sprite_anim_or_insert`
    // participates in below only ever runs from INSIDE a paint, so it cannot see a
    // view that has stopped painting altogether (a background tab, a hidden pane).
    // Idempotent — see its own doc comment — so calling it on every `frame_for` costs
    // nothing once installed.
    view.ensure_sprite_visibility_watch();
    let played = view.with_sprite_anim_or_insert(
        r,
        || SpriteAnim::new(bytes),
        |anim| {
            anim.mark_seen();
            anim.recompute(view, r);
            anim.current_texture()
        },
    );
    played.flatten().or(natural)
}

/// Test-only oracle, carrying the exact cfg its only callers do
/// (`codeview::animsprite_tests`, `self::gtk_tests`) rather than a bare `#[cfg(test)]`
/// (POLICY § Unit tests) — whether a tick callback is currently installed. GTK exposes
/// no `gtk_widget_has_tick_callback` at this project's floor, so, exactly as
/// `animation::paintable::AnimatedPaintable::tick_installed` documents for pictures,
/// this is a self-report of this type's own bookkeeping rather than an independent
/// read of GTK's internal callback table.
#[cfg(all(test, feature = "gtk-integration-tests"))]
impl SpriteAnim {
    pub(crate) fn is_ticking(&self) -> bool {
        self.tick.is_some()
    }
}
