//! What a disclosure block is called, so a control can find it again after the
//! document has moved underneath it. Pure; no GTK.
//!
//! # Why the identity is the OPENING DELIMITER and nothing more
//!
//! A control built by one render is clicked against a later document, so it carries a
//! [`crate::docref::AnchoredSpan`] — the construct's own text beside its offset — and
//! re-resolves at the click. What that text is decides what survives:
//!
//! - **The whole block** would be destroyed by its own body: typing a character inside
//!   a disclosure would change the identity of the disclosure being typed in, so the
//!   reader's own edit would strand the control above it.
//! - **The `<details>` tag alone** is `<details>` in nearly every document, so it names
//!   every disclosure at once and resolves to none of them.
//!
//! The delimiter — `<details …>` through the `</summary>` that follows it, when one
//! does — is the widest span that is stable under an edit to the body and the narrowest
//! that carries the reader-visible label. It is still not unique (two blocks may share a
//! summary), which is why the span is captured under
//! [`Ambiguity::Unique`](crate::docref::Ambiguity::Unique) rather than nearest-match:
//! for a fold, choosing wrongly toggles a block the reader was not pointing at.
//!
//! # The synthetic case
//!
//! Front matter renders as a `<details>` that the document does not contain — the
//! events tile the real fence bytes ([`super::super::frontmatter`]) — so there is no
//! tag at its offset to anchor on. Its identity is the **opening fence line**, which
//! is not distinctive (`---` is also a thematic break). That is safe for the one reason
//! that applies to no other block: front matter is by definition what a document
//! OPENS with, so its offset is 0 and the exact-offset fast path always hits. If front
//! matter ever becomes relocatable, this anchor stops resolving and every front-matter
//! toggle starts re-deriving instead — visibly, not silently.

use std::ops::Range;

/// How far past the block's start an opening delimiter may run. A summary is one
/// reader-visible line; the bound exists so that anchoring a block near the top of a
/// large document does not scan (and lowercase) the rest of it.
const WINDOW: usize = 4096;

/// The source range of the disclosure delimiter opening at `at`, or `None` when `at`
/// does not begin one.
///
/// `md` is the **cleaned** source the render walked, which is the space a
/// [`FoldKey`](crate::fold::FoldKey) is measured in.
pub(crate) fn opening_delimiter(md: &str, at: usize) -> Option<Range<usize>> {
    let rest = md.get(at..)?;
    let mut end = WINDOW.min(rest.len());
    while !rest.is_char_boundary(end) {
        end -= 1;
    }
    let window = &rest[..end];
    // Names and boolean attributes are case-insensitive; the lowercased twin is
    // byte-for-byte the same length, so its offsets index `window` too (the rule
    // `renderer::rawhtml::lex` already relies on).
    let lower = window.to_ascii_lowercase();
    if !lower.starts_with("<details") {
        // The synthetic block: anchor on the line at `at`, terminator included.
        let line_end = window.find('\n').map_or(window.len(), |i| i + 1);
        return (line_end > 0).then(|| at..at + line_end);
    }
    let tag_end = lower.find('>')? + 1;
    // A `</summary>` belonging to a LATER block is not this one's label, so the search
    // stops at whichever disclosure boundary comes first.
    let bound = ["<details", "</details"]
        .iter()
        .filter_map(|needle| lower[tag_end..].find(needle).map(|i| tag_end + i))
        .min()
        .unwrap_or(lower.len());
    let delimiter_end = match lower[tag_end..bound].find("</summary>") {
        Some(i) => tag_end + i + "</summary>".len(),
        None => tag_end,
    };
    Some(at..at + delimiter_end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delimiter_runs_from_the_tag_through_the_summary_that_labels_it() {
        let md = "lead\n\n<details>\n<summary>One</summary>\n\nbody\n\n</details>\n";
        let at = md.find("<details>").unwrap();
        let got = opening_delimiter(md, at).expect("a disclosure opens here");
        assert_eq!(&md[got], "<details>\n<summary>One</summary>");
    }

    #[test]
    fn an_edit_inside_the_body_leaves_the_identity_intact() {
        // The whole reason the body is excluded: the reader's own typing must not
        // rename the block they are typing in.
        let md = "<details>\n<summary>One</summary>\n\nbody\n\n</details>\n";
        let before = opening_delimiter(md, 0).map(|r| md[r].to_string());
        let edited = md.replace("body", "body, now longer");
        let after = opening_delimiter(&edited, 0).map(|r| edited[r].to_string());
        assert_eq!(before, after);
        assert_eq!(before.as_deref(), Some("<details>\n<summary>One</summary>"));
    }

    #[test]
    fn the_open_attribute_is_part_of_the_tag_and_stays_in_the_identity() {
        let md = "<details open>\n<summary>Two</summary>\n\nb\n\n</details>\n";
        let got = opening_delimiter(md, 0).unwrap();
        assert_eq!(&md[got], "<details open>\n<summary>Two</summary>");
    }

    #[test]
    fn a_block_with_no_summary_anchors_on_its_tag_alone() {
        // Malformed input is rendered, not judged (rubric 2.26d), so it must still
        // produce an identity — a weak one, which `Ambiguity::Unique` then refuses to
        // guess with.
        let md = "<details>\n\nbody\n\n</details>\n";
        assert_eq!(&md[opening_delimiter(md, 0).unwrap()], "<details>");
    }

    #[test]
    fn a_later_blocks_summary_is_not_borrowed_as_this_ones_label() {
        // Two adjacent `<details>` with no blank line between them are ONE raw-HTML
        // block (the compact GitHub form), so the second's `</summary>` sits inside
        // this one's window.
        let md = "<details>\n<details>\n<summary>Inner</summary>\n</details>\n</details>\n";
        assert_eq!(&md[opening_delimiter(md, 0).unwrap()], "<details>");
        let second = md[1..].find("<details>").unwrap() + 1;
        assert_eq!(
            &md[opening_delimiter(md, second).unwrap()],
            "<details>\n<summary>Inner</summary>"
        );
    }

    #[test]
    fn the_tag_name_is_matched_case_insensitively() {
        let md = "<DETAILS OPEN>\n<SUMMARY>Shouty</SUMMARY>\n</DETAILS>\n";
        assert_eq!(
            &md[opening_delimiter(md, 0).unwrap()],
            "<DETAILS OPEN>\n<SUMMARY>Shouty</SUMMARY>"
        );
    }

    #[test]
    fn a_synthetic_block_anchors_on_the_fence_line_at_the_top_of_the_document() {
        let md = "---\ntitle: x\n---\n\nbody\n";
        assert_eq!(&md[opening_delimiter(md, 0).unwrap()], "---\n");
    }

    #[test]
    fn an_offset_outside_the_document_yields_nothing_rather_than_panicking() {
        let md = "<details>\n<summary>One</summary>\n</details>\n";
        assert_eq!(opening_delimiter(md, md.len() + 1), None);
        assert_eq!(opening_delimiter(md, md.len()), None);
        assert_eq!(opening_delimiter("", 0), None);
        // Not on a char boundary: `é` is two bytes.
        assert_eq!(opening_delimiter("é", 1), None);
    }

    #[test]
    fn the_identity_never_runs_past_the_window() {
        // A pathological `<details` with no `>` at all must not scan the document.
        let md = format!("<details{}", "x".repeat(WINDOW * 2));
        assert_eq!(opening_delimiter(&md, 0), None);
    }
}
