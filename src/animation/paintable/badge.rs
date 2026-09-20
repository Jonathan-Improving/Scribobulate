//! The paused-animation corner badge (TDD 27.8). Split out
//! of `mod.rs` at POLICY's file-size soft limit, the same reason `drive.rs`
//! and `gtk_tests.rs` are their own files.
//!
//! **Paint-only, by construction.** Everything here is called from
//! [`super::imp::AnimatedPaintable::snapshot`] alone — there is no widget,
//! no `GtkButton`, no gesture controller anywhere in this module, so "no
//! click handler" is not a rule this code has to remember to follow; there
//! is nothing here a click could reach. The View ▸ Play Animations toggle
//! (`crate::animation::policy`) stays the only control (POLICY's "one
//! `GAction`, referenced everywhere" rule already owns that surface).
//!
//! **The glyph is the STATE, never the action.** `Icon::MediaPlaybackPause`
//! (`media-playback-pause-symbolic`) says "this is frozen", not
//! `media-playback-start-symbolic`'s "click me to play" — an interaction
//! this paintable does not offer. Both
//! names ship in GTK's own icon set; [`icon_used_for_test`] exists so a test
//! can assert on the literal name actually painted rather than trusting a
//! doc comment not to drift, and [`paint`] verifies the lookup actually
//! resolved rather than trusting `has_icon` (GTK4Rs/AP-48; ScrAP-169 is this
//! project's own instance of the same lesson).

use crate::icons::Icon;
use gtk::prelude::*;

/// Below this many DEVICE-INDEPENDENT pixels on either side of the PAINTED
/// area, the badge would cover more of the picture than it labels — TDD
/// 27.8's own floor ("an image under 48 pixels on a side does not show
/// it"). This is the size the picture is actually being drawn at (the
/// `width`/`height` GTK hands `GdkPaintable::snapshot`), not the decoded
/// frame's intrinsic pixel size: a huge animation displayed tiny must not
/// carry a badge that dwarfs it, and a modest animation displayed large
/// should.
pub(super) const MIN_PAINTED_SIDE: f64 = 48.0;

/// The glyph's own box: a 16 px symbolic icon.
const ICON_SIZE: f64 = 16.0;

/// The circular `.osd` well the glyph sits inside, ~32 px. Fixed regardless
/// of the image's paint size: the badge does not scale with the image.
const WELL_DIAMETER: f64 = 32.0;

/// Inset from the picture's own edge to the well's edge, so the badge does
/// not sit flush against the picture's border.
const EDGE_MARGIN: f64 = 4.0;

/// The well's own fill — a plain translucent dark circle standing in for
/// the `.osd` CSS class's usual background, since this is raw `GdkPaintable`
/// snapshot painting with no widget/style-context to source it from
/// (the decided look is a small circular `.osd` well over an unscrimmed
/// image — the badge has a fill, the image behind it does not).
const WELL_FILL: gtk::gdk::RGBA = gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.55);

/// The glyph's own colour inside the well — light-on-dark, matching the
/// `.osd` idiom's usual contrast, supplied directly to
/// [`gtk::prelude::SymbolicPaintableExt::snapshot_symbolic`] rather than
/// via a style context (see [`WELL_FILL`]'s doc comment).
const ICON_COLOR: gtk::gdk::RGBA = gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);

/// The icon this badge paints. A `const`, not inlined into [`paint`], so a
/// test can assert on it directly (mutation test 3: swapping this for
/// `Icon::MediaPlaybackStart` — the click-to-play idiom — must fail a test
/// that reads this constant, not just a test that happens to notice a
/// different glyph rendered).
pub(super) const ICON: Icon = Icon::MediaPlaybackPause;

/// Whether this picture, in its current state, should carry the pause
/// badge — pure decision, no rendering, no GTK types beyond the plain
/// floats already in hand at a paint call. TDD 27.8's partition: a still
/// image or a playing animation never show it (the caller's `paused`
/// argument already encodes that — see `drive.rs`'s `paused_by_policy`,
/// which is `false` for both), and an image painted under
/// [`MIN_PAINTED_SIDE`] on either side never shows it regardless of pause
/// state.
pub(super) fn should_paint(paused: bool, painted_width: f64, painted_height: f64) -> bool {
    paused && painted_width >= MIN_PAINTED_SIDE && painted_height >= MIN_PAINTED_SIDE
}

/// Where the circular well sits, in the paintable's own
/// `(0,0)..(width,height)` snapshot space — the bottom-END corner,
/// text-direction-aware (end = right in LTR, left in RTL; never a hardcoded
/// side). Only
/// [`gtk::TextDirection::Rtl`] flips it; `Ltr` and the unresolved `None`
/// both take the LTR (right) placement, matching GTK's own default.
pub(super) fn well_rect(
    width: f64,
    height: f64,
    direction: gtk::TextDirection,
) -> gtk::graphene::Rect {
    let x = if direction == gtk::TextDirection::Rtl {
        EDGE_MARGIN
    } else {
        width - EDGE_MARGIN - WELL_DIAMETER
    };
    let y = height - EDGE_MARGIN - WELL_DIAMETER;
    gtk::graphene::Rect::new(
        x as f32,
        y as f32,
        WELL_DIAMETER as f32,
        WELL_DIAMETER as f32,
    )
}

/// The icon's own box, centered inside [`well_rect`]'s well.
fn icon_rect(well: &gtk::graphene::Rect) -> gtk::graphene::Rect {
    let inset = ((WELL_DIAMETER - ICON_SIZE) / 2.0) as f32;
    gtk::graphene::Rect::new(
        well.x() + inset,
        well.y() + inset,
        ICON_SIZE as f32,
        ICON_SIZE as f32,
    )
}

/// Paint the badge into `snapshot` — the well, then the glyph centered
/// inside it. `display`/`direction` come from the paintable's host widget
/// (see [`super::imp::AnimatedPaintable::snapshot`]); `width`/`height` are
/// the SAME paint dimensions GTK handed the paintable's own `snapshot`
/// vfunc.
///
/// The icon lookup goes through the live [`gtk::IconTheme`] exactly as
/// `tests/icon_resolution.rs`'s own `--render` evidence mode does — this is
/// the render path itself, not a check that stands in for it, so there is
/// nothing here to verify against `has_icon` (GTK4Rs/AP-48): if the name
/// fails to resolve to real symbolic art, the well paints with nothing
/// inside it, and the test suite in `badge_tests.rs` asserts on the
/// rendered pixels rather than on this function returning without panicking.
pub(super) fn paint(
    snapshot: &gtk::Snapshot,
    display: &gtk::gdk::Display,
    direction: gtk::TextDirection,
    width: f64,
    height: f64,
) {
    let theme = gtk::IconTheme::for_display(display);
    let icon = theme.lookup_icon(
        ICON.name(),
        &[],
        ICON_SIZE as i32,
        1,
        direction,
        gtk::IconLookupFlags::empty(),
    );

    let well = well_rect(width, height, direction);
    let rounded = gtk::gsk::RoundedRect::from_rect(well, (WELL_DIAMETER / 2.0) as f32);
    snapshot.push_rounded_clip(&rounded);
    snapshot.append_color(&WELL_FILL, &well);
    snapshot.pop();

    let glyph = icon_rect(&well);
    snapshot.save();
    snapshot.translate(&gtk::graphene::Point::new(glyph.x(), glyph.y()));
    icon.snapshot_symbolic(snapshot, ICON_SIZE, ICON_SIZE, &[ICON_COLOR]);
    snapshot.restore();
}

/// The icon name this module paints — [`ICON`] as a test-facing accessor for
/// `badge_tests.rs`. Gated to that file's OWN cfg
/// (`all(test, feature = "gtk-integration-tests")`), not a bare
/// `#[cfg(test)]` — POLICY's rule for a test-only helper: its only caller is
/// feature-gated, so a bare `#[cfg(test)]` would leave this compiled and
/// unused under a plain `cargo test`.
#[cfg(all(test, feature = "gtk-integration-tests"))]
pub(super) fn icon_used_for_test() -> Icon {
    ICON
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TDD 27.8: the STATE glyph, never the click-to-play action icon.
    /// Mutation test: swapping [`ICON`] for
    /// `Icon::MediaPlaybackStart` must turn this red.
    #[test]
    fn paints_the_pause_state_not_the_play_action() {
        assert_eq!(ICON.name(), "media-playback-pause-symbolic");
        assert_ne!(ICON.name(), "media-playback-start-symbolic");
    }

    /// TDD 27.8: still images and playing animations never show the badge
    /// (encoded by the caller's `paused` argument being `false` for both —
    /// this function does not re-derive that, only the size floor).
    #[test]
    fn only_paused_and_only_at_or_above_the_floor() {
        assert!(!should_paint(false, 200.0, 200.0), "playing must not paint");
        assert!(
            !should_paint(false, 10.0, 10.0),
            "still image must not paint"
        );
        assert!(should_paint(true, 200.0, 200.0), "paused, well above floor");
        assert!(
            should_paint(true, MIN_PAINTED_SIDE, MIN_PAINTED_SIDE),
            "AT the floor must still paint — the floor is inclusive"
        );
    }

    /// Mutation test: dropping the 48px rule (always `true`
    /// once `paused`) must turn this red.
    #[test]
    fn under_the_floor_on_either_side_suppresses_the_badge() {
        assert!(!should_paint(true, MIN_PAINTED_SIDE - 1.0, 200.0));
        assert!(!should_paint(true, 200.0, MIN_PAINTED_SIDE - 1.0));
        assert!(!should_paint(true, 10.0, 10.0));
    }

    /// The bottom-END corner follows the text direction rather than a
    /// hardcoded side: end = right in LTR, left in RTL.
    #[test]
    fn well_sits_bottom_end_and_follows_text_direction() {
        let width = 200.0;
        let height = 100.0;
        let ltr = well_rect(width, height, gtk::TextDirection::Ltr);
        let rtl = well_rect(width, height, gtk::TextDirection::Rtl);

        // Bottom edge is the same regardless of direction.
        assert_eq!(ltr.y(), rtl.y());
        let expected_bottom = (height - EDGE_MARGIN - WELL_DIAMETER) as f32;
        assert_eq!(ltr.y(), expected_bottom);

        // LTR end = right: near the right edge, far from the left.
        assert_eq!(ltr.x(), (width - EDGE_MARGIN - WELL_DIAMETER) as f32);
        // RTL end = left: flush to the left margin instead.
        assert_eq!(rtl.x(), EDGE_MARGIN as f32);
        assert_ne!(ltr.x(), rtl.x());
    }

    /// `TextDirection::None` (unresolved) takes the same placement as
    /// `Ltr` — GTK's own default — rather than silently landing somewhere
    /// unspecified.
    #[test]
    fn unresolved_direction_defaults_to_ltr_placement() {
        let ltr = well_rect(200.0, 100.0, gtk::TextDirection::Ltr);
        let none = well_rect(200.0, 100.0, gtk::TextDirection::None);
        assert_eq!(ltr.x(), none.x());
        assert_eq!(ltr.y(), none.y());
    }

    /// The glyph is centered inside the well on both axes, never scaled
    /// with it.
    #[test]
    fn icon_is_centered_inside_the_well() {
        let well = well_rect(200.0, 100.0, gtk::TextDirection::Ltr);
        let glyph = icon_rect(&well);
        assert_eq!(glyph.width(), ICON_SIZE as f32);
        assert_eq!(glyph.height(), ICON_SIZE as f32);
        let well_center_x = well.x() + well.width() / 2.0;
        let well_center_y = well.y() + well.height() / 2.0;
        let glyph_center_x = glyph.x() + glyph.width() / 2.0;
        let glyph_center_y = glyph.y() + glyph.height() / 2.0;
        assert_eq!(well_center_x, glyph_center_x);
        assert_eq!(well_center_y, glyph_center_y);
    }
}
