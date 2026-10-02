//! The corpora for checks 21 and 22 — the register's next-free number, and a register
//! entry prescribing a `clippy.toml`-banned route.
//!
//! Same contract as `corpus.rs`: every case calls the predicate THE CHECK calls. Both checks
//! went in with no corpus at all, and check 22 had never produced a finding on the real
//! register: its warning words vetoed a whole line, and a register line of prose almost
//! always contains one of them somewhere. The MUST-FAIL cases below were each green under
//! that version, which is what proves they discriminate.

use crate::lint::checks::register::{banned_route_findings, next_free_verdict, NextFree};

// ── Check 21: the declared next-free number is free ──────────────────────────

#[test]
fn a_declared_number_below_the_highest_heading_is_taken() {
    let text = "# Register\n\nNext free number: **10**\n\n## 9. Old\n\n## 12. Newest\n";
    assert_eq!(
        next_free_verdict(text),
        NextFree::Taken {
            declared: 10,
            highest: 12
        }
    );
}

/// The boundary: a number equal to the highest heading already has a body. This is the
/// case a `>=` mutation of the comparison would let through.
#[test]
fn a_declared_number_equal_to_the_highest_heading_is_taken() {
    let text = "Next free number: 12\n\n## 12. Newest\n";
    assert_eq!(
        next_free_verdict(text),
        NextFree::Taken {
            declared: 12,
            highest: 12
        }
    );
}

/// Reserved gaps are legal: the header need only be ABOVE the highest heading.
#[test]
fn a_declared_number_above_the_highest_heading_is_free() {
    let text = "Next free number: 15\n\n## 9. Old\n\n## 12. Newest\n";
    assert_eq!(next_free_verdict(text), NextFree::Free);
}

#[test]
fn a_register_with_no_header_line_is_missing_not_free() {
    assert_eq!(next_free_verdict("## 12. Newest\n"), NextFree::Missing);
}

// ── Check 22: a register entry prescribing a banned route ────────────────────

const BANS: &str = r#"disallowed-methods = [
    { path = "gtk4::gdk::Texture::from_file", reason = "x" },
    { path = "gtk4::gdk_pixbuf::Pixbuf::from_stream", reason = "x" },
    { path = "gtk4::prelude::TextViewExt::scroll_to_mark", reason = "x" },
]"#;

/// Prescriptions. Each carries a word the whole-line veto took as a warning — elsewhere on
/// the line, or in a different sentence — and each was green under it.
const PRESCRIBES: &[&str] = &[
    // An unrelated "not" later in the line.
    "**Resolution**: load it with `Texture::from_file`; the old loader did not cache.",
    // The review's own example: "was " anywhere vetoed it.
    "**Resolution**: use `Texture::from_file` here — it was the fastest route.",
    // A warning word in the PREVIOUS sentence does not reach this one.
    "**Lesson**: never mind the old loader. Use `Texture::from_file` here.",
    // `ban` inside another word is not the word.
    "**Resolution**: the urban banner loads with `Texture::from_file`.",
    // "instead of" AFTER the name is the prescription's own alternative, not a warning.
    "**Resolution**: call `Texture::from_file` instead of the decoder.",
    // R6-AP-01: the commonest phrasings of a Resolution. A two-way connective ("avoid",
    // "instead of", "rather than") opens a clause whose prescription follows the comma,
    // and "used to"/"no longer"/a bare "never" say nothing about the named call. Each was
    // green under the 60-character window that vetoed on any of them.
    "**Resolution**: to avoid the copy, use `Texture::from_file`.",
    "**Resolution**: instead of the decoder, call `Texture::from_file`.",
    "**Resolution**: rather than decoding by hand, call `Texture::from_file`.",
    "**Resolution**: the decoder is no longer needed — call `Texture::from_file`.",
    "**Lesson**: never block the main loop: load with `Texture::from_file`.",
    "**Resolution**: the helper used to load it is `Texture::from_file`.",
    // A trait-method ban is spelled by its TYPE in prose, never by its `*Ext` trait: the
    // reader once built only `TextViewExt::scroll_to_mark`, which no register line writes.
    "**Resolution**: jump there with `TextView::scroll_to_mark`.",
    // ...or as a method call on a receiver.
    "**Resolution**: then call `view.scroll_to_mark(&mark, 0.0, false, 0.0, 0.0)`.",
];

/// Warnings, which must stay legal: an entry names a banned call in order to warn about it.
const WARNS_ABOUT: &[&str] = &[
    "**Lesson**: never use `Texture::from_file` for a document image.",
    "**Resolution**: do not reach for `Texture::from_file`; go through the decoder.",
    "**Scribobulate**: `gtk4::LinkButton::new` and `Texture::from_file` stay banned in clippy.toml.",
    "**Resolution**: route through the decoder rather than `Texture::from_file`.",
    // The two-way connectives still warn when the banned name is their direct object —
    // through a receiver, for the method-call spelling.
    "**Resolution**: avoid `Texture::from_file`; go through the decoder.",
    "**Resolution**: go through the decoder instead of `Texture::from_file`.",
    "**Resolution**: farscroll re-issues the scroll, rather than `view.scroll_to_mark(…)`.",
    "**Lesson**: never use `TextView::scroll_to_mark` on a view still being laid out.",
    "**Resolution**: you must not call `view.scroll_to_mark(…)` directly.",
    // A longer name that merely STARTS with a banned one is a different method.
    "**Resolution**: wrap `Pixbuf::from_stream_async` in the gate.",
    // Titles and TOC rows name the mistake — that is what an anti-pattern entry IS — and
    // the descriptive fields report what happened rather than what to do.
    "## 352. `GdkTexture::from_file` reaches a pixbuf module's INCREMENTAL path",
    "| 352 | `GdkTexture::from_file` reaches a pixbuf module's INCREMENTAL path |",
    "**Symptom**: `GdkTexture::from_file` on the same bytes succeeds — and leaks.",
    "**Root cause**: `GdkTexture::from_file` decodes a scalable source at its natural size.",
];

#[test]
fn a_prescription_of_a_banned_route_is_reported() {
    for line in PRESCRIBES {
        assert!(
            !banned_route_findings(BANS, line).is_empty(),
            "MISS (should flag): {line}"
        );
    }
}

#[test]
fn a_warning_about_a_banned_route_is_not_reported() {
    for line in WARNS_ABOUT {
        let found = banned_route_findings(BANS, line);
        assert!(found.is_empty(), "FALSE POSITIVE: {found:?}");
    }
}
