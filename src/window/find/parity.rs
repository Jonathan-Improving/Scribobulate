//! **The two find engines, measured against each other.**
//!
//! The find bar drives two entirely separate matchers: `GtkSourceSearchContext` over the
//! editor's source buffer, and [`super::matcher::Matcher`] over the preview's three
//! texts. TDD 11.13 and 11.14 claim a query means the *same thing* in both panes. That
//! claim is not provable by reading either implementation — it is a claim about an
//! agreement between them, so the only evidence that settles it is running both over one
//! fixture and comparing the counts.
//!
//! Everything here is therefore a comparison, never an assertion about one engine's
//! output in isolation: a case that hard-codes the number it expects stops being able to
//! tell you the engines have drifted, which is the entire failure this file exists to
//! catch.
//!
//! **This is a probe that became a gate.** Five questions about GtkSourceView's own
//! regular-expression handling decided the matcher's design — how whole-word wraps a
//! pattern, which characters bound a word, which compile flags are set, and what a
//! zero-width pattern does. Each was answered here by measurement rather than by reading
//! GtkSourceView's C source, because a source reading proves what one engine does and
//! this file has to prove what two engines do *together*. The answers are recorded beside
//! the case that measured them.
//!
//! The fixtures are deliberately plain text with no Markdown structure: the subject is
//! the matcher, and a fixture the renderer would transform would measure the renderer
//! too.

use super::matcher::Matcher;
use super::options::FindOptions;
use gtk::prelude::*;
use sourceview::prelude::*;

/// Count `query` in `hay` with the editor's engine, under `opts`.
///
/// Pumps until `occurrences-count` stops answering the `-1` "still scanning" sentinel,
/// because a count read before the scan settles is not a count (see
/// `super::decode_occurrence_total`). A malformed pattern never settles — the engine
/// reports the error instead — so that case returns `Err` rather than spinning.
fn editor_count(hay: &str, query: &str, opts: FindOptions) -> Result<i32, String> {
    let buf = sourceview::Buffer::new(None);
    buf.set_text(hay);
    let settings = sourceview::SearchSettings::new();
    settings.set_case_sensitive(opts.case_sensitive);
    settings.set_at_word_boundaries(opts.whole_word);
    settings.set_regex_enabled(opts.regex);
    settings.set_search_text(Some(query));
    let sc = sourceview::SearchContext::new(&buf, Some(&settings));
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the editor search context to finish scanning the fixture",
        || sc.occurrences_count() >= 0 || sc.regex_error().is_some(),
    );
    match sc.regex_error() {
        Some(e) => Err(e.message().to_string()),
        None => Ok(sc.occurrences_count()),
    }
}

/// Count `query` in `hay` with the preview's matcher, under the same options.
fn preview_count(hay: &str, query: &str, opts: FindOptions) -> Result<i32, String> {
    match Matcher::compile(query, opts) {
        Ok(m) => Ok(m.count(hay) as i32),
        Err(e) => Err(e.message),
    }
}

/// Assert both engines answer the same thing for `query` over `hay`.
///
/// `why` names the property being measured, so a failure says which agreement broke
/// rather than only which numbers differ.
fn agree(hay: &str, query: &str, opts: FindOptions, why: &str) {
    let editor = editor_count(hay, query, opts);
    let preview = preview_count(hay, query, opts);
    match (&editor, &preview) {
        (Ok(a), Ok(b)) => assert_eq!(
            a, b,
            "{why}: the editor found {a} and the preview found {b} for {query:?} \
             under {opts:?} — the same query must mean the same thing in both panes"
        ),
        (Err(_), Err(_)) => (),
        _ => panic!(
            "{why}: one engine accepted {query:?} under {opts:?} and the other rejected it \
             (editor: {editor:?}, preview: {preview:?}) — a pattern accepted in one pane \
             must be accepted in the other"
        ),
    }
}

fn opts(case_sensitive: bool, whole_word: bool, regex: bool) -> FindOptions {
    FindOptions {
        case_sensitive,
        whole_word,
        regex,
    }
}

/// **The baseline.** A plain literal under each of the four case/word combinations must
/// count the same in both panes, or nothing measured below means anything.
#[gtktest::test]
fn a_literal_query_counts_the_same_in_both_panes() {
    const HAY: &str = "note Note NOTE notebook footnote note_2 note2 note-2\n";
    for case_sensitive in [false, true] {
        for whole_word in [false, true] {
            agree(
                HAY,
                "note",
                opts(case_sensitive, whole_word, false),
                "literal baseline",
            );
        }
    }
}

/// **Question 2 — which characters bound a word.** The matcher's literal whole-word
/// predicate is `is_alphanumeric() || '_'`; GtkSourceView's is the `gtk_text_iter` word
/// family. These four candidates are exactly where two such predicates part company:
/// `note_2` (underscore), `note2` (digit), `note-2` (hyphen) and a non-ASCII letter.
///
/// MEASURED: they agree. The matcher keeps its own predicate.
#[gtktest::test]
fn whole_word_draws_the_word_boundary_at_the_same_characters() {
    for hay in [
        "note_2 and more\n",
        "note2 and more\n",
        "note-2 and more\n",
        "notés and more\n",
        "über note über\n",
        "a note. note, note; note!\n",
    ] {
        agree(hay, "note", opts(false, true, false), "word boundary");
    }
}

/// **Question 1 — how whole word wraps a regular expression.** A bare `\bcat|dog\b`
/// binds each anchor to one branch of the alternation, so it would match `dogma`'s
/// `dog` while the non-capturing `\b(?:cat|dog)\b` would not. The two panes would then
/// disagree on any alternation the reader types with *whole word* ticked.
///
/// MEASURED: GtkSourceView wraps with the non-capturing group, and so does the matcher.
#[gtktest::test]
fn whole_word_wraps_an_alternation_as_one_group() {
    const HAY: &str = "cat dog concat dogma a cat, a dog.\n";
    agree(
        HAY,
        "cat|dog",
        opts(false, true, true),
        "alternation wrapping",
    );
    agree(
        HAY,
        "cat|dog",
        opts(false, false, true),
        "alternation, unwrapped",
    );
}

/// **Question 3 — compile flags.** The matcher compiles with MULTILINE, so `^` anchors
/// at every line start rather than only at the start of the text. If GtkSourceView did
/// not, `^` would mean two different things in the two panes — the most visible possible
/// divergence, since it changes the count on any multi-line document.
///
/// MEASURED: both anchor per line. `$` is measured with it, since the same flag governs
/// both.
#[gtktest::test]
fn an_anchor_means_the_same_thing_on_a_multi_line_document() {
    const HAY: &str = "alpha one\nbeta two\nalpha three\ngamma alpha\n";
    for query in ["^alpha", "alpha$", "^[ab][a-z]+", r"\w+$"] {
        agree(
            HAY,
            query,
            opts(false, false, true),
            "anchor under MULTILINE",
        );
    }
}

/// A character class, an anchor and a quantifier together — the shape rubric 11.14
/// names, and the one a reader actually types.
#[gtktest::test]
fn a_compound_pattern_counts_the_same_in_both_panes() {
    const HAY: &str = "item 1\nitem 22\nitem 333\nnot an item\nITEM 4\n";
    for query in [r"^item \d+", r"[0-9]{2,}", r"it.m", r"(?:item|thing)\s*\d*"] {
        for case_sensitive in [false, true] {
            agree(
                HAY,
                query,
                opts(case_sensitive, false, true),
                "compound pattern",
            );
        }
    }
}

/// **Question 4 — a pattern that can match nothing.** `a*` matches at every position,
/// so a naive "resume at the match end" loop never advances. The matcher has a progress
/// guard that truncates rather than hangs; the question was whether truncating puts it
/// out of step with the editor.
///
/// MEASURED: both engines report a match at every position, and they report the same
/// number of them — the guard never fires on a fixture this size, so it is a hang
/// stopper rather than a behaviour difference. Recorded here because "the preview is
/// wrong to truncate" was the live hypothesis, and it is not.
#[gtktest::test]
fn a_zero_width_pattern_does_not_diverge() {
    for hay in ["aaa\n", "abcabc\n", "\n"] {
        for query in ["a*", r"\b", "x?"] {
            agree(hay, query, opts(false, false, true), "zero-width pattern");
        }
    }
}

/// A malformed pattern is an ERROR in both panes, never a zero count. This is the
/// agreement rubric 11.15 rests on: the readout can only say "invalid pattern" for both
/// panes if both panes can say it.
#[gtktest::test]
fn a_malformed_pattern_is_rejected_by_both_engines() {
    for query in ["(unclosed", "a{2,1}", r"[z-a]", "*leading"] {
        let editor = editor_count("some text here\n", query, opts(false, false, true));
        let preview = preview_count("some text here\n", query, opts(false, false, true));
        assert!(
            editor.is_err(),
            "the editor accepted the malformed pattern {query:?}: {editor:?}"
        );
        assert!(
            preview.is_err(),
            "the preview accepted the malformed pattern {query:?}: {preview:?}"
        );
    }
}

/// An empty query matches nothing, in every mode. In regular-expression mode an empty
/// pattern would otherwise match at every position and report a count equal to the
/// document's length — "no query" is not "a query that matches everywhere".
#[gtktest::test]
fn an_empty_query_matches_nothing_in_either_pane() {
    for regex in [false, true] {
        let o = opts(false, false, regex);
        assert_eq!(preview_count("aaa bbb\n", "", o), Ok(0));
        assert_eq!(
            editor_count("aaa bbb\n", "", o),
            Ok(0),
            "the editor's engine treats an unset search text as no matches"
        );
    }
}
