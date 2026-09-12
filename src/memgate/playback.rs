//! Playback memory gates (TDD 6.10), and TDD 6.7's finalization half extended
//! to the whole animation state — WP11, `sdd/PLAN.memory-gates.md`.
//!
//! Compiled only under `--features memory-gates`, alongside [`super::gtk`].
//! Every body here drives the REAL `AnimatedPaintable` (WP7b) — a leak here is
//! a leak in the object that actually plays an animation, not in a hand-rolled
//! stand-in for it. `do not touch src/animation/**` (this WP's brief) means
//! these tests reach it only through its existing `pub(crate)` seams
//! (`AnimatedPaintable::new`, `set_should_play`, `tick_installed`,
//! `decoder_active`) — never a new one added for this WP's convenience.

use gtk::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::animation::paintable::AnimatedPaintable;
use crate::animation::policy::EnableAnimationsGuard;
use crate::memgate::footprint::{current, SAMPLE_COUNT, TOLERANCE_BYTES, WARMUP};
use crate::memgate::slope::assert_flat;
use crate::testpump::{self, Clock};
use crate::window::testkit::test_app_suffixed;

fn anim_bytes() -> Arc<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/anim.webp");
    Arc::from(std::fs::read(&path).expect("fixture reads"))
}

/// A realized, mapped window hosting `pic` — the same shape
/// `animation::paintable::gtk_tests::realize` uses. Duplicated rather than
/// exported: this WP owns `src/memgate/**` only, not a new cross-module seam
/// into `animation::paintable`.
fn realize(app: &gtk::Application, pic: &gtk::Picture) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::new(app);
    window.set_child(Some(pic));
    window.present();
    testpump::until(Clock::Idle, "the window to map", || pic.is_mapped());
    window
}

/// The painted content right now, via `GdkTexture::download` — see
/// `animation::paintable::gtk_tests::painted_bytes`'s own doc comment for why
/// the WHOLE buffer, not one pixel: `anim.webp`'s frames can differ only in a
/// sub-region.
fn painted_bytes(paintable: &gtk::gdk::Paintable) -> Vec<u8> {
    let texture = paintable
        .current_image()
        .downcast::<gtk::gdk::Texture>()
        .unwrap_or_else(|p| {
            panic!("expected a Texture-shaped current image, got {p:?}");
        });
    let stride = texture.width() as usize * 4;
    let mut buf = vec![0u8; stride * texture.height() as usize];
    texture.download(&mut buf, stride);
    buf
}

fn build_playing(
    app: &gtk::Application,
) -> (gtk::Picture, AnimatedPaintable, gtk::ApplicationWindow) {
    let pic = gtk::Picture::new();
    let animated = AnimatedPaintable::new(pic.upcast_ref(), anim_bytes()).expect("fixture decodes");
    pic.set_paintable(Some(&animated));
    let window = realize(app, &pic);
    (pic, animated, window)
}

/// `anim.webp`'s own cycle, measured directly against the fixture
/// (`richimg::Animation::new` + repeated `next_frame()`, run by hand while
/// developing this test — not read from any header this module trusts
/// blindly): 24 frames, each a constant 125 ms delay, `LoopCount::Infinite`.
/// If the fixture ever changes shape, that only changes how many of these
/// samples land in one loop — `assert_flat`'s slope shape does not depend on
/// the exact cycle length.
const FIXTURE_FRAMES_PER_LOOP: usize = 24;

/// Three full cycles: "many loops" (TDD 6.10) without an unbounded wall-clock
/// cost. See [`playback_slope_across_many_loops_ttd_6_10`]'s own doc comment
/// for the measured total runtime this adds to step 5b.
const PLAYBACK_LOOPS: usize = 3;
const PLAYBACK_SAMPLES: usize = FIXTURE_FRAMES_PER_LOOP * PLAYBACK_LOOPS;

/// A generous per-frame wait: `anim.webp`'s own delay is 125 ms; this only
/// needs to be a FAILURE bound (GTK4Rs/AP-122), not a tight one.
const PER_FRAME_DEADLINE: Duration = Duration::from_secs(2);

/// TDD 6.10, first half: a playing animation sampled across MANY loops must
/// not climb.
///
/// Drives the REAL `AnimatedPaintable` under a REAL frame clock rather than
/// calling `richimg::Animation::next_frame()` directly: a deterministic,
/// sleep-free timer does not exist at this project's GTK floor (see
/// `animation::paintable`'s own module doc comment on why
/// `gtk_widget_has_tick_callback` is not bound, and no substitute exists
/// either), and bypassing the paintable would test the decoder only — the
/// mutation this rubric is written against (a `Vec` in the PAINTABLE
/// retaining every presented frame) lives one layer above the decoder, in the
/// object this test therefore has to actually drive. So this uses wall-clock
/// time, but never a blind sleep: each of the [`PLAYBACK_SAMPLES`] frame
/// flips is awaited by polling the painted bytes for a change
/// (`testpump::until_or_for`), which converges as soon as the real change
/// happens rather than waiting out a fixed span.
///
/// Measured total wall-clock runtime this adds to step 5b: **~9–11 s**
/// ([`PLAYBACK_SAMPLES`] = 72 flips × `anim.webp`'s 125 ms cadence, plus
/// pump/compare overhead) — modest, and the only honest way to observe many
/// real loops of a real tick-driven animation.
#[gtktest::test]
fn playback_slope_across_many_loops_ttd_6_10() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app_suffixed("playback-slope");
    let (pic, animated, _window) = build_playing(&app);
    assert!(
        animated.tick_installed(),
        "sanity: autoplay must be running before sampling"
    );

    let mut last = painted_bytes(pic.paintable().as_ref().unwrap());
    let mut samples = Vec::with_capacity(PLAYBACK_SAMPLES);
    for i in 0..PLAYBACK_SAMPLES {
        let changed = testpump::until_or_for(Clock::Frame, PER_FRAME_DEADLINE, || {
            painted_bytes(pic.paintable().as_ref().unwrap()) != last
        });
        assert!(changed, "frame flip {i} never happened within the deadline");
        last = painted_bytes(pic.paintable().as_ref().unwrap());
        samples.push(current().expect("footprint"));
    }
    assert_flat(&samples, WARMUP, TOLERANCE_BYTES)
        .unwrap_or_else(|err| panic!("TDD 6.10 playback slope: {err}"));
}

/// TDD 6.10, second half: repeatedly making the animation not-visible and
/// visible again must not grow the footprint across cycles.
///
/// `animation::paintable::drive::recompute` reduces "not visible" to
/// `drop_decoder_state` (releases the decoder AND the canvas) and "visible
/// again" to `ensure_decoded` (rebuilds both from the shared encoded bytes,
/// restarting at frame 0) — PLAN.memory-gates.md's "Animation state: per
/// picture, bounded by what is on screen". This drives
/// `AnimatedPaintable::set_should_play` directly rather than through a real
/// scroll/tab/window-state signal: verifying that a real such event reaches
/// this call is `animation::visibility`'s own job (WP8); `set_should_play` IS
/// the seam its watch callback drives (`drive.rs`'s `try_bootstrap`), so
/// calling it directly targets exactly what this rubric is about — the
/// memory behaviour of the drop/rebuild cycle — with no wall-clock cost at
/// all: every transition here is synchronous, unlike the tick-driven test
/// above.
#[gtktest::test]
fn scroll_away_and_back_cycles_do_not_grow_footprint_ttd_6_10() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app_suffixed("scroll-cycles");
    let (_pic, animated, _window) = build_playing(&app);
    assert!(
        animated.tick_installed(),
        "sanity: playing before any cycle"
    );
    assert!(
        animated.decoder_active(),
        "sanity: decoder present while visible"
    );

    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    for cycle in 0..SAMPLE_COUNT {
        animated.set_should_play(false);
        assert!(
            !animated.decoder_active(),
            "cycle {cycle}: going out of view must release the decoder"
        );
        assert!(
            !animated.tick_installed(),
            "cycle {cycle}: an invisible picture must have no tick callback"
        );

        animated.set_should_play(true);
        assert!(
            animated.decoder_active(),
            "cycle {cycle}: coming back into view must rebuild the decoder"
        );
        assert!(
            animated.tick_installed(),
            "cycle {cycle}: coming back into view must resume playing"
        );
        samples.push(current().expect("footprint"));
    }
    assert_flat(&samples, WARMUP, TOLERANCE_BYTES)
        .unwrap_or_else(|err| panic!("TDD 6.10 scroll-away/back cycles: {err}"));
}

/// TDD 6.7, extended to the whole animation state (WP11): once the picture
/// holding an animation is dropped, its `AnimatedPaintable` AND the current
/// frame's own texture must finalize — a weak-ref assertion for each, with no
/// main-loop pump, the same shape `memgate::gtk`'s
/// `decoded_texture_finalizes_ttd_6_7` uses for a still image's decoded
/// texture, and the same positive control: assert ALIVE while the picture
/// still holds it, so this cannot pass vacuously.
///
/// `animation::paintable::gtk_tests` already asserts the paintable half of
/// this (`dropping_the_picture_releases_the_paintable_and_its_decoder`); new
/// here is the CURRENT TEXTURE'S OWN independent weak ref (ScrAP-254: an
/// invariant held by only one mechanism at a time is not proven by exercising
/// the other one) — a leak that retained just the displayed frame somewhere
/// OUTSIDE the paintable would not show up on the paintable's weak ref alone.
///
/// The decoder itself (`richimg::Animation`) is a plain Rust value — a
/// `Box<dyn Codec>`, not a `GObject` — so it has no identity a weak reference
/// can name independently of the paintable that owns it. Its release is
/// evidenced by the SAME paintable weak-ref going empty: WP7b's own
/// `dispose()` (`src/animation/paintable/mod.rs`) unconditionally
/// `self.animation.take()`s in the same call that releases everything else,
/// so a paintable that finalizes has, by that code path, already dropped its
/// decoder. A decoder leaked independently of the paintable's own lifetime
/// (ScrAP-351's shape — a leak invisible to any refcount assertion) is what
/// [`scroll_away_and_back_cycles_do_not_grow_footprint_ttd_6_10`] above
/// catches instead, by amplification across many cycles rather than by
/// naming a single instance.
#[gtktest::test]
fn animation_state_finalizes_with_no_main_loop_pump_ttd_6_7() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app_suffixed("anim-finalize");
    let (pic, animated, window) = build_playing(&app);
    assert!(animated.tick_installed(), "sanity: playing before drop");

    let paintable_weak = animated.downgrade();
    let texture_weak = {
        let texture = pic
            .paintable()
            .expect("picture has a paintable")
            .current_image()
            .downcast::<gtk::gdk::Texture>()
            .unwrap_or_else(|p| panic!("expected a Texture current image, got {p:?}"));
        texture.downgrade()
    };

    // Positive controls, exactly as 6.7 uses for the still-image texture: a
    // test that asserted finalization while the picture and window still
    // held everything would pass on healthy code and train us to delete this.
    assert!(
        paintable_weak.upgrade().is_some(),
        "TDD 6.7 positive control: the picture must still hold the paintable"
    );
    assert!(
        texture_weak.upgrade().is_some(),
        "TDD 6.7 positive control: the paintable must still hold the current texture"
    );

    drop(animated);
    assert!(
        paintable_weak.upgrade().is_some(),
        "the PICTURE still holds a strong ref — dropping our own local must not free it yet"
    );

    window.set_child(gtk::Widget::NONE);
    drop(window);
    drop(pic);

    assert!(
        paintable_weak.upgrade().is_none(),
        "TDD 6.7: the AnimatedPaintable must finalize once the picture and window are \
         gone, with no main-loop pump (Cairo renderer is pinned)"
    );
    assert!(
        texture_weak.upgrade().is_none(),
        "TDD 6.7: the current frame's texture must finalize alongside its paintable, \
         with no main-loop pump"
    );
}
