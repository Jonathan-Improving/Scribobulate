//! **The blockquote decorations — the panel behind a quote and the accent bar down
//! its left edge**, both drawn from one measured extent.
//!
//! Lifted out of `snapshot_layer` whole, so the diff is a move rather than a rewrite.
//! The two draws are deliberately NOT adjacent in the paint: the panel opens the
//! below-text pass because a quote is the outermost container in the vocabulary, and
//! the bar closes the block decorations because a quoted heading band or code card
//! runs *under* it. `decorplan::PAINT_ORDER` is what holds them apart, and
//! `codeview::ordertests` is what proves the pixels agree.

use super::geometry::span_card_y_extent;
use super::paint::PaintCtx;
use gtk::prelude::*;
use gtk::{graphene, TextBuffer};

/// Every VISIBLE quote's clamped y-extent, computed once per below-text pass.
///
/// Consumed twice — by [`draw_panel`] and by [`draw_accent_bar`] — from the SAME
/// value, so the bar and the fill behind it can never disagree about where the quote
/// starts or ends. That disagreement is precisely how the TDD 18.29 defect announced
/// itself, and carrying the extents on the paint context rather than recomputing them
/// per painter is what keeps the property structural now the two draws live apart.
pub(super) fn visible_extents(
    view: &super::CodePreviewView,
    buffer: &TextBuffer,
    blockquotes: &[crate::span::QuoteSpan],
    vis_start: i32,
    vis_end: i32,
    vtop: f32,
    vbot: f32,
) -> Vec<QuoteExtent> {
    // One `QuoteSpan` per blockquote LEVEL (`renderer::end` closes and records at every
    // `TagEnd::BlockQuote`, not only the outermost), so a level's extent spans its intro
    // paragraph, any nested list or quote and its closing paragraph as ONE run, with the
    // blank separator lines inside it. An enclosing level's extent covers the levels
    // inside it, which is exactly what makes the outer bar run past the inner region
    // rather than stopping where it begins (TDD 2.11b).
    //
    // That is also the whole of the fix to TDD 18.29: the panel used to be a
    // `paragraph_background_rgba` on the quote tag, which GTK fills PER PARAGRAPH, so a
    // quote holding a paragraph plus a list rendered as three disconnected rectangles
    // with the page showing through between them — beside a bar that had always been
    // drawn from this single extent and was therefore continuous. Same quote, two
    // different extents, which is what made it visible.
    //
    // The clamping discipline is `span_card_y_extent`'s (GTK4Rs/AP-22: never measure an
    // off-screen, unvalidated iter), and the extent carries NO extra pad (GTK4Rs/AP-127)
    // — `line_yrange` already includes each line's own `pixels_above/below_lines`, which
    // is where the panel's vertical breathing room comes from, exactly as it did from the
    // tag.
    blockquotes
        .iter()
        .filter(|q| !q.span.is_empty() && !q.span.is_outside(vis_start, vis_end))
        .filter_map(|&crate::span::QuoteSpan { span, depth }| {
            let (top, bottom) =
                span_card_y_extent(view, buffer, span, vis_start, vis_end, vtop, vbot);
            (bottom > top).then_some(QuoteExtent { top, bottom, depth })
        })
        .collect()
}

/// One visible blockquote level's painted extent: where it starts and ends vertically,
/// and how deeply it is nested (which is what decides the bar's horizontal offset).
#[derive(Clone, Copy, Debug)]
pub(super) struct QuoteExtent {
    pub(super) top: f32,
    pub(super) bottom: f32,
    /// 1-based, already clamped to `tags::MAX_QUOTE_DEPTH` by the renderer.
    pub(super) depth: u8,
}

/// The quote panel.
pub(super) fn draw_panel(snapshot: &gtk::Snapshot, ctx: &PaintCtx) {
    let (lm, card_w) = (ctx.lm, ctx.card_w);
    let quote_extents = &ctx.quote_extents;
    // Absent unless the active theme states `blockquote_bg`, read at PAINT
    // time for the same reason the heading band's fill is: selecting a theme
    // repaints, it does not re-render.
    //
    // The extent is the CONTENT COLUMN — `lm`/`card_w`, the same rect the
    // code-block card and the heading band take — so the panel starts at the
    // accent bar's own left edge and the two read as one object, and the
    // quoted text sits inset from both edges by `blockquote_bar_width +
    // blockquote_text_gap` (the `blockquote` tag's margins) rather than
    // running flush to the fill, which is what a panel wants and what a
    // paragraph background pinned to the text column could not give.
    //
    // DEPTH 1 ONLY, and that is the operator's ruling (2026-08-28) rather than an
    // oversight: the background does NOT nest. A nested level inherits its parent's
    // fill, so depth is carried by the bars alone and 18.29's "ONE continuous panel"
    // stays literally true. Painting a level's panel over its parent's would also
    // double any translucent `blockquote_bg`, so the inner region would read darker
    // for a reason no theme key asked for.
    if let Some(panel) = crate::theme::active().blockquote_bg {
        for e in quote_extents.iter().filter(|e| e.depth == 1) {
            snapshot.append_color(
                &panel,
                &graphene::Rect::new(lm, e.top, card_w, e.bottom - e.top),
            );
        }
    }
    draw_panel_scene(snapshot, ctx);
}

/// The quote panel's scene, in its BOTTOM-RIGHT corner (TDD 18.56).
///
/// Painted after the fill and over it — a scene composites rather than replaces, the
/// same rule `heading_band_scene` follows — and at depth 1 only, because the panel it
/// sits on is depth 1 only.
///
/// **Only where the quote's bottom is genuinely visible, and that gate is the whole
/// correctness argument.** `quote_extents` are viewport-CLAMPED by
/// `span_card_y_extent`, which is deliberate (GTK4Rs/AP-22: measuring an off-screen,
/// unvalidated iter blanks the view). A bottom-anchored decoration drawn from a clamped
/// bottom would pin itself to the VIEWPORT and slide up the quote as the reader scrolls
/// — ScrAP-333's shape, and the same trap the accent bar's tile phase already carries a
/// note about.
///
/// Asking for the true bottom instead is not available: obtaining it means measuring the
/// quote's last line while it is off-screen, which is the exact read the clamp exists to
/// prevent. So this takes the other branch — `bottom < vbot` means the clamp did not
/// bite and the bottom edge is the quote's own. When it did bite, the quote runs past
/// the bottom of the screen, so its floor is off-screen and there is nothing to draw:
/// the gate is not a compromise, it is the same picture.
fn draw_panel_scene(snapshot: &gtk::Snapshot, ctx: &PaintCtx) {
    let theme = crate::theme::active();
    let Some(scene) = theme.sprites.blockquote_scene.as_ref() else {
        return;
    };
    let zoom = ctx.imp.gutter_zoom.get();
    for e in ctx
        .quote_extents
        .iter()
        .filter(|e| e.depth == 1 && e.bottom < ctx.vbot)
    {
        let rect = graphene::Rect::new(ctx.lm, e.top, ctx.card_w, e.bottom - e.top);
        crate::widgets::draw_scene_corner(snapshot, &rect, scene, zoom);
    }
}

/// The quote's accent bar.
pub(super) fn draw_accent_bar(snapshot: &gtk::Snapshot, ctx: &PaintCtx) {
    let (lm, quote_extents) = (ctx.lm, &ctx.quote_extents);
    // Blockquote accent bars — same visible-only, viewport-clamped Y-extent
    // logic as the code-block backgrounds (so we never read an off-screen,
    // unvalidated iter — GTK4Rs/AP-22), but drawn as a thin vertical rect at the
    // body-text left margin. Blockquotes are buffer text, so there is no
    // anchored widget here to re-measure/churn (GTK4Rs/AP-23).
    let bar_color = *ctx.imp.bq_bar.borrow();
    // The bar's width is a themed decoration metric: a design-time px at
    // zoom 1.0, scaled here through the same `round(n * zoom)` the
    // `blockquote` tag scales its indent by (`tags.rs`). Scaling it is a
    // deliberate correction — the indent already scaled while the bar did
    // not, so a zoomed-in quote drew a hairline bar in a wide gutter. At
    // zoom 1.0 this is byte-identical to the previous constant.
    let bqm = crate::theme::active();
    let zoom_now = ctx.imp.gutter_zoom.get();
    let bar_w = crate::theme::px(bqm.metrics.blockquote_bar_width, zoom_now) as f32;
    // A theme may tile a sprite down the bar instead of filling it (TDD
    // 18.28), at the sprite's NATURAL size — `texture`, not `scaled`: 1:1
    // pixels need no filter, and GSK 4.6's `append_texture` offers no filter
    // choice (GTK4Rs/AP-114). The tile is clipped to the bar's own rect, so a
    // theme using one wants `blockquote_bar_width` at the tile's width.
    // The engine decides which of the bar's two appearances applies
    // (`theme::Fill`); this site renders the answer. `bar_color` is the
    // palette-derived default a theme that states neither key falls back
    // to, so it is passed in rather than re-derived here.
    let bar_decor = bqm.blockquote_bar_decor();
    // The tile is resampled so its WIDTH is the bar's width, and this is a correction
    // rather than a refinement. `blockquote_bar_width` is a themed pixel metric, so
    // `bar_w` above scales with zoom; a texture's natural size does not. Tiling the
    // natural plate into a zoomed bar therefore left a CLIPPED PARTIAL COLUMN down the
    // right-hand edge at every zoom but 1.0 — the gutter grew, the plate did not, and
    // `push_repeat` filled the remainder with a slice of the next tile. The defect is
    // horizontal only: the bar's height is the quote's extent, which no themed metric
    // governs.
    //
    // Height takes the SAME factor as the width so the tile keeps its aspect — deriving
    // it from the zoom independently would let rounding pull the two apart and shear the
    // pattern. Nearest-neighbour through `sprite::scaled` (cached per size, so a zoom
    // level costs one texture, not one per paint) for the reason every other resampled
    // sprite here takes it: GSK 4.6's `append_texture` filters linearly with no choice
    // (GTK4Rs/AP-114), which turns pixel art to mush.
    //
    // At zoom 1.0 a theme that sized `blockquote_bar_width` to its tile — which the key's
    // own comment tells it to — hits the `==` short-circuit and gets the natural texture
    // back, byte-identical to what this drew before.
    let bar_sprite = bar_decor.sprite.and_then(|s| {
        use gtk::gdk::prelude::TextureExt;
        let natural = crate::sprite::texture(s)?;
        let (tw, th) = (natural.width(), natural.height());
        let w = bar_w.round() as i32;
        if tw <= 0 || th <= 0 || w <= 0 {
            return None;
        }
        if w == tw {
            return Some(natural);
        }
        let h = (f64::from(th) * f64::from(w) / f64::from(tw)).round() as i32;
        crate::sprite::scaled(s, w, h)
    });
    // The SAME `quote_extents` the panel was filled from, so the bar and the
    // fill behind it can never disagree about where the quote starts or ends
    // — which is precisely how the TDD 18.29 defect announced itself.
    //
    // One bar PER LEVEL, each stepped in by its own depth (TDD 2.11b). The step is the
    // same `bar + gap` the `bq-{depth}` tag indents that level's text by, read from the
    // same theme keys, so a bar cannot drift from the column it marks (POLICY "One theme
    // key, every application path"). Depth is 1-based and already clamped by the
    // renderer, so `depth - 1` cannot underflow and the offset cannot run away on a
    // pathologically nested document.
    let bq_step = crate::theme::px(
        bqm.metrics.blockquote_bar_width + bqm.metrics.blockquote_text_gap,
        zoom_now,
    ) as f32;
    for &QuoteExtent { top, bottom, depth } in quote_extents {
        let x = lm + bq_step * f32::from(depth - 1);
        let rect = graphene::Rect::new(x, top, bar_w, bottom - top);
        // The sprite OUTRANKS the flat colour, and this is an `else`
        // rather than a paint-over on purpose: filling first and tiling
        // on top looks identical for an opaque tile and lets the flat
        // colour bleed through a transparent one — a bug reachable only
        // by the sprites nobody happened to test.
        match &bar_sprite {
            // `rect.y` here is `quote_extents`' viewport-CLAMPED top, so the
            // tile grid must NOT be anchored to it: `tile_texture` anchors at
            // the document instead, and its docs carry the measurement. The
            // clamp is right for the two draws that are position-invariant
            // (the panel fill, the flat bar) and wrong for the one that
            // carries a phase — same extent, one more consumer than it was
            // designed for.
            Some(tex) => crate::widgets::tile_texture(snapshot, &rect, tex),
            // Either the theme states no sprite, or the one it states
            // would not decode. Both degrade to the flat bar rather than
            // leaving a gap, which is `sdd/THEMING.md`'s inert-by-default
            // rule and what every sibling decoration does.
            // Either the theme states no sprite, or the one it states
            // would not decode. Both degrade to the flat bar, which is
            // the theme's own colour where it states one and the
            // palette-derived default where it does not.
            None => snapshot.append_color(&bar_decor.flat_or(bar_color), &rect),
        }
    }
}
