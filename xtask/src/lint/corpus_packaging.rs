//! The corpus for check 19 — the gate on the desktop-entry gate.
//!
//! Same contract as `corpus.rs`: every case calls the predicate THE CHECK calls
//! (`category_findings`), never a copy of it, because a corpus over a re-implementation is
//! evidence about the copy.
//!
//! A file of its own rather than cases in `corpus.rs`: that file is already twice the size
//! limit and carries a self-exclusion from the gate's text checks, which nothing here
//! needs — no case below quotes a citation form.
//!
//! **The cases that matter are the last one or two in each group.** A corpus of things that
//! obviously work proves nothing; each group ends on the case that would silently make the
//! check lenient, or the one a plausible edit actually produces.

use crate::lint::checks::packaging::category_findings;

/// The real entry, reduced to the keys the predicate reads. Every mutation below is this
/// with one line changed, so a case says what it tests by what it differs in.
const GOOD: &str = "\
[Desktop Entry]
Type=Application
Name=Scribobulate
Categories=Office;Development;Utility;TextEditor;GTK;
MimeType=text/markdown;
";

fn findings(entry: &str) -> Vec<String> {
    category_findings(entry)
}

// ── The passing subject ───────────────────────────────────────────────────────

#[test]
fn the_delivered_categories_line_passes() {
    assert!(findings(GOOD).is_empty(), "{:?}", findings(GOOD));
}

/// Order carries no meaning in the spec, and a check that accidentally depended on it
/// would fail the day someone alphabetised the line — a change with no effect on any menu.
#[test]
fn the_order_of_the_categories_does_not_matter() {
    let reordered = GOOD.replace(
        "Categories=Office;Development;Utility;TextEditor;GTK;",
        "Categories=GTK;TextEditor;Utility;Development;Office;",
    );
    assert!(
        findings(&reordered).is_empty(),
        "{:?}",
        findings(&reordered)
    );
}

/// `X-` is the spec's vendor escape hatch. It must pass without being listed, or every
/// downstream packager's addition becomes this gate's problem.
#[test]
fn a_vendor_prefixed_category_is_allowed_unlisted() {
    let vendored = GOOD.replace("GTK;", "GTK;X-SuSE-Core-Office;");
    assert!(findings(&vendored).is_empty(), "{:?}", findings(&vendored));
}

// ── The regressions it exists to catch ────────────────────────────────────────

/// THE regression. `desktop-file-validate` emits a hint about multiple main categories,
/// and acting on that hint reads as cleanup — this is the edit it produces.
#[test]
fn collapsing_back_to_a_single_main_category_fails() {
    let collapsed = GOOD.replace(
        "Categories=Office;Development;Utility;TextEditor;GTK;",
        "Categories=Utility;TextEditor;GTK;",
    );
    let found = findings(&collapsed);
    assert!(
        found.iter().any(|f| f.contains("'Office' is not declared"))
            && found
                .iter()
                .any(|f| f.contains("'Development' is not declared")),
        "{found:?}"
    );
}

/// A misspelled category is an error nowhere else in the toolchain: the menu builder
/// ignores what it does not recognise, so the entry silently loses a menu and every other
/// gate stays green. It must fail twice — the menu is missing AND the word is not a
/// category — because either finding alone sends the reader to the wrong fix.
#[test]
fn a_misspelled_main_category_fails_as_both_missing_and_unrecognised() {
    let typo = GOOD.replace("Development;", "Developement;");
    let found = findings(&typo);
    assert!(
        found
            .iter()
            .any(|f| f.contains("'Development' is not declared")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|f| f.contains("'Developement'")
                && f.contains("neither a registered main category")),
        "{found:?}"
    );
}

/// Values are semicolon-TERMINATED, not separated. Dropping the last one is the natural
/// mistake when appending by hand, and it malforms the final category only — so the app
/// loses whichever menu happens to be written last, which is not the one being edited.
#[test]
fn a_missing_trailing_semicolon_fails() {
    let unterminated = GOOD.replace("TextEditor;GTK;", "TextEditor;GTK");
    let found = findings(&unterminated);
    assert!(
        found.iter().any(|f| f.contains("no trailing ';'")),
        "{found:?}"
    );
}

/// An additional category whose required main category is absent. `WordProcessor` requires
/// `Office`; adding it while dropping `Office` produces an entry the menu builder may drop
/// entirely. The check catches it by failing closed on the unlisted word rather than by
/// carrying a partial table of the spec's requirements — a table covering only the
/// categories we happen to use would read as coverage and would not be.
#[test]
fn an_unlisted_additional_category_fails_closed() {
    let added = GOOD.replace("TextEditor;", "TextEditor;WordProcessor;");
    let found = findings(&added);
    assert!(
        found.iter().any(|f| f.contains("'WordProcessor'")),
        "{found:?}"
    );
}

// ── Leniency the parser must not grant ────────────────────────────────────────

/// The case that would make the whole check ornamental: a `contains("Categories=")` test
/// is satisfied by a comment, and this file already carries a comment line above `Exec`,
/// so a commented-out declaration is a realistic state for it to be in.
#[test]
fn a_commented_out_declaration_does_not_satisfy_the_check() {
    let commented = GOOD.replace("Categories=", "#Categories=");
    let found = findings(&commented);
    assert!(
        found.iter().any(|f| f.contains("no Categories= line")),
        "{found:?}"
    );
}

/// `MimeType=text/markdown;` also ends in a semicolon and also holds a multi-value list.
/// A predicate that scanned for any terminated list, rather than for this key, would read
/// it as the subject and report every required category missing from a line that never
/// claimed to hold one.
#[test]
fn another_multi_value_key_is_not_mistaken_for_the_categories_line() {
    let no_categories = GOOD.replace(
        "Categories=Office;Development;Utility;TextEditor;GTK;\n",
        "",
    );
    let found = findings(&no_categories);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("no Categories= line"), "{found:?}");
}
