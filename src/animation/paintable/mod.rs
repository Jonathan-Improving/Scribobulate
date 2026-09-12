//! `AnimatedPaintable` — the `GdkPaintable` that actually moves (TDD 27.1, 27.4).
//!
//! One instance per on-screen animated picture (TDD "Animation state: per
//! picture, bounded by what is on screen"): it owns its own `richimg::Animation`
//! and [`schedule::Schedule`], decodes off the main thread through
//! [`worker::decode_next_frame`], and swaps its current [`gtk::gdk::MemoryTexture`]
//! on the GTK main thread when a decoded frame is due (POLICY § Architecture
//! rules, "decode off the main thread; present on the frame clock").
//!
//! # The tick-callback oracle: `gtk_widget_has_tick_callback` is not bound
//!
//! The plan this WP implements assumes `has_tick_callback()` is a readable
//! oracle for "is this animation's callback installed right now". It is not, on
//! this project's floor: `gtk_widget_has_tick_callback` does not exist in GTK
//! 4.6.9's public headers at all (`grep -n has_tick_callback
//! /usr/include/gtk-4.0/gtk/gtkwidget.h` finds nothing — only
//! `gtk_widget_add_tick_callback`/`gtk_widget_remove_tick_callback`), and
//! gtk4-rs 0.10.3 binds no such function either (absent from both
//! `gtk4-sys-0.10.3/src/*.rs` and `gtk4-0.10.3/src/widget.rs`, under every
//! feature this crate enables). So there is no independent way to ask GTK
//! "does this widget currently have my tick callback registered".
//!
//! [`AnimatedPaintable::tick_installed`] is the equivalent this module exposes
//! instead: a `Cell<bool>` this type updates itself, in lock-step with every
//! `add_tick_callback`/`TickCallbackId::remove` call it makes. **This is a
//! weaker oracle than the plan assumed** — it reports this type's own
//! bookkeeping, not an independent read of GTK's internal callback table, so a
//! bug that installed a callback without setting the flag (or vice versa)
//! would not be caught by asserting on the flag alone. The tests below pair it
//! with the corroborating fact the module doc comment for WP7b's oracle
//! verification calls for: that the displayed texture stops changing across
//! real wall-clock time once paused, and resumes changing once resumed — a
//! functional check of the frame clock's actual behaviour, not just of this
//! type's self-report.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::ObjectSubclassIsExt;
use std::sync::Arc;
use std::time::Duration;

use super::{policy, schedule, visibility, worker};
use crate::imagedecode::{memory_texture_from_frame, richimg_limits};

mod badge;
mod drive;

/// Open `bytes` as a fresh `richimg::Animation`, positioned at frame 0, with
/// frame 0's own texture and delay — the exact sequence [`AnimatedPaintable::new`]
/// performs once at construction, and the one [`drive::ensure_decoded`] repeats
/// every time visibility returns after WP8 dropped the decoder: "coming back
/// into view restarts from frame 0" (PLAN.memory-gates.md's "Animation state: per
/// picture") — never resuming mid-loop, which would mean re-decoding every delta
/// frame since the last full-canvas one.
fn open_from_frame0(
    bytes: &Arc<[u8]>,
) -> Option<(richimg::Animation, gtk::gdk::Texture, Duration)> {
    let limits = richimg_limits();
    let mut animation = richimg::Animation::new(Arc::clone(bytes), &limits).ok()?;
    let frame0 = animation.next_frame().ok()?;
    let delay = frame0.delay;
    let texture = memory_texture_from_frame(frame0, "animation")?;
    Some((animation, texture, delay))
}

mod imp {
    use super::*;
    use gtk::gdk;
    use gtk::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub(crate) struct AnimatedPaintable {
        /// The `GtkPicture` this paintable is set on. Held WEAKLY (POLICY
        /// weak-capture rule, ScrAP-60/ScrAP-155): an animation must not keep
        /// its own picture alive, and the tick callback is installed/removed
        /// on this widget, never on the paintable itself (`GdkPaintable` has
        /// no frame clock of its own).
        pub(super) host: glib::WeakRef<gtk::Widget>,
        /// Resolved once `host` has a root with a live `gtk::Application`
        /// (see [`AnimatedPaintable::try_bootstrap`]) — needed on every later
        /// policy re-check, not just the first.
        pub(super) app: glib::WeakRef<gtk::Application>,
        /// The whole ENCODED file, shared by reference with every other picture
        /// showing it (`animation::source`) — the one thing WP8's visibility
        /// controller never drops. Kept here, independent of `animation`
        /// (the decoder), for two reasons: it is what lets this picture rebuild
        /// from frame 0 after `animation`/`texture` are dropped for invisibility
        /// (see `drive::ensure_decoded`), and it is this picture's own strong
        /// hold on the shared registry entry — if the ONLY strong holder were
        /// `animation` (which WP8 drops while invisible), the registry would see
        /// zero strong holders and silently stop sharing with a second picture
        /// of the same file that asks while this one is off-screen.
        pub(super) bytes: RefCell<Option<Arc<[u8]>>>,
        /// The frame currently painted.
        pub(super) texture: RefCell<Option<gdk::Texture>>,
        /// Frame 0's own texture and delay, kept separately from `texture` so
        /// "reduce animations" can restore exactly this (TDD 27.6/27.7) without
        /// re-decoding.
        pub(super) frame0_texture: RefCell<Option<gdk::Texture>>,
        pub(super) frame0_delay: Cell<Duration>,
        /// `None` while a decode of the next frame is in flight — moved out by
        /// [`AnimatedPaintable::start_decode`] and handed back by
        /// [`AnimatedPaintable::on_decoded`]. This IS the "at most one decode in
        /// flight" guard at this layer, the same way `worker::decode_next_frame`
        /// taking `Animation` by value is the guard one layer down: there is
        /// nothing left to decode with while it is `None`.
        pub(super) animation: RefCell<Option<richimg::Animation>>,
        /// The frame's own pixel dimensions, cached OUTSIDE `texture` so
        /// `intrinsic_width`/`intrinsic_height` stay truthful to the `SIZE`
        /// flag's promise ("the intrinsic size will never change") across a
        /// WP8 visibility drop — `flags()` never changes what it returns, so
        /// the values it is promising about must not depend on whether
        /// `texture` currently exists.
        pub(super) width: Cell<i32>,
        pub(super) height: Cell<i32>,
        pub(super) schedule: RefCell<Option<schedule::Schedule>>,
        pub(super) tick: RefCell<Option<crate::animation::tick::TickHandle>>,
        /// This type's own record of whether `tick` is installed — see the
        /// module doc comment on why this, and not `has_tick_callback()`, is
        /// the oracle.
        pub(super) tick_installed: Cell<bool>,
        /// Which INCARNATION of this paintable's decoder is current. Bumped every time
        /// the decoder is torn down (`drive::drop_decoder_state`) or rebuilt
        /// (`drive::ensure_decoded`), captured when a frame decode is dispatched, and
        /// compared when that decode completes.
        ///
        /// **Identity, not visibility — and the distinction is the whole point.**
        /// `on_decoded` used to decide whether a late arrival was still wanted by
        /// reading `play_wanted`, which answers "is this picture on screen NOW". That
        /// is the right answer for lose-and-stay-lost, and the wrong one for
        /// lose-then-REGAIN: a decode in flight, the picture scrolls out (decoder
        /// dropped), scrolls back (a fresh decoder is built and the schedule
        /// restarted), and then the OLD decode lands with `play_wanted` true again.
        /// The guard sees a visible picture and waves it through, so a stale decoder
        /// overwrites the fresh one and presents a stale frame index into a freshly
        /// restarted schedule. A generation compares the decode against the decoder it
        /// was started for, which is the question actually being asked (QA round 2,
        /// F-R2-2).
        pub(super) decoder_generation: Cell<u64>,
        pub(super) policy_watch: RefCell<Option<policy::PolicyWatch>>,
        /// WP8: the live subscription to `animation::visibility`'s signals for
        /// this picture's own `host`. Set up in `try_bootstrap`, alongside
        /// `policy_watch` — dropping it (in `dispose`) disconnects everything
        /// it installed.
        pub(super) visibility_watch: RefCell<Option<visibility::VisibilityWatch>>,
        /// Disconnected in `dispose`; see [`AnimatedPaintable::try_bootstrap`].
        pub(super) root_notify: RefCell<Option<glib::SignalHandlerId>>,
        /// "Is this picture somewhere visibility considers on screen" —
        /// WP7b's own default (`true`) is what a picture starts at before
        /// `try_bootstrap` seeds it with `visibility::current`, and what
        /// `AnimatedPaintable::set_should_play` (WP8's own seam, driven by
        /// `visibility_watch`'s callback) updates from then on.
        pub(super) play_wanted: Cell<bool>,
        /// TDD 27.8's own trigger, set ONLY by `drive::recompute`'s
        /// policy-off branch: `true` exactly while playback is frozen
        /// because Play Animations is off or "reduce animations" is on —
        /// the two cases PLAN.memory-gates.md's badge section names. Never
        /// set for an invisible picture (nothing is painted there — see
        /// `recompute`'s doc comment on why that is a different state) nor
        /// for a schedule that stopped on its own after a finite loop count
        /// finished (TDD 27.1: "stays on its last frame", not a badge
        /// state this WP was asked to cover). `snapshot` below is the one
        /// reader.
        pub(super) paused_by_policy: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AnimatedPaintable {
        const NAME: &'static str = "ScribAnimatedPaintable";
        type Type = super::AnimatedPaintable;
        type ParentType = glib::Object;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for AnimatedPaintable {
        /// GObject dispose, not Rust `Drop`: a GObject's refcount can be
        /// dropped to zero from C-side code this binding does not see, and
        /// `dispose` (unlike `Drop`) is guaranteed to run exactly once at that
        /// point regardless of which side triggered it. Every step below is
        /// `.take()`-based so a second call (GLib permits `dispose` to run
        /// more than once) is a safe no-op.
        fn dispose(&self) {
            // `TickHandle`'s Drop removes the registration, so taking it is enough.
            let _ = self.tick.take();
            self.tick_installed.set(false);
            // Dropping the PolicyWatch/VisibilityWatch disconnects every
            // handler each one installed (their own guarantee) — no callback
            // into this paintable after it is gone.
            self.policy_watch.take();
            self.visibility_watch.take();
            if let (Some(host), Some(handler)) = (self.host.upgrade(), self.root_notify.take()) {
                host.disconnect(handler);
            }
            self.bytes.take();
            self.animation.take();
            self.schedule.take();
        }
    }

    impl PaintableImpl for AnimatedPaintable {
        /// The default `GdkPaintable` implementation returns the paintable
        /// ITSELF, which is correct for something whose contents never
        /// change but wrong here: "current image" is meant to be a static
        /// snapshot of the instant it is asked for, and an `AnimatedPaintable`
        /// keeps moving. Return the current frame's plain `GdkTexture`
        /// instead — itself a paintable that never changes, which is exactly
        /// the static-snapshot contract this vfunc promises.
        fn current_image(&self) -> gdk::Paintable {
            self.texture
                .borrow()
                .as_ref()
                .map(|t| t.clone().upcast())
                .unwrap_or_else(|| self.parent_current_image())
        }

        /// Reads the cached `width`/`height`, never `texture`'s own dimensions —
        /// see the field doc comment on `width`. WP8 drops `texture` while
        /// invisible; this must not change when that happens, or it breaks the
        /// promise `flags()` makes below.
        fn intrinsic_width(&self) -> i32 {
            self.width.get()
        }

        fn intrinsic_height(&self) -> i32 {
            self.height.get()
        }

        /// `SIZE` only, never `CONTENTS`: the frame size never changes over an
        /// animation's life (every richimg frame shares the canvas dimensions),
        /// but the pixels do — a `CONTENTS`-flagged paintable tells GTK it can
        /// cache a snapshot of it forever, which would freeze the very first
        /// frame on screen.
        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::SIZE
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let texture = self.texture.borrow();
            let Some(texture) = texture.as_ref() else {
                return;
            };
            // `gtk_snapshot_append_texture` is a `GtkSnapshot` method, not a
            // `GdkSnapshot` one (the interface vfunc's own parameter type) —
            // GTK always calls a paintable's snapshot with its own live
            // `GtkSnapshot`, so this downcast is expected to succeed; a `None`
            // (a caller supplying some other `GdkSnapshot` implementation)
            // degrades to painting nothing rather than panicking.
            let Some(snapshot) = snapshot.downcast_ref::<gtk::Snapshot>() else {
                return;
            };
            let rect = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
            snapshot.append_texture(texture, &rect);

            // TDD 27.8: the pause badge, painted directly here — never a
            // child widget, never a click handler (see `badge`'s module doc
            // comment). `host` supplies the display (for the icon lookup)
            // and the resolved text direction (bottom-END corner); no host
            // means nothing to paint the badge relative to, so it is
            // skipped rather than guessed at.
            if badge::should_paint(self.paused_by_policy.get(), width, height) {
                if let Some(host) = self.host.upgrade() {
                    badge::paint(snapshot, &host.display(), host.direction(), width, height);
                }
            }
        }
    }
}

glib::wrapper! {
    pub(crate) struct AnimatedPaintable(ObjectSubclass<imp::AnimatedPaintable>)
        @implements gtk::gdk::Paintable;
}

/// `host`'s `gtk::Application`, via its root window — `None` until `host` is
/// parented into a tree whose root is a window that has one (an application
/// isn't available at the moment a picture is built during a document render;
/// see [`AnimatedPaintable::try_bootstrap`]).
fn application_of(host: &gtk::Widget) -> Option<gtk::Application> {
    host.root()
        .and_then(|r| r.dynamic_cast::<gtk::Window>().ok())
        .and_then(|w| w.application())
}

impl AnimatedPaintable {
    /// Build a paintable that plays `bytes` (an animated WebP/GIF/APNG file,
    /// already sniffed by `richimg::sniff` upstream — `imagedecode::decode`'s
    /// `DecodedImage::animation` is only ever `Some` for such a file) on
    /// `host`'s frame clock. `host` is the `gtk::Picture` this paintable is
    /// about to be set on; the tick callback is installed and removed on it,
    /// held only weakly.
    ///
    /// This decodes frame 0 synchronously, ON the caller's thread — a SECOND
    /// decode of it, beyond the one `imagedecode::decode` already performed to
    /// seed the picture's still texture. That duplication is deliberate, not
    /// an oversight: this paintable owns its OWN `richimg::Animation`
    /// (decoder state is never shared, TDD "Animation state"), and its
    /// position must be advanced past frame 0 to match `Schedule::start`'s
    /// "frame 0 already on screen" contract — the very next decode this
    /// paintable performs must return frame 1, not frame 0 again. It also
    /// supplies frame 0's own delay, which `imagedecode::AnimationSource`
    /// does not carry (only `richimg::Info`, no per-frame data). Returns
    /// `None` on any failure to (re-)open or decode it, in which case the
    /// caller leaves the picture showing its ordinary still texture — the
    /// same safe degrade an unrelated decode failure gets elsewhere in this
    /// project.
    pub(crate) fn new(host: &gtk::Widget, bytes: Arc<[u8]>) -> Option<Self> {
        let (animation, texture, frame0_delay) = open_from_frame0(&bytes)?;
        let loop_count = animation.info().loop_count;
        let width = texture.width();
        let height = texture.height();

        let obj: Self = glib::Object::new();
        {
            let imp = obj.imp();
            imp.host.set(Some(host));
            imp.bytes.replace(Some(bytes));
            imp.width.set(width);
            imp.height.set(height);
            imp.texture.replace(Some(texture.clone()));
            imp.frame0_texture.replace(Some(texture));
            imp.frame0_delay.set(frame0_delay);
            imp.animation.replace(Some(animation));
            imp.schedule.replace(Some(schedule::Schedule::start(
                glib::monotonic_time(),
                frame0_delay,
                loop_count,
            )));
            imp.play_wanted.set(true);
        }

        obj.imp().try_bootstrap();
        if obj.imp().policy_watch.borrow().is_none() {
            // No application yet (the picture is being built during a
            // document render, before it is anchored into any realized
            // window) — retry once this widget's root changes. A weak
            // capture: the closure must not be what keeps `obj` alive.
            let weak = obj.downgrade();
            let handler = host.connect_root_notify(move |_| {
                if let Some(obj) = weak.upgrade() {
                    obj.imp().try_bootstrap();
                }
            });
            obj.imp().root_notify.replace(Some(handler));
        }
        Some(obj)
    }

    /// WP8's seam: "is this picture, right now, somewhere visibility considers
    /// on screen". Driven by `try_bootstrap`'s `visibility::watch` callback —
    /// a picture arranges its own visibility watching from its own `host`, so
    /// nothing outside this module calls this. It only re-asks the combined
    /// "should this be playing" question (this AND the Play Animations
    /// policy) whenever either input changes, which is what makes this call
    /// re-derive the right answer rather than needing the caller to also poke
    /// the policy machinery.
    pub(crate) fn set_should_play(&self, wanted: bool) {
        self.imp().play_wanted.set(wanted);
        self.imp().recompute();
    }

    /// Whether a tick callback is currently installed. See the module doc
    /// comment for why this — a self-reported flag — stands in for
    /// `gtk_widget_has_tick_callback`, which does not exist at this project's
    /// GTK floor nor in the gtk4-rs binding.
    #[allow(
        dead_code,
        reason = "the test oracle for play/pause/reduce-animations below; no production caller"
    )]
    pub(crate) fn tick_installed(&self) -> bool {
        self.imp().tick_installed.get()
    }

    /// WP8's own test oracle, the same shape as [`Self::tick_installed`]
    /// above: whether this paintable currently holds a `richimg::Animation`
    /// (the decoder AND canvas, per PLAN.memory-gates.md's "Each on-screen
    /// animation owns its decoder and one working canvas") — `false` exactly
    /// while `drive::drop_decoder_state` has cleared it for invisibility.
    /// `animation` is private to `imp`, so nothing outside this module can
    /// observe this any other way; a test asserting "the decoder was
    /// released" needs a direct read the same way `tick_installed` needed one
    /// for "the tick callback was removed" (`gtk_widget_has_tick_callback`
    /// does not exist either).
    #[allow(
        dead_code,
        reason = "the test oracle for WP8's visibility-driven decoder release; no production caller"
    )]
    pub(crate) fn decoder_active(&self) -> bool {
        self.imp().animation.borrow().is_some()
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod badge_tests;
#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_tests;
