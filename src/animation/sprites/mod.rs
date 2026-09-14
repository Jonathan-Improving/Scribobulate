//! Animated theme sprites (TDD 27.9).
//!
//! A theme's sprite is painted DIRECTLY — by the preview's paint plan
//! (`crate::decorplan`), or by a widget's own `snapshot` (`widgets::rule`,
//! `widgets::table`, `widgets::sprite_icon`) — never by a `GdkPaintable`-backed picture
//! the way a document image is (`crate::animation::paintable`). So a sprite has no
//! geometry of its own to watch. What it has is a HOST: the widget whose paint draws it.
//!
//! # One table per host, one seam per paint
//!
//! A host owns a [`SpriteTable`] — one [`SpriteAnim`] per animated sprite reference it
//! currently draws — and brackets each of its paints with [`SpriteTable::begin_pass`]
//! and [`SpriteTable::end_pass`]. Everything a paint draws reaches pixels through
//! [`Frames`], which [`SpriteTable::frames`] hands out for that pass: [`Frames::natural`]
//! for a sprite tiled at its natural size, [`Frames::scaled`] for one resampled to a box
//! the layout chose. For a still sprite both are exactly `sprite::texture` and
//! `sprite::scaled`; for an animated one they answer the current frame.
//!
//! **A `Frames` call is only made from the point in a paint where that decoration is
//! already known to be on screen** — the paint plan's viewport gates (`span.is_outside`,
//! `row_on_screen`, …) for the preview, and for a widget host the whole widget, whose
//! own visibility the table asks once per pass (`animation::visibility::current`,
//! because an anchored child is still snapshotted when scrolled away). That call IS the
//! sprite's scroll-visibility signal: nothing else answers "can anyone see this sprite".
//!
//! # How a pass proves absence, not just presence
//!
//! A decoration on screen calls `Frames` and marks its [`SpriteAnim`] SEEN. One that has
//! scrolled off calls nothing at all, so there is no "you are now invisible" event. The
//! pass boundary turns that silence into a verdict: `begin_pass` clears every flag,
//! `end_pass` drops whatever is still unmarked, and dropping a `SpriteAnim` releases its
//! decoder and removes its tick callback (`TickHandle`'s and `PolicyWatch`'s own `Drop`).
//!
//! **That only runs from inside a real paint.** An unmapped host (a background tab, a
//! hidden pane) is never painted, so the table also carries an `animation::visibility`
//! watch on its host, installed the first time an animated sprite is asked for. When the
//! host is unmapped or minimized the watch drops every entry itself, since no paint will
//! come to do it. Any other report — scrolling, resizing — can only have changed
//! geometry, which a paint measures more reliably than the watch can, so the watch just
//! asks the host to repaint (when a pass refused to play, or something is playing) and
//! the pass decides. Without that request a widget's cached render node would go on
//! showing the frame it froze on, with nothing left to start it.
//!
//! # Reused, not reimplemented
//!
//! Frame timing, loop counts and skip-when-late are [`schedule::Schedule`]; decoding is
//! [`worker::decode_next_frame`], with the same one-decode-per-animation cap a document
//! image uses; play/pause is [`policy::current`]. Nearest-neighbour resampling is
//! `imagedecode::resample_nearest`, the one `sprite::scaled` uses, so an animated sprite
//! resamples exactly as its still form would.
//!
//! # A still sprite costs nothing new
//!
//! [`crate::sprite::animated_bytes`] answers `None` for a still sprite with one cached
//! lookup, and `Frames` then returns what `sprite::texture`/`sprite::scaled` would have
//! returned before this module existed.

use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use super::{policy, schedule, visibility, worker};
use crate::imagedecode::FramePixels;
use crate::sprite::SpriteRef;

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_tests;
#[cfg(all(test, feature = "gtk-integration-tests"))]
pub(crate) mod testkit;

/// How many distinct resampled sizes one animation keeps for its CURRENT frame. Every
/// size is re-derived per frame, so this bounds the per-frame work a host whose sizes
/// vary (a chip sized by its row) can ask for; past it the cache starts over rather
/// than growing.
const MAX_SCALED_SIZES: usize = 16;

/// One host's playback state for one animated sprite reference — the sprite
/// equivalent of `animation::paintable::AnimatedPaintable`, minus everything that
/// exists only because a picture is a `GdkPaintable`.
///
/// Deliberately simpler than `AnimatedPaintable` in one respect: pausing for "reduce
/// animations" freezes on the frame showing, the same as a Play Animations toggle-off,
/// rather than resetting to frame 0. A theme sprite carries no pause badge (TDD 27.8 is
/// the picture's), so nothing on screen would disagree with staying on the current frame.
pub(crate) struct SpriteAnim {
    /// The frame currently painted, as pixels (for resampling) and as a texture.
    pixels: FramePixels,
    texture: gtk::gdk::Texture,
    /// The current frame resampled, per `(w, h)`. Cleared whenever the frame changes.
    scaled: HashMap<(i32, i32), Option<gtk::gdk::Texture>>,
    animation: Option<richimg::Animation>,
    schedule: Option<schedule::Schedule>,
    /// `Some` exactly while a tick callback is installed on the host.
    tick: Option<super::tick::TickHandle>,
    app: glib::WeakRef<gtk::Application>,
    /// Kept for its `Drop`: the subscription that forces a repaint of the host on a
    /// Play Animations / reduce-animations change, so a paused-but-visible sprite still
    /// notices "resume" on an otherwise static pane.
    policy_watch: Option<policy::PolicyWatch>,
    seen_this_pass: bool,
    /// Process-unique identity, captured when a decode is dispatched and re-checked on
    /// completion. A key is not an identity: an entry dropped and rebuilt for the same
    /// sprite while a decode was in flight shares the key, and must not be advanced by
    /// a decode it never asked for (QA round 2, F-R2-2).
    generation: u64,
}

/// Source of [`SpriteAnim::generation`]. Monotonic for the life of the process.
static NEXT_SPRITE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What a tick callback or a decode completion holds to reach back into its entry:
/// both halves weak, so neither keeps the host or its table alive.
#[derive(Clone)]
struct Backref {
    host: glib::WeakRef<gtk::Widget>,
    table: Weak<TableInner>,
    sprite: SpriteRef,
}

impl Backref {
    fn with_anim<R>(&self, f: impl FnOnce(&mut SpriteAnim) -> R) -> Option<R> {
        let table = self.table.upgrade()?;
        let mut anims = table.anims.borrow_mut();
        anims.get_mut(&self.sprite).map(f)
    }
}

impl SpriteAnim {
    /// Open `bytes` and decode frame 0. `None` on any failure; the caller then paints
    /// the sprite's ordinary still texture.
    fn new(bytes: Arc<[u8]>) -> Option<Self> {
        let limits = crate::imagedecode::richimg_limits();
        let mut animation = richimg::Animation::new(bytes, &limits).ok()?;
        let frame0 = animation.next_frame().ok()?;
        let delay = frame0.delay;
        let loop_count = animation.info().loop_count;
        let pixels = FramePixels::of(frame0, "theme sprite")?;
        let texture = pixels.texture();
        Some(SpriteAnim {
            pixels,
            texture,
            scaled: HashMap::new(),
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

    /// The current frame at its natural size.
    pub(crate) fn current_texture(&self) -> gtk::gdk::Texture {
        self.texture.clone()
    }

    /// The current frame resampled to exactly `w × h`, nearest-neighbour. The natural
    /// size is answered with the frame itself, as `sprite::scaled` would for a still.
    fn scaled_texture(&mut self, w: i32, h: i32) -> Option<gtk::gdk::Texture> {
        if (w, h) == (self.pixels.width(), self.pixels.height()) {
            return Some(self.texture.clone());
        }
        if !self.scaled.contains_key(&(w, h)) && self.scaled.len() >= MAX_SCALED_SIZES {
            self.scaled.clear();
        }
        let pixels = &self.pixels;
        self.scaled
            .entry((w, h))
            .or_insert_with(|| pixels.resampled(w, h))
            .clone()
    }

    fn present(&mut self, pixels: FramePixels) {
        self.texture = pixels.texture();
        self.pixels = pixels;
        self.scaled.clear();
    }

    /// Resolve the host's application and subscribe to policy changes, once. The
    /// subscription only forces a repaint; the decision is re-taken every paint.
    fn ensure_bootstrapped(&mut self, host: &gtk::Widget) {
        if self.policy_watch.is_some() {
            return;
        }
        let Some(app) = application_of(host) else {
            return;
        };
        self.app.set(Some(&app));
        let weak = host.downgrade();
        let watch = policy::watch(&app, move |_effective| {
            if let Some(host) = weak.upgrade() {
                host.queue_draw();
            }
        });
        self.policy_watch = Some(watch);
    }

    fn playing(&self) -> bool {
        self.app.upgrade().is_some_and(|app| policy::current(&app))
    }

    /// Reconcile "policy allows playing" and act on it — every paint that finds this
    /// decoration on screen.
    fn recompute(&mut self, back: &Backref, host: &gtk::Widget) {
        self.ensure_bootstrapped(host);
        if self.playing() {
            self.start_ticking(back, host);
        } else {
            self.stop_ticking();
        }
    }

    fn start_ticking(&mut self, back: &Backref, host: &gtk::Widget) {
        if self.tick.is_some() {
            return;
        }
        let back = back.clone();
        let id = host.add_tick_callback(move |_widget, frame_clock| {
            let now = frame_clock.frame_time();
            back.with_anim(|anim| anim.on_tick(&back, now))
                .unwrap_or(glib::ControlFlow::Break)
        });
        self.tick = Some(super::tick::TickHandle::new(id));
    }

    /// Drop the handle WITHOUT removing the registration — correct only when returning
    /// `ControlFlow::Break` from inside the callback, where GTK has already removed it.
    fn forget_tick_gtk_already_removed(&mut self) {
        if let Some(handle) = self.tick.take() {
            handle.forget();
        }
    }

    /// Remove the tick callback from OUTSIDE the callback. Dropping the handle IS the
    /// removal ([`super::tick::TickHandle`]).
    fn stop_ticking(&mut self) {
        self.tick = None;
    }

    /// One frame-clock tick. Re-checks policy every tick, so a pause is honoured within
    /// about one frame rather than at the schedule's next due time.
    fn on_tick(&mut self, back: &Backref, now: schedule::Micros) -> glib::ControlFlow {
        if !self.playing() {
            self.forget_tick_gtk_already_removed();
            return glib::ControlFlow::Break;
        }
        let Some(schedule) = self.schedule.as_mut() else {
            self.forget_tick_gtk_already_removed();
            return glib::ControlFlow::Break;
        };
        match schedule.poll(now) {
            schedule::Action::Hold => glib::ControlFlow::Continue,
            schedule::Action::Stopped => {
                self.forget_tick_gtk_already_removed();
                glib::ControlFlow::Break
            }
            schedule::Action::NeedNextFrame => {
                self.start_decode(back);
                glib::ControlFlow::Continue
            }
        }
    }

    /// Hand the animation to the worker and resume on the main context. A no-op if a
    /// decode is already in flight (`animation` already taken).
    fn start_decode(&mut self, back: &Backref) {
        let Some(animation) = self.animation.take() else {
            return;
        };
        let generation = self.generation;
        let back = back.clone();
        glib::MainContext::default().spawn_local(async move {
            let (animation, result) = worker::decode_next_frame(animation).await;
            let repaint = back
                .with_anim(|anim| anim.on_decoded(generation, animation, result))
                .unwrap_or(false);
            // Outside `with_anim`, so no table borrow is held while GTK is asked to
            // repaint. A self-drawn decoration has no `invalidate-contents` GTK listens
            // for: nothing else would tell the host to paint the new frame.
            if repaint {
                if let Some(host) = back.host.upgrade() {
                    host.queue_draw();
                }
            }
        });
    }

    /// A decode completed — take the animation back and act on the result. `true` when
    /// the visible frame changed. Re-checks policy before swapping the frame, because a
    /// decode dispatched while playing can land after the reader paused.
    fn on_decoded(
        &mut self,
        generation: u64,
        animation: richimg::Animation,
        result: Result<richimg::Frame, richimg::Error>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        self.animation = Some(animation);
        let frame = match result {
            Ok(frame) => frame,
            Err(err) => {
                log::warn!(
                    "theme sprite: frame decode failed, freezing on the last frame shown: {err}"
                );
                // Two things leak on this path and each needs retiring: the schedule's
                // in-flight latch (never cleared here, so `poll` would answer `Hold`
                // forever — F-R2-1) and the tick registration (reached from the decode
                // completion, not from inside the callback, so nothing else removes it).
                if let Some(schedule) = self.schedule.as_mut() {
                    schedule.abandon();
                }
                self.stop_ticking();
                return false;
            }
        };
        let playing = self.playing();
        let now = glib::monotonic_time();
        let (index, delay) = (frame.index, frame.delay);
        let show = match self.schedule.as_mut() {
            Some(s) => s.present(now, index, delay),
            None => return false,
        };
        if !playing {
            self.stop_ticking();
            return false;
        }
        let mut repaint = false;
        if show {
            if let Some(pixels) = FramePixels::of(frame, "theme sprite") {
                self.present(pixels);
                repaint = true;
            }
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

/// `host`'s `gtk::Application`, via its root window.
fn application_of(host: &gtk::Widget) -> Option<gtk::Application> {
    host.root()
        .and_then(|r| r.dynamic_cast::<gtk::Window>().ok())
        .and_then(|w| w.application())
}

struct TableInner {
    anims: RefCell<HashMap<SpriteRef, SpriteAnim>>,
    /// Whether the host is visible for THIS pass — `None` until the first animated
    /// sprite of the pass asks, so a pass that draws no animated sprite asks nothing.
    pass_visible: Cell<Option<bool>>,
    visibility: RefCell<Option<visibility::VisibilityWatch>>,
    /// Set when a pass refused to play because the host was not visible, or when the
    /// visibility watch dropped live entries. The next visible report then asks the host
    /// to repaint, which is what starts or rebuilds them. Without it nothing would: GTK
    /// reuses a widget's last render node until that widget's own draw is queued, so a
    /// host whose sprite was refused paints no second pass by itself. MEASURED in the
    /// running app: a disclosure indicator below the viewport at its first paint was
    /// snapshotted once, and stayed on its first frame after the window grew to show it.
    repaint_when_visible: Cell<bool>,
}

/// One host's animated sprites. See the module docs for the pass protocol.
pub(crate) struct SpriteTable {
    inner: Rc<TableInner>,
}

impl Default for SpriteTable {
    fn default() -> Self {
        SpriteTable {
            inner: Rc::new(TableInner {
                anims: RefCell::new(HashMap::new()),
                pass_visible: Cell::new(None),
                visibility: RefCell::new(None),
                repaint_when_visible: Cell::new(false),
            }),
        }
    }
}

impl SpriteTable {
    /// Start a paint: every entry is unseen until a `Frames` call marks it.
    pub(crate) fn begin_pass(&self) {
        self.inner.pass_visible.set(None);
        for anim in self.inner.anims.borrow_mut().values_mut() {
            anim.seen_this_pass = false;
        }
    }

    /// End a paint: drop every entry this pass did not draw.
    pub(crate) fn end_pass(&self) {
        self.inner
            .anims
            .borrow_mut()
            .retain(|_, anim| anim.seen_this_pass);
    }

    /// The frame source for one pass of `host`'s paint.
    pub(crate) fn frames<'a>(&'a self, host: &'a gtk::Widget) -> Frames<'a> {
        Frames {
            live: Some(Live { host, table: self }),
        }
    }

    /// Drop every entry AND the visibility watch — for a host's `dispose`, which runs
    /// before this table's own `Drop` and while the host can still be disconnected from.
    pub(crate) fn release(&self) {
        self.inner.anims.borrow_mut().clear();
        self.inner.visibility.borrow_mut().take();
    }

    /// Run `f` against the live entry for `r`, if there is one. Never creates one.
    #[cfg(all(test, feature = "gtk-integration-tests"))]
    pub(crate) fn with_anim<R>(
        &self,
        r: &SpriteRef,
        f: impl FnOnce(&mut SpriteAnim) -> R,
    ) -> Option<R> {
        self.inner.anims.borrow_mut().get_mut(r).map(f)
    }

    fn pass_visible(&self, host: &gtk::Widget) -> bool {
        if let Some(v) = self.inner.pass_visible.get() {
            return v;
        }
        let v = visibility::current(host);
        self.inner.pass_visible.set(Some(v));
        v
    }

    /// Install the host's visibility watch, once.
    fn ensure_visibility_watch(&self, host: &gtk::Widget) {
        if self.inner.visibility.borrow().is_some() {
            return;
        }
        let table = Rc::downgrade(&self.inner);
        let weak_host = host.downgrade();
        let watch = visibility::watch(host, move |visible| {
            let (Some(table), Some(host)) = (table.upgrade(), weak_host.upgrade()) else {
                return;
            };
            if !visible && visibility::hidden_regardless_of_geometry(&host) {
                // No paint will come to prune these, so drop them here.
                let mut anims = table.anims.borrow_mut();
                if !anims.is_empty() {
                    anims.clear();
                    table.repaint_when_visible.set(true);
                }
                return;
            }
            // Only geometry can have changed, and this report may have read allocations
            // the frame's layout has not moved yet. MEASURED: the one report after
            // scrolling an icon into view said hidden, while the paint it asked for found
            // the icon on screen. A paint reads allocations after layout, so ask for one
            // and let the pass decide — if there is anything to decide.
            let playing = !table.anims.borrow().is_empty();
            if table.repaint_when_visible.replace(false) || playing {
                host.queue_draw();
            }
        });
        self.inner.visibility.replace(Some(watch));
    }

    /// Play `r` for this pass and run `f` against its entry — `None` for a still
    /// sprite, an invisible host, or an animation that will not open.
    fn play<R>(
        &self,
        host: &gtk::Widget,
        r: &SpriteRef,
        f: impl FnOnce(&mut SpriteAnim) -> R,
    ) -> Option<R> {
        // Populates `animated_bytes` for a sprite nothing has decoded yet (cached).
        crate::sprite::texture(r)?;
        let bytes = crate::sprite::animated_bytes(r)?;
        self.ensure_visibility_watch(host);
        if !self.pass_visible(host) {
            // Refused for invisibility: have the watch ask for a repaint once the host is
            // visible, or this sprite is never asked about again (see the field's docs).
            self.inner.repaint_when_visible.set(true);
            return None;
        }
        let back = Backref {
            host: host.downgrade(),
            table: Rc::downgrade(&self.inner),
            sprite: r.clone(),
        };
        let mut anims = self.inner.anims.borrow_mut();
        let anim = match anims.entry(r.clone()) {
            std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
            std::collections::hash_map::Entry::Vacant(v) => v.insert(SpriteAnim::new(bytes)?),
        };
        anim.seen_this_pass = true;
        anim.recompute(&back, host);
        Some(f(anim))
    }
}

#[derive(Clone, Copy)]
struct Live<'a> {
    host: &'a gtk::Widget,
    table: &'a SpriteTable,
}

/// Where a paint gets a sprite's pixels from, handed out per pass by
/// [`SpriteTable::frames`].
#[derive(Clone, Copy)]
pub(crate) struct Frames<'a> {
    live: Option<Live<'a>>,
}

impl Frames<'_> {
    /// `r` at its natural size: the current frame if it is animated and playing here,
    /// else the still texture.
    pub(crate) fn natural(&self, r: &SpriteRef) -> Option<gtk::gdk::Texture> {
        if let Some(Live { host, table }) = self.live {
            if let Some(tex) = table.play(host, r, |anim| anim.current_texture()) {
                return Some(tex);
            }
        }
        crate::sprite::texture(r)
    }

    /// `r` resampled to exactly `w × h`, nearest-neighbour: the current frame if it is
    /// animated and playing here, else `sprite::scaled`.
    pub(crate) fn scaled(&self, r: &SpriteRef, w: i32, h: i32) -> Option<gtk::gdk::Texture> {
        if w <= 0 || h <= 0 {
            return None;
        }
        if let Some(Live { host, table }) = self.live {
            if let Some(Some(tex)) = table.play(host, r, |anim| anim.scaled_texture(w, h)) {
                return Some(tex);
            }
        }
        crate::sprite::scaled(r, w, h)
    }
}

/// Test-only oracle: whether a tick callback is currently installed. GTK 4.6 exposes no
/// `gtk_widget_has_tick_callback`, so this is this type's own bookkeeping.
#[cfg(all(test, feature = "gtk-integration-tests"))]
impl SpriteAnim {
    pub(crate) fn is_ticking(&self) -> bool {
        self.tick.is_some()
    }
}
