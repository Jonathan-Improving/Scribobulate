//! Display-free tests for the inline-code chip's per-surface resolution.
//!
//! Every one of these is a colour question with a right answer, so none of them needs
//! a view — which is the point of resolving the chip here rather than in the tag
//! setup, where the same assertions would have needed a realized `GtkTextView`.

use super::*;
use crate::palette::{contrast, Palette};

/// A theme resolved from a `themes.toml` fragment, layered over the builtins.
///
/// Every fixture below states its own page, so [`palette_of`] needs no desktop probe
/// and none of these tests needs GTK.
fn themed(spec: &str) -> crate::theme::Theme {
    let mut themes = crate::theme::Themes::builtin();
    themes.merge_over_for_test(spec);
    themes.resolve("t")
}

fn builtin(id: &str) -> crate::theme::Theme {
    crate::theme::Themes::builtin().resolve(id)
}

/// The palette a theme that states its own page resolves to, without the desktop
/// probe `Palette::for_theme` would take — the same construction
/// `palette::tests::selected_text_clears_the_legibility_floor_on_every_theme` uses.
fn palette_of(t: &crate::theme::Theme) -> Palette {
    let bg = t.background.unwrap_or(gdk::RGBA::WHITE);
    let fg = t.foreground.unwrap_or(gdk::RGBA::BLACK);
    Palette::from_base(bg, fg, fg, t.accent_color.unwrap_or(fg), t)
}

fn rgba(hex: &str) -> gdk::RGBA {
    crate::theme::parse_color(hex).expect("test colour parses")
}

/// **The defect, stated as a ratio.** A chip derived from the PAGE, under a heading
/// ink chosen for its BAND, is unreadable — which is what Pixel Quest's h2 shipped:
/// cream `#fff3c4` on a chip tinted from a pale sky-blue page, ~1.3:1.
///
/// The gate is on the ink the surface actually carries, against the chip actually
/// drawn there, for every builtin theme and every surface — because "it looks right on
/// the page" is the reasoning that produced the defect. Its sibling in
/// `theme::tests::contrast` holds the same floor for everything that is not a chip.
#[test]
fn every_builtin_themes_ink_reads_on_its_own_chip() {
    let themes = crate::theme::Themes::builtin();
    for crate::theme::ChooserEntry { id, .. } in themes.chooser_list() {
        let theme = themes.resolve(&id);
        // A theme that states no page derives from the desktop, which owns its own
        // contrast — the same carve-out every legibility gate here takes.
        if theme.background.is_none() || theme.foreground.is_none() {
            continue;
        }
        let p = palette_of(&theme);
        let chips = p.code_chips;

        for level in 0..HEADING_LEVELS {
            let Some(chip) = chips.on(CodeSurface::Heading(level)) else {
                continue;
            };
            let ink = theme.heading_colors[level].unwrap_or(p.body_fg);
            let ratio = contrast(ink, chip);
            assert!(
                ratio >= 4.5,
                "{id}: h{} ink reads {ratio:.2}:1 on its own inline-code chip",
                level + 1
            );
        }

        if let Some(chip) = chips.on(CodeSurface::Quote) {
            let ink = theme.blockquote_fg.unwrap_or(p.body_fg);
            let ratio = contrast(ink, chip);
            assert!(
                ratio >= 4.5,
                "{id}: quote ink reads {ratio:.2}:1 on its chip"
            );
        }

        if let Some(chip) = chips.on(CodeSurface::TableHead) {
            let ink = theme.table_head_fg.unwrap_or(p.body_fg);
            let ratio = contrast(ink, chip);
            assert!(
                ratio >= 4.5,
                "{id}: table-header ink reads {ratio:.2}:1 on its chip"
            );
        }

        let ratio = contrast(p.body_fg, chips.page());
        assert!(
            ratio >= 4.5,
            "{id}: body ink reads {ratio:.2}:1 on its chip"
        );
    }
}

/// The page's chip is unchanged: the same 8% mix it has always been, and a theme that
/// states `code_inline_bg` still gets exactly what it stated (TDD 18.2).
#[test]
fn the_pages_chip_is_what_it_always_was() {
    let t = themed("[themes.t]\nbackground = \"#ffffff\"\nforeground = \"#111111\"\n");
    let p = palette_of(&t);
    assert_eq!(
        p.code_chips.page(),
        mix_rgba(p.page_bg, p.body_fg, TINT),
        "the page's chip must be byte-identical to the pre-surface derivation"
    );

    let stated = themed("[themes.t]\ncode_inline_bg = \"#123456\"\n");
    let p = palette_of(&stated);
    assert_eq!(p.code_chips.page(), rgba("#123456"));
}

/// A stated `code_inline_bg` answers for the PAGE and nowhere else. It was chosen
/// against the page, so carrying it onto a band is the original defect with a theme
/// key's blessing.
#[test]
fn a_stated_page_chip_does_not_reach_a_band() {
    let t = themed(
        "[themes.t]\nbackground = \"#ffffff\"\nforeground = \"#000000\"\n\
         code_inline_bg = \"#ffeedd\"\n\
         heading_band_color_h2 = \"#202040\"\nheading_color_h2 = \"#ffffff\"\n",
    );
    let p = palette_of(&t);
    let band_chip = p
        .code_chips
        .on(CodeSurface::Heading(1))
        .expect("a banded level has a chip");
    assert_ne!(band_chip, rgba("#ffeedd"));
    assert_eq!(band_chip, mix_rgba(rgba("#202040"), gdk::RGBA::WHITE, TINT));
}

/// An unbanded level has no chip of its own — the renderer never asks for one, and a
/// colour here would be a surface that is not drawn.
#[test]
fn an_unbanded_level_has_no_chip() {
    let t = themed("[themes.t]\nheading_band_color_h1 = \"#202040\"\n");
    let p = palette_of(&t);
    assert!(p.code_chips.on(CodeSurface::Heading(0)).is_some());
    for level in 1..HEADING_LEVELS {
        assert!(
            p.code_chips.on(CodeSurface::Heading(level)).is_none(),
            "h{} states no band and must offer no chip",
            level + 1
        );
    }
}

/// A gradient band's chip comes from its FIRST stop, where the text sits.
#[test]
fn a_gradient_band_tints_from_its_first_stop() {
    let t = themed(
        "[themes.t]\nheading_band_color_h1 = \"#300000\"\n\
         heading_band_gradient_to_color_h1 = \"#000030\"\nheading_color_h1 = \"#ffffff\"\n",
    );
    let p = palette_of(&t);
    assert_eq!(
        p.code_chips.on(CodeSurface::Heading(0)),
        Some(mix_rgba(rgba("#300000"), gdk::RGBA::WHITE, TINT))
    );
}

/// A translucent quote panel is flattened against the page before it is tinted —
/// otherwise the chip is derived from a colour no reader ever sees.
#[test]
fn a_translucent_quote_panel_is_flattened_first() {
    let t = themed(
        "[themes.t]\nbackground = \"#ffffff\"\nforeground = \"#000000\"\n\
         blockquote_bg = \"#00000080\"\n",
    );
    let p = palette_of(&t);
    let seen = mix_rgba(rgba("#ffffff"), rgba("#000000"), 128.0 / 255.0);
    let chip = p
        .code_chips
        .on(CodeSurface::Quote)
        .expect("a filled quote has a chip");
    // Same colour to within one 8-bit step — the alpha round-trips through f32.
    assert!(
        contrast(chip, mix_rgba(seen, rgba("#000000"), TINT)) < 1.02,
        "quote chip {chip:?} was not tinted from the composited panel"
    );
}

/// A quote the theme does not fill is a quote on the page: no panel, no chip of its
/// own, and `surface_at` never names it.
#[test]
fn an_unfilled_quote_stays_on_the_page() {
    let t = builtin(crate::theme::SYSTEM_ID);
    assert!(t.blockquote_bg.is_none(), "System fills no quote panel");
    let p = palette_of(&t);
    assert!(p.code_chips.on(CodeSurface::Quote).is_none());
    assert_eq!(surface_at(&t, None, 3), CodeRunSurface::Page);
}

/// A band made of a TILE alone has no colour to tint from, so there is no chip —
/// absent, not guessed from the page.
#[test]
fn a_tile_only_band_offers_no_chip() {
    let mut t = themed("[themes.t]\nbackground = \"#ffffff\"\nforeground = \"#111111\"\n");
    // Stand a decoded sprite in the h1 band slot without a fill beside it. Built
    // directly rather than through a theme key, because a key needs a file on disk and
    // the question here is purely "what does the resolver do with a sprite and no
    // flat rung?".
    t.sprites.heading_band[0] = Some(crate::sprite::SpriteRef::Compiled("nothing-in-particular"));
    let p = palette_of(&t);
    assert!(
        t.heading_band_decor(0).is_present(),
        "a sprite alone IS a band"
    );
    assert!(
        p.code_chips.on(CodeSurface::Heading(0)).is_none(),
        "a tile has no colour to tint a chip from"
    );
    // And the renderer still routes there, so the run takes the face and no fill.
    assert_eq!(surface_at(&t, Some(0), 0), CodeRunSurface::Heading(0));
}

/// The precedence: a banded heading wins over a quote it sits inside, because the band
/// is the nearer surface. h6-and-deeper folds onto h5's slot like every other key.
#[test]
fn the_nearer_surface_wins() {
    let t =
        themed("[themes.t]\nheading_band_color_h2 = \"#202040\"\nblockquote_bg = \"#101010\"\n");
    assert_eq!(surface_at(&t, Some(1), 2), CodeRunSurface::Heading(1));
    // h3 carries no band, so a run in one inside a quote takes the panel.
    assert_eq!(surface_at(&t, Some(2), 2), CodeRunSurface::Quote);
    assert_eq!(surface_at(&t, Some(2), 0), CodeRunSurface::Page);
    assert_eq!(surface_at(&t, None, 1), CodeRunSurface::Quote);
    // Over-deep levels clamp rather than index out of range.
    assert_eq!(
        surface_at(&t, Some(HEADING_LEVELS + 4), 0),
        CodeRunSurface::Page
    );
}

/// The header row is filled even where the theme states nothing, so its chip is
/// derived from the fill the palette produced — not from the page.
#[test]
fn the_header_cells_chip_follows_the_header_fill() {
    let t = themed("[themes.t]\ntable_head_bg = \"#22603a\"\ntable_head_fg = \"#ffd400\"\n");
    let p = palette_of(&t);
    assert_eq!(
        p.code_chips.on(CodeSurface::TableHead),
        Some(mix_rgba(rgba("#22603a"), rgba("#ffd400"), TINT))
    );

    let bare = themed("[themes.t]\nbackground = \"#ffffff\"\nforeground = \"#111111\"\n");
    let p = palette_of(&bare);
    assert_eq!(
        p.code_chips.on(CodeSurface::TableHead),
        Some(mix_rgba(p.table_head_bg, p.body_fg, TINT)),
        "an unstated header still has a fill, and the chip follows it"
    );
}
