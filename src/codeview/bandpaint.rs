//! **One line-wide band, painted at the content column** — the single arithmetic
//! behind both TDD 18.25 (headings) and TDD 18.48 (disclosure summaries).
//!
//! The two callers differ only in *what they iterate and how they resolve the
//! decoration*: a heading's band is stated per level ([`super::bands`]), a
//! disclosure's is flat ([`super::disclosurebands`]). Everything downstream of that
//! — the visibility gate, the extent, the rect and the radius — is identical, and
//! lived in two copies that had already diverged over sprite hoisting. It lives here
//! once so the next correction to it is found and applied in one place.
//!
//! **The paint itself moved one level further out** when a THIRD caller appeared that
//! is not a text-view pass at all: a table's header row (TDD 18.57) is the same band
//! drawn by an anchored widget. So the clip, the sprite → gradient → flat precedence
//! and the compositing scene are `crate::widgets::paint_band_into`, and what remains
//! here is the span half — the part that genuinely needs a `GtkTextView`.
//!
//! Where each caller sits in the compositing order is stated once, in
//! `decorplan::PAINT_ORDER`.

use super::geometry::span_card_y_extent;
use super::paint::PaintCtx;
use crate::decorplan::band_corner_radius;
use gtk::graphene;

/// Paint `span`'s band, or nothing when the span is empty, off-screen, or measures
/// to no height.
///
/// `radius_design_px` is a design-time metric at zoom 1.0 — scaled here by the zoom
/// THIS RENDER was laid out at, like every other themed pixel metric (POLICY: pixel
/// metrics do not follow the CSS `font-size` rule).
///
/// **The band's sprites are resolved only past the visibility gate just below** (TDD
/// 27.9): `ctx.frames()` is handed to `paint_band_into`, and a `Frames` call is that
/// sprite's whole visibility signal for this paint (`crate::animation::sprites`), so a
/// band that returns early here never plays its tile or its scene.
pub(super) fn paint_band(
    snapshot: &gtk::Snapshot,
    ctx: &PaintCtx,
    span: crate::span::BufferSpan,
    decor: &crate::theme::Band<'_>,
    radius_design_px: i32,
) {
    if span.is_empty() || span.is_outside(ctx.vis_start, ctx.vis_end) {
        return;
    }
    // The extent is the CONTENT COLUMN — `lm`/`card_w`, the very rect the code-block
    // card uses — not the text column a `paragraph_background` tag would pin it to. A
    // tag band follows the TAG's margins, so a banded line inside a quote or a list
    // would band at a different width from its siblings; the content column is also
    // the one extent the HTML and PDF sinks can match, which is what keeps TDD 25.3
    // honest rather than nearly honest.
    //
    // A soft-wrapped line gets ONE continuous band for free: the extent comes from
    // `span_card_y_extent`, whose ends are `line_yrange` reads, and `line_yrange`
    // spans every display row of the logical line. No display-line X is needed at
    // all, which matters because at GTK 4.6 there is no way to obtain one on the
    // paint path without a line-display cache insert (ScrAP-105).
    let (top, bottom) = span_card_y_extent(
        ctx.view,
        &ctx.buffer,
        span,
        ctx.vis_start,
        ctx.vis_end,
        ctx.vtop,
        ctx.vbot,
    );
    if bottom <= top {
        return;
    }
    let rect = graphene::Rect::new(ctx.lm, top, ctx.card_w, bottom - top);
    // `gutter_zoom` is the zoom THIS RENDER was laid out at — named for the list
    // gutter that first needed it, not scoped to it, and every pixel metric painted
    // here scales by it.
    let radius = band_corner_radius(
        radius_design_px,
        ctx.imp.gutter_zoom.get(),
        ctx.card_w,
        bottom - top,
    );
    // Everything from here down — the clip, the sprite → gradient → flat precedence and
    // the scene that composites over it — is `widgets::paint_band_into`, shared with the
    // table header's band (TDD 18.57), which is a band drawn by an anchored widget
    // rather than by this pass. What stays here is the half that is genuinely about a
    // text view: which span, measured how, at what extent.
    crate::widgets::paint_band_into(snapshot, &rect, decor, radius, ctx.frames());
}
