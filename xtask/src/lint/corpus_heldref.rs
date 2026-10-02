//! The corpus for check 24 — a held-reference construction site with no
//! Document-Reference CAM row — and, through it, for the test-module skipper.
//!
//! Same contract as `corpus.rs`: every case calls the predicate THE CHECK calls. The
//! skipper is where this check went blind: it opened a skip at ANY `#[cfg(…test…)]`
//! attribute and ran it to the next column-zero `}`, so a gated `mod t;` declaration hid
//! the whole next top-level item — measured at about 5,400 production lines of `src/`.

use crate::lint::checks::architecture::unrowed_captures;

const NAME: &str = "src/widget.rs";
/// A matrix that names some OTHER file, so any production capture in `NAME` is unrowed.
const SECTION: &str = "| src/other.rs | a row |\n";

/// The measured blind spot: an out-of-line test module's declaration is a one-line item,
/// and the production code below it is production code.
#[test]
fn a_capture_below_a_gated_mod_declaration_is_seen() {
    let text = "\
#[cfg(test)]
mod t;

impl Widget {
    fn hold(&self) {
        let span = AnchoredSpan::capture(source, range);
    }
}
";
    let found = unrowed_captures(NAME, text, SECTION);
    assert_eq!(found.len(), 1, "MISS: {found:?}");
    assert!(found[0].starts_with("src/widget.rs:6:"), "{found:?}");
}

/// A gated FIELD or a gated FUNCTION inside an impl is not a test module either.
#[test]
fn a_capture_below_a_gated_field_or_fn_is_seen() {
    let text = "\
struct Widget {
    #[cfg(test)]
    probe: u32,
}

impl Widget {
    #[cfg(any(test, feature = \"gtk-tests\"))]
    fn probe(&self) {}

    fn hold(&self) {
        let span = AnchoredSpan::capture(source, range);
    }
}
";
    assert_eq!(unrowed_captures(NAME, text, SECTION).len(), 1);
}

/// A real inline test module is still skipped — through nested braces, a brace in a
/// string and a char literal, and a second attribute between the gate and the `mod`.
/// Depth, not column zero, ends it, so production code AFTER it is seen again.
#[test]
fn an_inline_test_module_is_skipped_and_the_code_after_it_is_not() {
    let text = "\
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        if true {
            let s = \"}\";
            let c = '}';
            let span = AnchoredSpan::capture(source, range);
        }
    }
}

fn production() {
    let span = AnchoredSpan::capture(source, range);
}
";
    let found = unrowed_captures(NAME, text, SECTION);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("src/widget.rs:17:"), "{found:?}");
}

/// An indented inline test module (inside another module) is skipped to ITS close, not to
/// the next column-zero brace.
#[test]
fn a_nested_inline_test_module_ends_at_its_own_brace() {
    let text = "\
mod outer {
    #[cfg(test)]
    mod tests {
        fn f() { let _ = AnchoredSpan::capture(source, range); }
    }

    fn production() {
        let span = AnchoredSpan::capture(source, range);
    }
}
";
    let found = unrowed_captures(NAME, text, SECTION);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("src/widget.rs:8:"), "{found:?}");
}

/// `#[cfg(not(test))]` gates the PRODUCTION half of a pair.
#[test]
fn a_not_test_gate_is_production() {
    let text = "\
#[cfg(not(test))]
mod live {
    fn f() { let _ = AnchoredSpan::capture(source, range); }
}
";
    assert_eq!(unrowed_captures(NAME, text, SECTION).len(), 1);
}

/// A file the matrix names passes; so does the type's own module.
#[test]
fn a_rowed_file_and_the_types_own_module_pass() {
    let text = "fn f() { let _ = AnchoredSpan::capture(source, range); }\n";
    assert!(unrowed_captures(NAME, text, "| src/widget.rs | row |").is_empty());
    assert!(unrowed_captures("src/docref.rs", text, SECTION).is_empty());
}
