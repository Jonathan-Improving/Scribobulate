//! The packaging-metadata checks (19).
//!
//! These guard declarations that no build consumes and no test exercises. A `.desktop`
//! file is read by the desktop environment's menu builder at install time and by nothing
//! else in this tree, so an edit to it is validated by a human noticing, months later,
//! that the application is filed somewhere unexpected.

use super::{fail, header, pass};
use crate::lint::Tree;

const DESKTOP_ENTRY: &str = "data/scribobulate.desktop";

/// The main categories the application must be reachable from.
///
/// `Utility` alone is what this entry carried, and it buried a Markdown editor among
/// the calculators and archive managers. `Office` and `Development` are where the two
/// halves of the audience actually look: prose is written from one menu and a README from
/// the other, and the freedesktop menu spec permits several main categories precisely so
/// an application that belongs in both can say so.
const REQUIRED_MAIN: [&str; 3] = ["Office", "Development", "Utility"];

/// The registered main categories, freedesktop menu spec 1.1 (§ Main Categories).
///
/// A CLOSED list, frozen by that spec rather than by this project, which is why it can be
/// written out here without rotting. Its job is to catch a misspelling: `Developement` in
/// a `Categories=` line is not an error anywhere — the menu builder simply ignores a
/// category it does not know, and the entry silently loses a menu.
const MAIN_CATEGORIES: [&str; 13] = [
    "AudioVideo",
    "Audio",
    "Video",
    "Development",
    "Education",
    "Game",
    "Graphics",
    "Network",
    "Office",
    "Science",
    "Settings",
    "System",
    "Utility",
];

/// The non-main categories this entry is allowed to declare.
///
/// Deliberately only what the entry actually uses, so the check FAILS CLOSED: an
/// additional category added later is unrecognised until someone lists it here, which
/// forces the question the spec asks about every additional category — which main
/// category does it require? `WordProcessor` and `Publishing` require `Office`;
/// `TextEditor` requires `Utility` or `Development`. Declaring one whose main category is
/// absent produces an entry the menu builder is free to drop.
const ALLOWED_ADDITIONAL: [&str; 2] = [
    // Requires Utility or Development — both present, see REQUIRED_MAIN.
    "TextEditor",
    // Toolkit hint; the spec reserves no main category for it.
    "GTK",
];

/// Check 19 — the desktop entry lists the menus the application must appear under.
///
/// Not a restatement of the file: the file says which categories are declared, this says
/// which ones are *required*, and the gap between those two is the regression. The
/// tempting tidy-up — collapsing a multi-main `Categories=` back to one because
/// `desktop-file-validate` emits a hint about it — is exactly the edit that reads as
/// cleanup and is a behaviour change. That hint is advisory ("might appear more than once
/// in the application menu") and appearing more than once is the intent here.
pub fn desktop_categories(tree: &Tree) -> bool {
    header(
        "19",
        "the desktop entry declares its required menu categories",
    );

    let Some(text) = tree.text(DESKTOP_ENTRY) else {
        return fail(
            &format!("{DESKTOP_ENTRY} is missing or unreadable"),
            &[],
            &[],
        );
    };

    let findings = category_findings(text);
    if findings.is_empty() {
        return pass();
    }
    fail(
        &format!("{DESKTOP_ENTRY} would not file the application where it belongs:"),
        &findings,
        &[
            "REQUIRED_MAIN in this check is the source of truth for which menus we claim.",
            "An unrecognised category is a typo or a deliberate addition: fix it, or add it",
            "to ALLOWED_ADDITIONAL along with the main category the spec says it requires.",
        ],
    )
}

/// Every way `text` fails check 19, empty when it passes.
///
/// Split from the check so the corpus can exercise THE PREDICATE the gate runs rather than
/// a copy of it, and so a mutation can be expressed as a string instead of an edit to a
/// tracked file.
pub fn category_findings(text: &str) -> Vec<String> {
    // The last unindented `Categories=` wins, the way the menu builder's parser reads it.
    // A commented-out line (`#Categories=`) is not a declaration and must not satisfy this
    // check -- that is the difference between `strip_prefix` and a `contains` test.
    let Some(value) = text
        .lines()
        .filter_map(|line| line.strip_prefix("Categories="))
        .next_back()
    else {
        return vec![format!(
            "no Categories= line; add one terminating each of: {};",
            REQUIRED_MAIN.join(";")
        )];
    };

    let mut findings = Vec::new();

    // The spec requires the trailing semicolon: `Categories=` is a multi-value key and
    // each value is semicolon-TERMINATED, not semicolon-separated. Without it the final
    // category is malformed, so the app loses whichever menu happens to be written last.
    if !value.ends_with(';') {
        findings.push(format!(
            "the Categories= value '{value}' has no trailing ';' (values are terminated, not separated)"
        ));
    }

    let declared: Vec<&str> = value.split(';').filter(|field| !field.is_empty()).collect();

    for required in REQUIRED_MAIN {
        if !declared.contains(&required) {
            findings.push(format!("'{required}' is not declared"));
        }
    }

    for field in &declared {
        if MAIN_CATEGORIES.contains(field) || ALLOWED_ADDITIONAL.contains(field) {
            continue;
        }
        // `X-` is the spec's escape hatch for a vendor category; it cannot be misspelled
        // into something meaningful, so there is nothing here to catch.
        if field.starts_with("X-") {
            continue;
        }
        findings.push(format!(
            "'{field}' is neither a registered main category nor a listed additional one"
        ));
    }

    findings
}
