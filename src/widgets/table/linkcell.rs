//! **A link in a table cell** — the one widget shape it renders in, the single
//! activation path it shares with every other link in the document, and the hit-test
//! a hover needs.
//!
//! Every cell is a selectable `GtkLabel` whose markup carries any link as a Pango
//! `<a href>` (ScrAP-4), whether the link is the cell's whole content
//! (`| [Handbook](https://example.com) |`) or one word among others
//! (`| ☑ [#6378](…) |`). A link is then exactly its caption: the rest of the cell is
//! text, a click on the caption follows it, and a swipe across it selects it
//! (TDD 2.9a). A cell that is nothing but a link was once a `GtkLinkButton`, which
//! made the whole cell one click target whose caption could not be selected, and whose
//! caption lived one level inside the button where find could not see it (ScrAP-250).
//!
//! **The containment gate is bypassable**, and sealing it is what this module is for.
//! `GtkLabel` ships a default `activate-link` handler that calls `gtk_show_uri` with
//! the raw href (`gtk_label_activate_link`), so a handler that forgets to return
//! [`glib::Propagation::Stop`] hands `file:///etc/passwd` straight to the desktop.
//! Hence one activation function the cell's connection delegates to, which cannot
//! return anything else (GTK4Rs/AP-239).

use crate::codeview::CodePreviewView;
use gtk::prelude::*;
use gtk::{glib, Label};

/// The Pango markup that opens an inline link inside a cell's label, and its closing
/// tag [`LINK_MARKUP_CLOSE`]. Everything between them is the link's caption, which the
/// renderer emits exactly as it emits any other cell text.
///
/// **The `title` is escaped twice, and that is not a mistake.** It is what gives a
/// cell link the same hover tooltip a body link gets, and the two escapes undo two
/// different parsers: Pango unescapes the attribute value when it parses this markup,
/// then `gtk_label_query_tooltip` hands the *result* to `gtk_tooltip_set_markup`,
/// which parses it **again** (`gtklabel.c:1757`). A URL is the one string here that
/// routinely contains `&` (`?a=1&b=2`), so a single escape yields a tooltip that fails
/// to parse and silently shows nothing — the ScrAP-163 blank-label failure one layer
/// out. The `href` is escaped once: nothing parses it a second time.
pub(crate) fn link_markup_open(url: &str) -> String {
    let href = glib::markup_escape_text(url);
    let title = glib::markup_escape_text(&href);
    format!("<a href=\"{href}\" title=\"{title}\">")
}

/// Closes [`link_markup_open`].
pub(crate) const LINK_MARKUP_CLOSE: &str = "</a>";

/// Build a table cell's `GtkLabel` from its finished Pango `markup`, with link
/// activation already wired.
///
/// Every cell comes through here — including cells with no link at all — because
/// "does this cell contain a link?" is a question about the markup string, and
/// answering it at the call site is exactly how one cell ends up with the gate and
/// another without it. Wiring `activate-link`
/// unconditionally costs one signal connection on a label that will never emit it.
///
/// The caller owns the rest: `.cell` classes, wrapping, selection, alignment — the
/// cell's *context*, which this module has no view of.
pub(crate) fn cell_markup_label(markup: &str) -> Label {
    let label = Label::builder().label(markup).use_markup(true).build();
    label.connect_activate_link(|label, uri| activate_cell_link(label.upcast_ref(), uri));
    label
}

/// Open `url`, clicked on a link inside table cell `cell` — the single activation every
/// cell delegates to, and the reason none can diverge from a body link.
///
/// It resolves the preview view from the cell's own ancestry rather than taking one as
/// an argument, because a cell is built by the renderer *before* it is anchored: there
/// is no view to capture at construction time, and capturing one later would strand a
/// stale reference across a re-render.
///
/// **Always returns [`glib::Propagation::Stop`], and that return is the security
/// property.** `GtkLabel` ships a default `activate-link` handler that calls
/// `gtk_show_uri` with the raw href (`gtklabel.c:2081`), gate and all bypassed — so
/// `Proceed` here would hand `file:///etc/passwd` in a table cell straight to the
/// desktop. Pinned by the measured tests in `renderer::end`.
fn activate_cell_link(cell: &gtk::Widget, url: &str) -> glib::Propagation {
    match cell
        .ancestor(CodePreviewView::static_type())
        .and_then(|a| a.downcast::<CodePreviewView>().ok())
    {
        Some(view) => crate::preview::activate_link_url(&view, url),
        // A cell not (yet) inside a preview — a unit-test fixture, or a widget already
        // detached by a re-render. There is no document to resolve a relative
        // reference against and no heading map to scroll, so fall back to the external
        // gate, which admits http/https/mailto and refuses everything else.
        None => crate::links::open_url(url),
    }
    glib::Propagation::Stop
}

/// The URL of the link under `(x, y)` (the label's own widget coordinates) in a cell
/// label built by [`cell_markup_label`], or `None`.
///
/// **Not `GtkLabel::current_uri`**, which reads as the answer and is not one for a
/// hover. In GTK 4.6 it reports the hovered link only once a press has set
/// `link_clicked`; otherwise it reports the *focus* link — the one at the selection
/// caret (`gtk_label_get_current_uri` → `gtk_label_get_focus_link`) — so over a link
/// the pointer merely rests on it answers `None` (GTK4Rs/AP-342). The label's own
/// hit-test is private, so this repeats it: the layout's index under the point,
/// against the link ranges [`markup_links`] reads from the label's markup.
pub(crate) fn label_link_at(label: &Label, x: f64, y: f64) -> Option<String> {
    let markup = label.label();
    if !markup.contains("<a ") {
        return None;
    }
    let layout = label.layout();
    let (ox, oy) = label.layout_offsets();
    let scale = f64::from(gtk::pango::SCALE);
    let (inside, index, _) = layout.xy_to_index(
        ((x - f64::from(ox)) * scale) as i32,
        ((y - f64::from(oy)) * scale) as i32,
    );
    if !inside {
        return None;
    }
    let index = usize::try_from(index).ok()?;
    markup_links(&markup)
        .into_iter()
        .find(|link| link.text.contains(&index))
        .map(|link| link.url)
}

/// One `<a href>` in a cell label's markup: where its caption lies in the label's
/// displayed text, and where it leads.
#[derive(Debug, PartialEq)]
pub(crate) struct MarkupLink {
    /// Byte range into the label's displayed text — the same space as the label's
    /// `PangoLayout` indices.
    pub(crate) text: std::ops::Range<usize>,
    pub(crate) url: String,
}

/// Every `<a href>` in `markup`, as GTK records it.
///
/// Mirrors `parse_uri_markup` (GTK 4.6 `gtklabel.c`): a link's range is counted in
/// bytes of *decoded* text — entities resolved, every element's tags dropped, the text
/// inside Pango elements included — which is exactly the text the label's layout
/// holds. Only markup this module builds reaches here (glib-escaped text, the tags the
/// renderer emits, [`link_markup_open`]), so a malformed fragment ends the scan rather
/// than guessing.
pub(crate) fn markup_links(markup: &str) -> Vec<MarkupLink> {
    let mut links = Vec::new();
    let mut open: Option<(usize, String)> = None;
    let mut len = 0;
    let mut rest = markup;
    while let Some(ch) = rest.chars().next() {
        match ch {
            '<' => {
                let Some(end) = rest.find('>') else { break };
                let tag = &rest[1..end];
                if tag == "/a" {
                    if let Some((start, url)) = open.take() {
                        links.push(MarkupLink {
                            text: start..len,
                            url,
                        });
                    }
                } else if let Some(attrs) = tag.strip_prefix("a ") {
                    open = attr_value(attrs, "href").map(|url| (len, url));
                }
                rest = &rest[end + 1..];
            }
            '&' => {
                let Some(end) = rest.find(';') else { break };
                let Some(decoded) = decode_entity(&rest[1..end]) else {
                    break;
                };
                len += decoded.len_utf8();
                rest = &rest[end + 1..];
            }
            _ => {
                len += ch.len_utf8();
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    links
}

/// The decoded value of attribute `name` in a tag's attribute text.
fn attr_value(attrs: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let at = attrs
        .match_indices(&key)
        .find(|(i, _)| *i == 0 || attrs.as_bytes()[i - 1].is_ascii_whitespace())?
        .0;
    let value = &attrs[at + key.len()..];
    let value = &value[..value.find('"')?];
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let end = rest[amp..].find(';')? + amp;
        out.push(decode_entity(&rest[amp + 1..end])?);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// One XML entity's character — the five named ones `g_markup_escape_text` writes and
/// the numeric references it writes for control characters.
fn decode_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Display-free tests of the cell-link markup. `link_markup_open` builds a string and
/// Pango parses one; neither needs a GDK display, so these run in the default
/// `cargo test` rather than behind `gtk-integration-tests`.
#[cfg(test)]
mod tests {
    use super::{link_markup_open, markup_links, MarkupLink, LINK_MARKUP_CLOSE};
    use gtk::pango;

    /// A URL with an `&` in its query string — `?a=1&b=2` is the shape that breaks a
    /// single-escaped tooltip, and the shape half the URLs in a real document have.
    const AMPED: &str = "https://example.com/s?a=1&b=2";

    /// `<a href>` is **not** Pango markup — it is `GtkLabel` markup, and the
    /// distinction decides where this fragment can be validated.
    ///
    /// `GtkLabel` runs its own `GMarkupParser` over the string first, lifting out the
    /// `a` elements and recording their `href`/`title` (`gtklabel.c:3376`,
    /// `parse_uri_markup` `:3542`), and only hands what remains to
    /// `pango_parse_markup`. So Pango alone rejects this fragment, and asserting
    /// otherwise would be asserting against the wrong parser. That the whole thing
    /// renders is pinned where it can be — on a real label, in the integration tests
    /// below.
    #[test]
    fn cell_link_markup_is_gtk_label_markup_not_bare_pango() {
        let markup = format!("{}caption{LINK_MARKUP_CLOSE}", link_markup_open(AMPED));
        assert!(
            pango::parse_markup(&markup, '\0').is_err(),
            "if Pango has learned to parse `<a href>` on its own, the two-parser \
             reasoning behind the double-escaped title needs re-deriving: {markup}"
        );
    }

    /// The `title` survives **two** parsers, which is why it is escaped twice.
    ///
    /// Pango unescapes the attribute value when it parses the markup above; GTK then
    /// hands that result to `gtk_tooltip_set_markup`, which parses it AGAIN
    /// (`gtklabel.c:1757`). This asserts the intermediate — what Pango yields — is
    /// itself valid markup, and (the mutation the second escape exists to prevent)
    /// that the single-escaped form is NOT: without it a URL with `&` produces a
    /// tooltip that fails to parse and silently shows nothing.
    #[test]
    fn a_cell_links_tooltip_title_survives_both_parsers() {
        let once = gtk::glib::markup_escape_text(AMPED);
        assert!(
            link_markup_open(AMPED).contains(&format!("title=\"{}\"", once.replace('&', "&amp;"))),
            "the title must be escaped twice: {}",
            link_markup_open(AMPED)
        );
        assert!(
            pango::parse_markup(&once, '\0').is_ok(),
            "what Pango hands to gtk_tooltip_set_markup must itself be valid markup"
        );
        assert!(
            pango::parse_markup(AMPED, '\0').is_err(),
            "MUTATION CHECK: the raw URL must NOT be valid markup, or a single escape \
             would pass this file's tooltip contract and the second escape would be \
             dead weight"
        );
    }

    /// The `href` is escaped exactly ONCE — nothing parses it a second time, so a
    /// double escape there would hand `&amp;` to the browser as a literal.
    #[test]
    fn a_cell_links_href_is_escaped_exactly_once() {
        assert!(
            link_markup_open(AMPED).contains("href=\"https://example.com/s?a=1&amp;b=2\""),
            "{}",
            link_markup_open(AMPED)
        );
    }
    /// A cell link's range is in DECODED text bytes, the label layout's index space:
    /// entities count as the character they stand for, every tag counts as nothing,
    /// and text inside Pango elements counts. The href comes back unescaped.
    #[test]
    fn a_cells_link_ranges_are_counted_in_displayed_text() {
        let markup = format!(
            "<b>R&amp;D</b> ☑ {}see <i>it</i>{} &lt;x&gt; {}two{}",
            link_markup_open(AMPED),
            LINK_MARKUP_CLOSE,
            link_markup_open("https://two.example/"),
            LINK_MARKUP_CLOSE
        );
        let shown = "R&D ☑ see it <x> two";
        let links = markup_links(&markup);
        assert_eq!(
            links,
            vec![
                MarkupLink {
                    text: shown.find("see").unwrap()..shown.find(" <x>").unwrap(),
                    url: AMPED.to_string(),
                },
                MarkupLink {
                    text: shown.find("two").unwrap()..shown.len(),
                    url: "https://two.example/".to_string(),
                },
            ]
        );
    }

    /// Plain cell markup holds no link.
    #[test]
    fn a_cell_without_a_link_has_no_link_ranges() {
        assert!(markup_links("<b>bold</b> &amp; plain").is_empty());
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_integration_tests {
    /// A cell's link markup is **consumed by `GtkLabel`'s link parser**, which is
    /// what makes the caption a link rather than decorated text.
    ///
    /// The rendered text is the whole oracle, and it discriminates because of what
    /// the unit tests above establish: `<a href>` is not Pango markup. Only GtkLabel's
    /// own `<a>` handler (`gtklabel.c:3376`) can turn this string into
    /// `☑ #6378 tracked` — hand the same string to Pango and the parse FAILS, leaving
    /// the label EMPTY (the ScrAP-163 blank cell). So a non-empty, tag-free text here
    /// means the `a` element was lifted out and recorded as a link.
    ///
    /// No display geometry is involved: the link table is built when the markup is
    /// parsed, not when the label is drawn. The URL carries an `&`, so a title that
    /// only survives one escape shows up here as a parse failure and a blank cell.
    #[gtktest::test]
    fn a_cells_link_markup_is_parsed_as_a_link_not_as_text() {
        use super::{cell_markup_label, link_markup_open, LINK_MARKUP_CLOSE};
        const URL: &str = "https://example.com/i?a=1&b=2";
        let label = cell_markup_label(&format!(
            "\u{2611} {}#6378{LINK_MARKUP_CLOSE} tracked",
            link_markup_open(URL)
        ));
        assert_eq!(
            label.text(),
            "☑ #6378 tracked",
            "EMPTY means the markup failed to parse and the cell renders blank \
             (ScrAP-163); text still containing `<a href` means the link markup was \
             never emitted and the caption is inert text — the defect GTK4Rs/AP-239 fixed"
        );
    }
}
