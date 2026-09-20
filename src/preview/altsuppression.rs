//! **An image's alt text reaches the render as nothing** (TDD 2.5) — the contract
//! `Renderer::alt_suppressed` states, asserted against real renders.
//!
//! Test-only, and carrying `gtk-integration-tests` rather than a bare `#[cfg(test)]`
//! for the reason `codeview::painttest` gives: a render needs a `GtkTextBuffer`.
//!
//! Its own module rather than more of `build.rs`, which is already far past the
//! file-size soft limit, and because the subject is one contract rather than one
//! function: the leak these guard was `Event::Text` being the ONLY event the
//! suppression named, so what has to be asserted is the *other* events an alt is
//! made of — and each of them is a separate document.

use super::build::build_render_products;
use gtk::prelude::*;

/// The buffer's text, with a `U+FFFC` standing in for each anchored child.
fn slice(products: &super::build::RenderProducts) -> String {
    let buf = &products.buf;
    buf.slice(&buf.start_iter(), &buf.end_iter(), true)
        .to_string()
}

/// How many images this render anchored, counted by the two widget shapes the
/// renderer builds for one: a `GtkOverlay` (a loaded `GtkPicture` under its
/// selection tint, `anchor_image`) and a `GtkImage` (the broken-image placeholder,
/// `anchor_broken`). Named positively rather than as "everything that is not a
/// table, a rule or a disclosure toggle" — the negative reads as complete and stops
/// being so the moment a fifth anchored child kind appears.
fn image_count(products: &super::build::RenderProducts) -> usize {
    products
        .anchored
        .iter()
        .filter(|(_, w)| w.is::<gtk::Overlay>() || w.is::<gtk::Image>())
        .count()
}

/// Both images below resolve to nothing on disk, so each anchors the broken-image
/// placeholder — which is the *point*: an unresolvable image still stands in for its
/// own alt (`image_placeholder_tooltip`), so these documents exercise the suppression
/// rather than an image that happened to load.
const SRC: &str = "no-such-file.png";

/// **CONTROL — the oracle discriminates.**
///
/// Outside an image, a backtick span renders its content into the buffer. Without
/// this, every assertion below would pass just as happily against a renderer that had
/// stopped rendering inline code at all.
#[gtktest::test]
fn an_inline_code_span_outside_an_image_still_reaches_the_buffer() {
    let products = build_render_products("before `code` after\n", None, 1.0, false);
    assert!(
        slice(&products).contains("code"),
        "CONTROL FAILED: inline code no longer reaches the buffer at all, so the \
         alt-suppression assertions below prove nothing — buffer = {:?}",
        slice(&products)
    );
}

/// **The reported defect**: a backtick span in an alt rendered beside the picture.
///
/// `Event::Text` was the only event the suppression named, so the alt's *other*
/// events walked straight into the document — here as the inline-code content
/// `code`, tagged and inked, immediately after the image's own `U+FFFC`.
#[gtktest::test]
fn an_inline_code_span_in_an_image_alt_reaches_the_buffer_as_nothing() {
    let md = format!("before\n\n![a `code` b]({SRC})\n\nafter\n");
    let products = build_render_products(&md, None, 1.0, false);
    assert_eq!(
        slice(&products),
        "before\n\n\u{fffc}\n\nafter",
        "the whole alt — backtick span and plain words alike — renders as the \
         placeholder's single U+FFFC and nothing else"
    );
    assert_eq!(image_count(&products), 1, "one placeholder, and only one");
}

/// A nested image is not an image: CommonMark folds it into the outer alt string,
/// which the outer picture already stands in for.
///
/// Two failures in one document, both from the suppression being a `bool`: the inner
/// `Start(Image)` resolved and anchored a SECOND placeholder, and its `TagEnd::Image`
/// cleared the flag — so the outer alt's tail (` tail`) rendered as body text.
#[gtktest::test]
fn a_nested_image_in_an_alt_anchors_nothing_and_does_not_end_the_outer_alt() {
    let md = format!("before\n\n![![inner](inner.png) tail]({SRC})\n\nafter\n");
    let products = build_render_products(&md, None, 1.0, false);
    let text = slice(&products);
    assert_eq!(
        image_count(&products),
        1,
        "only the OUTER image is drawn; buffer = {text:?}"
    );
    assert!(
        !text.contains("tail") && !text.contains("inner"),
        "the outer alt runs on past the nested image's end — none of it may render \
         — buffer = {text:?}"
    );
}

/// Raw inline HTML in an alt reached the image scanner, which resolved and anchored
/// it — and with "Show Unsafe Images" on would have FETCHED a remote one. The same
/// hazard the collapsed-body gate closes (ScrAP-147 / `render_image_slot`), arriving
/// by a second door: an `<img>` a reader cannot see, inside the alt of one they can.
#[gtktest::test]
fn raw_inline_html_in_an_image_alt_anchors_no_image() {
    let md = format!("before\n\n![x <img src=\"other.png\"> y]({SRC})\n\nafter\n");
    let products = build_render_products(&md, None, 1.0, false);
    assert_eq!(
        image_count(&products),
        1,
        "the `<img>` inside the alt must not be resolved, loaded or anchored; \
         buffer = {:?}",
        slice(&products)
    );
}

/// A link inside an alt is alt text too — it must record no clickable range.
///
/// The leak here was quieter than the others: `Start(Link)` opened a link range and
/// `TagEnd::Link` closed it, so the render carried a zero-width link at the image's
/// own offset with nothing to click.
#[gtktest::test]
fn a_link_in_an_image_alt_records_no_clickable_range() {
    let md = format!("before\n\n![a [l](https://example.invalid/) b]({SRC})\n\nafter\n");
    let products = build_render_products(&md, None, 1.0, false);
    assert!(
        products.maps.links.is_empty(),
        "the alt's link is alt text, not a link: {:?}",
        products.maps.links
    );
}

/// Document Rendering CAM rows 2 and 5 — the container context, and its copy map.
///
/// A table cell renders through a `GtkLabel` rather than the buffer, and it counts
/// its own offsets: the alt must contribute no cell TEXT (the leak) and no cell
/// WIDTH (which would start a copy out of that cell in the wrong place).
#[gtktest::test]
fn an_image_alt_in_a_table_cell_contributes_no_text_and_no_offset() {
    let md = format!("| c |\n|---|\n| ![a `q` b]({SRC}) tail |\n");
    let products = build_render_products(&md, None, 1.0, false);
    let labels = super::cells::collect_cell_labels(&products.anchored);
    let texts: Vec<String> = labels.iter().map(|l| l.text().to_string()).collect();
    assert_eq!(
        texts,
        vec!["c".to_string(), " tail".to_string()],
        "the body cell holds only the text OUTSIDE the image; the alt — plain words \
         and backtick span alike — reaches no label"
    );
    // The cell's own copy map must agree with what the label holds: resolving the
    // whole cell may not reach back into the alt for characters that are not there.
    let map = super::cells::cell_copymap(&labels[1]).expect("the body cell has a copy map");
    assert_eq!(
        crate::copymap::resolve_cell(&map, &md, 0, 5),
        " tail",
        "a copy of the cell's five rendered characters is its five rendered \
         characters — an alt counted into the cell's offsets shifts this right"
    );
}
