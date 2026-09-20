//! **The inline-code chip's fill, resolved against the SURFACE the run sits on.**
//!
//! An inline `code` span wears a fill one shade off its background — a chip. That
//! background used to be assumed to be the page, and the assumption is wrong wherever
//! the preview draws a surface of its own behind the text: a banded heading, a
//! blockquote panel, a table's header cell. On Pixel Quest's brown h2 band the chip
//! kept the page's pale blue and the heading's cream ink kept winning the foreground
//! (the chip tag sets a background and no ink, so the surrounding tag's ink shows
//! through) — cream on pale blue, ~1.3:1, an unreadable word inside a legible heading.
//! ScrAP-356 carries the measurement.
//!
//! So the chip is a RELATIONSHIP, not a colour: *this surface, tinted [`TINT`] of the
//! way toward the ink that surface carries*. Resolved once here, per surface, for
//! every renderer — the buffer's tags, the table cell's Pango markup, the HTML sink's
//! stylesheet and the PDF sink's spans — because a chip resolved per renderer is four
//! chances to answer a different question (POLICY "One theme key, every application
//! path").
//!
//! # Why the surrounding ink is left alone
//!
//! The chip sets no foreground anywhere. It does not need one: a fill that is 8% of
//! the way from a surface toward the ink already ON that surface cannot move their
//! contrast more than a few percent, so whatever ink the heading, the quote or the
//! header row chose for itself still reads. Giving the chip its own ink would instead
//! make it a fourth thing a theme has to tune per surface, and would silently outrank
//! the per-level heading colours a theme states.
//!
//! # A surface whose colour is unknown gets NO chip
//!
//! A band may be a tiled sprite with no flat rung stated, and then there is no colour
//! to tint. The answer is [`None`] — *absent, not guessed*, the same verdict
//! [`Band::is_present`](crate::theme::Band::is_present) gives the band itself: the run
//! keeps its monospace face on the tile, which is legible by construction because the
//! surface's own ink was chosen against that tile. Filling it from the page instead is
//! exactly the defect this module exists to remove.

use super::{contrast, mix_rgba, to_hex_opaque, WCAG_AA_TEXT};
use crate::theme::{BandPaint, Theme, HEADING_LEVELS};
use gtk::gdk;

/// How far from the surface toward its ink the chip sits.
///
/// 0.08 is the value the page's chip has always used; naming it is what lets every
/// other surface carry the same relationship rather than a second literal.
pub(crate) const TINT: f64 = 0.08;

/// Where an inline-code run sits when it is **document text** — the three surfaces a
/// document's own structure can put behind a run.
///
/// [`CodeSurface`]'s subset, and a separate type because a table's header CELL is not
/// document text on any surface: it is a Pango-markup label (preview), a `<th>` (HTML)
/// or its own measured run (PDF), so it can never be the answer to "where is this
/// run in the buffer?" — and a `match` that had to answer it anyway could only lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeRunSurface {
    Page,
    /// A banded heading level, `0..HEADING_LEVELS`.
    Heading(usize),
    /// Inside a blockquote the theme fills a panel for.
    Quote,
}

impl CodeRunSurface {
    /// The colour question this position asks.
    pub(crate) fn fill(self) -> CodeSurface {
        match self {
            CodeRunSurface::Page => CodeSurface::Page,
            CodeRunSurface::Heading(level) => CodeSurface::Heading(level),
            CodeRunSurface::Quote => CodeSurface::Quote,
        }
    }
}

/// Which surface an inline-code run sits on.
///
/// A closed vocabulary rather than a colour, because the *caller* knows where it is
/// and the *theme* knows what that place looks like, and only this module joins them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeSurface {
    /// The page — body prose, a list item, a table's BODY cell, an unbanded heading.
    Page,
    /// A banded heading level, `0..HEADING_LEVELS` (h1 · h2 · h3 · h4 · h5-and-deeper).
    Heading(usize),
    /// A blockquote's panel (`blockquote_bg`), where the theme fills one.
    Quote,
    /// A table's HEADER cell.
    TableHead,
}

/// The chip's fill on every surface, resolved from one theme.
///
/// `Copy` and colour-only: it holds no theme reference, so a renderer that has been
/// handed one cannot reach back for a second opinion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CodeChips {
    page: gdk::RGBA,
    heading: [Option<gdk::RGBA>; HEADING_LEVELS],
    quote: Option<gdk::RGBA>,
    table_head: Option<gdk::RGBA>,
}

impl CodeChips {
    /// Resolve every surface's chip.
    ///
    /// `page_bg`/`body_fg`/`table_head_bg` come from the [`Palette`](super::Palette)
    /// rather than the theme: under System they are the desktop's own colours, which
    /// no theme key states.
    pub(crate) fn resolve(
        theme: &Theme,
        page_bg: gdk::RGBA,
        body_fg: gdk::RGBA,
        table_head_bg: gdk::RGBA,
    ) -> Self {
        // The page's chip is the one a theme may state outright; every other surface
        // derives, because a stated colour was chosen against the PAGE and says
        // nothing about a band drawn over it.
        let page = theme
            .code_inline_bg
            .unwrap_or_else(|| tint(page_bg, body_fg));

        let heading = std::array::from_fn(|level| {
            let band = theme.heading_band_decor(level);
            // `heading_colors` is already folded with the bare `heading_color`, so an
            // unstated level indexes the theme's answer rather than re-deriving it.
            let ink = theme.heading_colors[level].unwrap_or(body_fg);
            band_surface(band.without_sprite()).map(|surface| tint(surface, ink))
        });

        let quote_ink = theme.blockquote_fg.unwrap_or(body_fg);
        let quote = theme
            .blockquote_panel_decor()
            .flat
            // A panel may be a translucent wash, and a chip tinted from the wash's own
            // premultiplied-nothing values would be a colour nobody ever sees. Flatten
            // it against the page first — which is exactly what the preview paints.
            .map(|panel| tint(over(page_bg, panel), quote_ink));

        let head_ink = theme.table_head_fg.unwrap_or(body_fg);
        let head_decor = theme.table_head_decor();
        let table_head = match band_surface(head_decor.without_sprite()) {
            Some(stated) => Some(stated),
            // A tile with no colour stated under it: the surface is the tile, and no
            // colour describes it.
            None if head_decor.sprite.is_some() => None,
            // Unlike a heading, the header row is filled even where the theme states
            // nothing — the palette derives one, and that derived fill is what the
            // cells' own CSS paints.
            None => Some(table_head_bg),
        }
        .map(|surface| tint(surface, head_ink));

        CodeChips {
            page,
            heading,
            quote,
            table_head,
        }
    }

    /// The chip's fill on `surface`, or `None` where that surface's colour is unknown
    /// (a tile) or absent (an unbanded heading level, an unfilled quote) — in which
    /// case the caller draws no chip at all.
    ///
    /// **A caller never falls back to [`CodeSurface::Page`] on `None`.** Falling back
    /// is the original defect: the page is not what is behind the run.
    pub(crate) fn on(&self, surface: CodeSurface) -> Option<gdk::RGBA> {
        match surface {
            CodeSurface::Page => Some(self.page),
            CodeSurface::Heading(level) => *self
                .heading
                .get(level.min(HEADING_LEVELS - 1))
                .unwrap_or(&None),
            CodeSurface::Quote => self.quote,
            CodeSurface::TableHead => self.table_head,
        }
    }

    /// The page's chip — the one surface that always has one, so callers that can only
    /// be on the page (the code-block panel's siblings, the HTML sink's bare `code`
    /// rule) need no `Option`.
    pub(crate) fn page(&self) -> gdk::RGBA {
        self.page
    }

    /// `#rrggbb` for `surface`, ready to interpolate — `None` where there is no chip.
    pub(crate) fn hex_on(&self, surface: CodeSurface) -> Option<String> {
        self.on(surface).map(to_hex_opaque)
    }
}

/// Where the renderer is, expressed as the surface behind it.
///
/// Pure, and the ONE definition of the precedence: a band the level actually carries
/// outranks a quote panel it may sit inside, and everything else is the page. The
/// renderer, both export sinks and the tag vocabulary read this rather than each
/// re-deciding — the shape `theme::decor` already imposes on every other decoration.
pub(crate) fn surface_at(
    theme: &Theme,
    heading_level: Option<usize>,
    quote_depth: usize,
) -> CodeRunSurface {
    if let Some(level) = heading_level {
        let level = level.min(HEADING_LEVELS - 1);
        if theme.heading_band_decor(level).is_present() {
            return CodeRunSurface::Heading(level);
        }
    }
    if quote_depth > 0 && theme.blockquote_panel_decor().is_present() {
        return CodeRunSurface::Quote;
    }
    CodeRunSurface::Page
}

/// The surface moved [`TINT`] off itself — **toward the ink, unless that would take
/// the ink below the legibility floor it currently clears**, in which case it moves
/// the same distance the other way.
///
/// Toward the ink is what a chip has always been and is the look this fixes nothing
/// about: on a light page it is the familiar grey plate. But a move toward the ink is
/// a move *down* the ink's own contrast, and a surface already sitting AT the floor
/// has none to give — measured on Pixel Quest, whose quote panel carries its navy ink
/// at 4.5:1 exactly and whose chip landed the word on it at 4.09:1. A decoration that
/// makes text unreadable has chosen wrong, so on that surface the chip moves away from
/// the ink instead: a lighter plate on a light panel, equally visible and legible by
/// construction.
///
/// Both directions keep the chip's DISTANCE from its surface identical, so "how loud
/// is a chip" stays one number.
fn tint(surface: gdk::RGBA, ink: gdk::RGBA) -> gdk::RGBA {
    let toward = mix_rgba(surface, ink, TINT);
    if contrast(ink, toward) >= WCAG_AA_TEXT {
        return toward;
    }
    // Away from the ink is toward whichever pole it is further from — the same
    // black/white escape `walk_to_contrast` takes when neither themed colour reads.
    let pole = if contrast(ink, gdk::RGBA::WHITE) >= contrast(ink, gdk::RGBA::BLACK) {
        gdk::RGBA::WHITE
    } else {
        gdk::RGBA::BLACK
    };
    let away = mix_rgba(surface, pole, TINT);
    // A theme whose ink was ALREADY below the floor on this surface cannot be rescued
    // by an 8% move, and the chip is not the place to try: take whichever direction
    // reads better and leave the surface's own contrast to `theme::tests::contrast`.
    if contrast(ink, away) > contrast(ink, toward) {
        away
    } else {
        toward
    }
}

/// `c` composited over `under`, so a translucent fill is tinted from the colour a
/// reader actually sees.
fn over(under: gdk::RGBA, c: gdk::RGBA) -> gdk::RGBA {
    mix_rgba(under, c, c.alpha() as f64)
}

/// A band's own colour where it has one: a gradient's FIRST stop (the chip sits on a
/// text line near the band's top, and a two-stop gradient has no single colour), else
/// the flat fill.
fn band_surface(paint: Option<BandPaint>) -> Option<gdk::RGBA> {
    match paint {
        Some(BandPaint::Gradient { from, .. }) => Some(from),
        Some(BandPaint::Flat(c)) => Some(c),
        None => None,
    }
}

#[cfg(test)]
mod tests;
