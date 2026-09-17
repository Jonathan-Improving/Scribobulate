//! `#[gtktest::test]` coverage for the pause badge (TDD 27.8), split out of
//! `gtk_tests.rs` for the same 500-line reason that file is its own — every
//! assertion here renders the paintable's OWN `snapshot()` into real pixels
//! and inspects the result, never reading `AnimatedPaintable`'s internal
//! flags (POLICY's "assert on the running behaviour, not your own
//! bookkeeping" instinct).
//!
//! Shares its fixture/harness helpers with `gtk_tests.rs` (`anim_bytes`,
//! `test_app`, `realize`, `add_play_animations_action`) rather than
//! duplicating them — see that file's doc comments on those functions.

use super::gtk_tests::{add_play_animations_action, anim_bytes, realize, test_app};
use super::{badge, AnimatedPaintable};
use crate::animation::policy::{self, EnableAnimationsGuard};
use gtk::prelude::*;

/// One frame's raw RGBA8 pixels, downloaded from whatever `paintable` paints
/// at `width`×`height` — the badge included, since this renders the
/// paintable's real `snapshot()` vfunc through a real GSK renderer, the same
/// software (Cairo) renderer this project ships on and `tests/icon_resolution.rs`'s
/// own `--render` evidence mode uses.
struct Rendered {
    stride: usize,
    bytes: Vec<u8>,
}

impl Rendered {
    /// The pixel at `(x, y)`, normalized to `(r, g, b, a)`.
    ///
    /// **MEASURED, not assumed:** a `GskCairoRenderer::render_texture` result
    /// downloads in Cairo's own native `ARGB32` surface layout — B, G, R, A
    /// byte order on this (little-endian) host — NOT the plain R,G,B,A order
    /// every other texture in this codebase is built from
    /// (`gdk::gdk::MemoryTexture::new(.., MemoryFormat::R8g8b8a8, ..)`
    /// elsewhere in this crate). Confirmed by a known-fill control: a solid
    /// `(128, 96, 64, 255)` texture rendered through this exact path and
    /// downloaded back as `(64, 96, 128, 255)` — R and B swapped, G and A
    /// untouched — before this method existed to correct it.
    ///
    /// `Texture::format()` (`gdk4::prelude::TextureExt::format`) would be
    /// the principled way to read this back, but it needs GTK 4.10
    /// (`#[cfg(feature = "v4_10")]` in `gdk4-0.10.3/src/auto/texture.rs`)
    /// and this project's floor is 4.6 (`features = ["v4_6"]` in
    /// `Cargo.toml`) — the method does not exist in this build at all, so
    /// there is nothing to query. Every comparison in this file is opaque
    /// wherever it matters (the badge composites onto a fully opaque
    /// frame), so premultiplication is never a concern, only this one fixed
    /// channel swap.
    fn pixel(&self, x: i32, y: i32) -> (u8, u8, u8, u8) {
        let row = y as usize * self.stride;
        let col = x as usize * 4;
        let i = row + col;
        let (b, g, r, a) = (
            self.bytes[i],
            self.bytes[i + 1],
            self.bytes[i + 2],
            self.bytes[i + 3],
        );
        (r, g, b, a)
    }
}

fn render(paintable: &gtk::gdk::Paintable, width: f64, height: f64) -> Rendered {
    use gtk::gsk;
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width, height);
    let node = snapshot
        .to_node()
        .expect("the paintable must paint SOMETHING at this size");
    let renderer = gsk::CairoRenderer::new();
    renderer.realize(None::<&gtk::gdk::Surface>).expect(
        "cairo renderer realizes headlessly — GSK_RENDERER=cairo is this project's own pin",
    );
    let texture = renderer.render_texture(&node, None);
    let stride = texture.width() as usize * 4;
    let mut bytes = vec![0u8; stride * texture.height() as usize];
    texture.download(&mut bytes, stride);
    renderer.unrealize();
    Rendered { stride, bytes }
}

/// Sum of R+G+B, ignoring alpha — a cheap "how dark is this" scalar. The
/// well's own fill is a translucent BLACK circle (`badge::WELL_FILL`), so
/// overlaying it can only ever make whatever was underneath darker or
/// leave it unchanged (compositing onto anything is not brightening) —
/// which is what makes "paused is darker here than playing" a sound,
/// fixture-color-independent signal that something was painted, without
/// needing to know the fixture's own corner colour up front.
fn darkness(rendered: &Rendered, x: i32, y: i32) -> u32 {
    let (r, g, b, _a) = rendered.pixel(x, y);
    255 * 3 - (r as u32 + g as u32 + b as u32)
}

/// A point that is INSIDE the circular well but OUTSIDE the 16px icon box
/// centered within it — see `badge::well_rect`'s geometry: a 32px well with
/// an 8px inset before the 16px icon starts, so `well.x + 2` is 6px clear of
/// the icon on every side while still well within the well's own radius.
/// Detects "is the WELL painted" independent of the glyph's own shape (a
/// pause icon's silhouette does not have to cover this specific point).
fn well_only_point(well: &gtk::graphene::Rect) -> (i32, i32) {
    (well.x() as i32 + 2, (well.y() + well.height() / 2.0) as i32)
}

fn animated(app: &gtk::Application) -> (gtk::Picture, AnimatedPaintable, gtk::ApplicationWindow) {
    let pic = gtk::Picture::new();
    let paintable =
        AnimatedPaintable::new(pic.upcast_ref(), anim_bytes()).expect("fixture decodes");
    pic.set_paintable(Some(&paintable));
    let window = realize(app, &pic);
    (pic, paintable, window)
}

/// TDD 27.8: paused paints the badge, playing does not — same picture, same
/// render size, compared against ITSELF across the transition, so the test
/// needs no assumption about the fixture's own colours.
#[gtktest::test]
fn paused_paints_the_badge_playing_does_not() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("badge.playpause");
    add_play_animations_action(&app, true);
    let (pic, paintable, _window) = animated(&app);
    assert!(paintable.tick_installed(), "sanity: autoplaying");

    let well = badge::well_rect(200.0, 200.0, gtk::TextDirection::Ltr);
    let (wx, wy) = well_only_point(&well);

    let playing = render(pic.paintable().as_ref().unwrap(), 200.0, 200.0);
    let playing_darkness = darkness(&playing, wx, wy);

    app.change_action_state(policy::ACTION_NAME, &false.to_variant());
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the pause to take effect",
        || !paintable.tick_installed(),
    );

    let paused = render(pic.paintable().as_ref().unwrap(), 200.0, 200.0);
    let paused_darkness = darkness(&paused, wx, wy);

    assert!(
        paused_darkness > playing_darkness,
        "pausing must darken the well corner (badge painted): playing={playing_darkness} paused={paused_darkness}"
    );
}

/// TDD 27.8: an image under 48px on a side never shows the badge, even
/// paused; the SAME picture at 60px does. Compares paused-vs-playing at
/// each size independently (as above), so a size where the badge is
/// correctly suppressed reads as "no change", not "some other darkening".
#[gtktest::test]
fn under_48px_suppresses_the_badge_at_or_above_shows_it() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("badge.threshold");
    add_play_animations_action(&app, true);
    let (pic, paintable, _window) = animated(&app);
    assert!(paintable.tick_installed(), "sanity: autoplaying");

    let paintable_dyn = pic.paintable().unwrap();
    let darkness_at = |size: f64| {
        let well = badge::well_rect(size, size, gtk::TextDirection::Ltr);
        let (x, y) = well_only_point(&well);
        darkness(&render(&paintable_dyn, size, size), x, y)
    };

    let small_playing = darkness_at(40.0);
    let large_playing = darkness_at(60.0);

    app.change_action_state(policy::ACTION_NAME, &false.to_variant());
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the pause to take effect",
        || !paintable.tick_installed(),
    );

    let small_paused = darkness_at(40.0);
    let large_paused = darkness_at(60.0);

    assert_eq!(
        small_paused, small_playing,
        "under badge::MIN_PAINTED_SIDE (40 < 48) the badge must be omitted entirely — well corner \
         must read identically paused vs playing"
    );
    assert!(
        large_paused > large_playing,
        "at/above the floor (60 >= 48) the badge must paint: playing={large_playing} paused={large_paused}"
    );
}

/// TDD 27.6/27.7: "reduce animations" holds frame 0 AND paints the badge —
/// both halves of the same state, not just the frame-freeze half
/// `gtk_tests.rs` already covers.
#[gtktest::test]
fn reduce_animations_holds_frame_zero_and_paints_the_badge() {
    let enable = EnableAnimationsGuard::set(true);
    let app = test_app("badge.reduce");
    let (pic, paintable, _window) = animated(&app);
    assert!(paintable.tick_installed(), "sanity: autoplaying");

    let well = badge::well_rect(200.0, 200.0, gtk::TextDirection::Ltr);
    let (wx, wy) = well_only_point(&well);
    let playing_darkness = darkness(
        &render(pic.paintable().as_ref().unwrap(), 200.0, 200.0),
        wx,
        wy,
    );

    enable.set_live(false);
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "reduce-animations to take effect",
        || !paintable.tick_installed(),
    );

    let reduced_darkness = darkness(
        &render(pic.paintable().as_ref().unwrap(), 200.0, 200.0),
        wx,
        wy,
    );
    assert!(
        reduced_darkness > playing_darkness,
        "reduce-animations must ALSO paint the badge, not just freeze frame 0: \
         playing={playing_darkness} reduced={reduced_darkness}"
    );
}

/// TDD 27.8: a still image — a plain `GdkTexture`, never wrapped in
/// `AnimatedPaintable` at all, exactly as the rest of this app renders a
/// non-animated picture — never carries the badge. Uses a fully KNOWN solid
/// colour so the assertion needs no differential capture: the well-corner
/// pixel must come back byte-identical to the fill colour painted.
#[gtktest::test]
fn a_plain_still_texture_never_shows_the_badge() {
    // Solid opaque mid-grey, deliberately not black or white — distinct
    // from both `badge::WELL_FILL` (black) and the icon colour (white), so
    // ANY blending at all would be visible as a shift away from this exact
    // value.
    const SIDE: i32 = 64;
    let fill = [128u8, 96u8, 64u8, 255u8];
    let mut pixels = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for _ in 0..(SIDE * SIDE) {
        pixels.extend_from_slice(&fill);
    }
    let bytes = gtk::glib::Bytes::from_owned(pixels);
    let texture = gtk::gdk::MemoryTexture::new(
        SIDE,
        SIDE,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        (SIDE * 4) as usize,
    );

    let rendered = render(texture.upcast_ref(), SIDE as f64, SIDE as f64);
    let well = badge::well_rect(SIDE as f64, SIDE as f64, gtk::TextDirection::Ltr);
    let (wx, wy) = well_only_point(&well);
    assert_eq!(
        rendered.pixel(wx, wy),
        (fill[0], fill[1], fill[2], fill[3]),
        "a plain texture's own paintable must never invoke the badge painting path"
    );
}

/// GTK4Rs/AP-48 / ScrAP-169: `has_icon` proves a name resolves, never that
/// it renders real symbolic art. This checks the SAME live `IconTheme` the
/// app uses, resolves through it, and requires an actual backing file —
/// then separately proves something bright (the glyph) was actually drawn
/// inside the well by scanning the rendered pixels, not by trusting the
/// lookup's own success.
#[gtktest::test]
fn the_pause_icon_actually_renders() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("badge.iconrenders");
    add_play_animations_action(&app, true);
    let (pic, paintable, _window) = animated(&app);
    assert!(paintable.tick_installed());

    let display = pic.display();
    let theme = gtk::IconTheme::for_display(&display);
    let resolved = theme.lookup_icon(
        badge::icon_used_for_test().name(),
        &[],
        16,
        1,
        gtk::TextDirection::Ltr,
        gtk::IconLookupFlags::empty(),
    );
    assert!(
        resolved.file().is_some(),
        "{} must resolve to a real backing file, not GTK's fallback — has_icon alone cannot prove \
         this (GTK4Rs/AP-48)",
        badge::icon_used_for_test().name()
    );

    app.change_action_state(policy::ACTION_NAME, &false.to_variant());
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the pause to take effect",
        || !paintable.tick_installed(),
    );

    let paused = render(pic.paintable().as_ref().unwrap(), 200.0, 200.0);
    let well = badge::well_rect(200.0, 200.0, gtk::TextDirection::Ltr);
    let (wx, wy) = well_only_point(&well);
    let well_only_darkness = darkness(&paused, wx, wy);

    // Scan the 16px icon box for a pixel visibly LIGHTER than the plain
    // well fill — the glyph's own white ink over the dark well, wherever
    // its silhouette actually falls, without hardcoding the glyph's shape.
    let inset = 8;
    let mut lightest_in_icon_box = 0u32;
    for dy in 0..16 {
        for dx in 0..16 {
            let (r, g, b, a) =
                paused.pixel(well.x() as i32 + inset + dx, well.y() as i32 + inset + dy);
            if a == 0 {
                continue;
            }
            let brightness = r as u32 + g as u32 + b as u32;
            lightest_in_icon_box = lightest_in_icon_box.max(brightness);
        }
    }
    let well_only_brightness = 255 * 3 - well_only_darkness;
    assert!(
        lightest_in_icon_box > well_only_brightness + 60,
        "no pixel inside the icon box is meaningfully lighter than the plain well fill — the icon \
         resolved (asserted above) but nothing legible was actually painted: lightest={lightest_in_icon_box} \
         well_only={well_only_brightness}"
    );
}

/// The bottom-end corner follows the text direction rather than a hardcoded
/// side — end = right in LTR, left in RTL.
///
/// Compares each corner against its OWN playing (no-badge) baseline rather
/// than comparing the two corners to EACH OTHER at a single moment — the
/// fixture's own decoded frame is not guaranteed to be equally dark in both
/// bottom corners (MEASURED: it is not), so a left-vs-right comparison at
/// one instant conflates "which corner the badge painted" with "which
/// corner of this frame happens to be darker to begin with".
#[gtktest::test]
fn the_badge_follows_text_direction() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("badge.rtl");
    add_play_animations_action(&app, true);
    let (pic, paintable, _window) = animated(&app);
    assert!(paintable.tick_installed(), "sanity: autoplaying");

    let right = well_only_point(&badge::well_rect(200.0, 200.0, gtk::TextDirection::Ltr));
    let left = well_only_point(&badge::well_rect(200.0, 200.0, gtk::TextDirection::Rtl));

    // No badge anywhere yet — each corner's own baseline, whatever the
    // fixture's own colours happen to be there.
    let playing = render(pic.paintable().as_ref().unwrap(), 200.0, 200.0);
    let playing_right = darkness(&playing, right.0, right.1);
    let playing_left = darkness(&playing, left.0, left.1);

    app.change_action_state(policy::ACTION_NAME, &false.to_variant());
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the pause to take effect",
        || !paintable.tick_installed(),
    );

    pic.set_direction(gtk::TextDirection::Ltr);
    let paused_ltr = render(pic.paintable().as_ref().unwrap(), 200.0, 200.0);
    let ltr_right = darkness(&paused_ltr, right.0, right.1);
    let ltr_left = darkness(&paused_ltr, left.0, left.1);
    assert!(
        ltr_right > playing_right,
        "LTR: the RIGHT (end) corner must darken — the badge belongs there: \
         playing={playing_right} paused={ltr_right}"
    );
    assert!(
        ltr_left <= playing_left + 2,
        "LTR: the LEFT corner must NOT darken — no badge there: \
         playing={playing_left} paused={ltr_left}"
    );

    pic.set_direction(gtk::TextDirection::Rtl);
    let paused_rtl = render(pic.paintable().as_ref().unwrap(), 200.0, 200.0);
    let rtl_left = darkness(&paused_rtl, left.0, left.1);
    let rtl_right = darkness(&paused_rtl, right.0, right.1);
    assert!(
        rtl_left > playing_left,
        "RTL: the badge must move to the LEFT (end) corner: playing={playing_left} paused={rtl_left}"
    );
    assert!(
        rtl_right <= playing_right + 2,
        "RTL: the RIGHT corner must no longer carry the badge: \
         playing={playing_right} paused={rtl_right}"
    );
}
