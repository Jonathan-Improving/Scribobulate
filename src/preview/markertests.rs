//! The heading marker's two shapes, through the real renderer (TDD 18.55, 27.9).
//!
//! A still marker is a buffer paintable; an ANIMATED one is an anchored
//! `widgets::sprite_icon::SpriteIcon` at the same size, because a buffer paintable
//! cannot animate without re-wrapping its line every frame (see that module). Sited in
//! its own file because `preview/build.rs`, which owns `build_render_products`, is far
//! past the soft size limit.

use super::build::build_render_products;
use crate::animation::sprites::testkit;
use crate::widgets::sprite_icon::SpriteIcon;
use gtk::prelude::*;

/// Design px the fixture theme sizes the h1 marker at — zoom 1.0, so also device px.
const MARKER_PX: i32 = 16;

/// Activate a theme whose h1 marker is `r`, at [`MARKER_PX`].
fn activate_marker_theme(r: &crate::sprite::SpriteRef) -> crate::theme::ActiveThemeGuard {
    let mut themes = crate::theme::themes();
    themes.merge_over_for_test(
        "[themes.markered]\nbackground = \"#ffffff\"\nforeground = \"#000000\"\n",
    );
    let mut theme = themes.resolve("markered");
    let slot = crate::theme::heading_slot(1);
    theme.sprites.heading_marker[slot] = Some(r.clone());
    theme.metrics.heading_marker_size[slot] = MARKER_PX;
    let guard = crate::theme::activate_for_test(theme);
    crate::sprite::clear_cache();
    guard
}

/// The buffer offset of the one `U+FFFC` a marked heading leaves.
fn marker_offset(buf: &gtk::TextBuffer) -> i32 {
    // `BufferText` keeps the `U+FFFC` a paintable or an anchor leaves; `TextBuffer::text`
    // drops it, so the marker could not be found at all (ScrAP-74).
    let text = crate::saferizer::BufferText::of(buf);
    let index = text
        .as_str()
        .chars()
        .position(|c| c == '\u{FFFC}')
        .expect("a marked heading leaves an object-replacement character");
    i32::try_from(index).expect("offset fits")
}

/// TDD 27.9: an animated marker is an anchored `SpriteIcon` — the shape that can play —
/// sized exactly as the paintable would have been and announced as decoration.
#[gtktest::test]
fn an_animated_heading_marker_is_an_anchored_sprite_icon_at_the_markers_size() {
    let (_dir, r) = testkit::animated_fixture();
    let _theme = activate_marker_theme(&r);

    let products = build_render_products("# Title\n", None, 1.0, false);

    let icons: Vec<SpriteIcon> = products
        .anchored
        .iter()
        .filter_map(|(_, w)| w.clone().downcast::<SpriteIcon>().ok())
        .collect();
    assert_eq!(icons.len(), 1, "one animated marker, one anchored icon");
    let icon = &icons[0];
    // The fixture is 480×270: height from the theme, width from the sprite's aspect —
    // the same arithmetic the paintable took.
    let want_w = (480.0_f64 * f64::from(MARKER_PX) / 270.0).round() as i32;
    let (w, _, _, _) = WidgetExt::measure(icon, gtk::Orientation::Horizontal, -1);
    let (h, _, _, _) = WidgetExt::measure(icon, gtk::Orientation::Vertical, -1);
    assert_eq!((w, h), (want_w, MARKER_PX));
    assert_eq!(
        icon.accessible_role(),
        gtk::AccessibleRole::Presentation,
        "a heading marker stands for no content"
    );

    let at = products.buf.iter_at_offset(marker_offset(&products.buf));
    assert!(
        at.child_anchor().is_some(),
        "the marker's character carries the icon's anchor"
    );
    assert!(at.paintable().is_none(), "and no still paintable beside it");
    crate::sprite::clear_cache();
}

/// TDD 18.55: a STILL marker is unchanged — a buffer paintable, no anchored widget.
#[gtktest::test]
fn a_still_heading_marker_stays_a_buffer_paintable() {
    let (_dir, r) = testkit::still_fixture();
    let _theme = activate_marker_theme(&r);

    let products = build_render_products("# Title\n", None, 1.0, false);

    assert!(
        products.anchored.iter().all(|(_, w)| !w.is::<SpriteIcon>()),
        "a still marker must not become a widget"
    );
    let at = products.buf.iter_at_offset(marker_offset(&products.buf));
    assert!(
        at.paintable().is_some(),
        "a still marker is a buffer paintable"
    );
    assert!(at.child_anchor().is_none(), "with no anchor");
    crate::sprite::clear_cache();
}
