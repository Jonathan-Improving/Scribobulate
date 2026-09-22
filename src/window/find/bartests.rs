//! The find bar driven as a reader drives it, for the match options.
//!
//! `parity` proves the two ENGINES agree over raw text. This file proves the two PANES
//! agree in a running window — which is a different claim, because between a reader
//! ticking a toggle and a number appearing sit the action, the tab's own state, the
//! editor's `SearchSettings`, the preview's hit cache and the readout. Every one of
//! those is a place where an option can reach one pane and not the other, and the shape
//! that failure takes on screen is a count that changes when you switch view mode.
//!
//! Its own module rather than more cases inside `window/find.rs`'s test block: these
//! drive the BAR, not the search, and the file they would otherwise join is already the
//! largest in the tree.

use super::super::testkit::test_app;
use crate::winstate::state;
use gtk::prelude::*;
use gtk::ApplicationWindow;

/// Four occurrences of the term in body text, three in a table cell and three inside a
/// collapsed disclosure — the preview's three separate search paths, each carrying the
/// same mixture of cases and one `notebook` that only a whole-word search rejects.
///
/// The counts the tests assert are derived from this fixture and are written out rather
/// than computed, because a count computed by the same matcher under test would agree
/// with it however wrong both were.
const MD: &str = "\
note Note NOTE notebook

| Head |
|---|
| note Note notebook |

<details>
<summary>Folded</summary>

note Note notebook

</details>
";

/// Every occurrence of `note`, case-insensitively and not whole-word:
/// 4 in the body, 3 in the cell, 3 in the collapsed body.
const ALL: i32 = 10;
/// Case-sensitive: the lowercase `note` and the `note` inside `notebook`, per source.
const CASE_EXACT: i32 = 6;
/// Whole word: everything except the three `notebook`s.
const WHOLE: i32 = 7;

/// Open the find bar and put `query` in it, for the pane the window is currently
/// showing.
fn search(win: &ApplicationWindow, query: &str) {
    crate::window::actions::simple_action(win, "find")
        .expect("win.find is registered")
        .activate(None);
    let st = state(win).expect("the window has an active tab");
    st.chrome().find_entry.set_text(query);
}

/// The readout's text once the search it describes has actually happened.
///
/// **Two waits, for two different delays, and neither is optional.**
/// `GtkSearchEntry` does not emit `search-changed` on the keystroke — it debounces,
/// so a readout read immediately after `set_text` is the PREVIOUS query's answer and
/// every assertion below would be measuring the wrong search. Then the editor's engine
/// answers its `-1` sentinel until it has swept the buffer, which is a second wait with
/// its own end condition. The preview's path has neither delay and simply passes
/// through both.
fn readout(win: &ApplicationWindow) -> String {
    // Past `GtkSearchEntry`'s own debounce. Generous: this is a wait for a fixed
    // toolkit delay, not a poll for a condition, so there is nothing to test for.
    crate::testpump::drain_for(
        crate::testpump::Clock::Idle,
        std::time::Duration::from_millis(400),
    );
    let st = state(win).expect("the window has an active tab");
    let label = st.chrome().match_count_label.clone();
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the find readout to stop reporting a scan in progress",
        || label.text() != "…",
    );
    label.text().to_string()
}

/// The readout's count, whatever wording it is wrapped in. `None` when the readout is
/// not reporting a count at all.
fn count(win: &ApplicationWindow) -> Option<i32> {
    let text = readout(win);
    if text == "No matches" {
        return Some(0);
    }
    text.split_whitespace()
        .next_back()
        .and_then(|n| n.parse().ok())
        .or_else(|| text.split_whitespace().next().and_then(|n| n.parse().ok()))
}

fn set_option(win: &ApplicationWindow, name: &str, on: bool) {
    crate::window::actions::change_action_state(win, name, &on.to_variant());
}

fn set_mode(win: &ApplicationWindow, mode: &str) {
    win.change_action_state("view-mode", &mode.to_variant());
    crate::testpump::drain_for(
        crate::testpump::Clock::Frame,
        std::time::Duration::from_millis(200),
    );
}

/// **The claim, end to end: an option means the same thing in whichever pane is
/// visible** (TDD 11.13).
///
/// The fixture puts matches in all three of the preview's search paths — body text, a
/// table cell's `GtkLabel`, and a collapsed disclosure that this render did not draw —
/// because those used to be three hard-wired matchers. An option reaching one of them
/// and not the others is a count that changes when the reader switches view mode, and
/// it is invisible on a fixture whose matches are all in one place.
///
/// Mutation check: drop the `set_case_sensitive` line from
/// `findbar::push_options_to_engine` and the editor legs fail while every preview leg
/// stays green — which is exactly the half-wired shape this exists to catch.
#[gtktest::test]
fn a_match_option_counts_the_same_in_either_pane() {
    let app = test_app("com.extollit.scribobulate.integrationtest.findoptions");
    let win = crate::window::new_window(&app, "IT-findoptions", MD, None);

    for mode in ["edit", "preview"] {
        set_mode(&win, mode);
        search(&win, "note");
        assert_eq!(
            count(&win),
            Some(ALL),
            "{mode}: the default is the case-insensitive literal the bar has always done"
        );

        set_option(&win, "find-match-case", true);
        assert_eq!(
            count(&win),
            Some(CASE_EXACT),
            "{mode}: match case must drop the capitalised occurrences"
        );
        set_option(&win, "find-match-case", false);

        set_option(&win, "find-whole-word", true);
        assert_eq!(
            count(&win),
            Some(WHOLE),
            "{mode}: whole word must reject the occurrences inside `notebook`"
        );
        set_option(&win, "find-whole-word", false);

        assert_eq!(
            count(&win),
            Some(ALL),
            "{mode}: unticking an option must restore the count it changed"
        );
    }
    win.destroy();
}

/// A regular expression reaches both panes, and reaches the two preview paths that are
/// not the buffer (TDD 11.14).
///
/// `^note` is chosen deliberately: an anchor is where a compile-flag difference between
/// the engines becomes visible, and it cannot match inside `notebook` either, so the
/// count says something about the pattern rather than only about the option being read.
#[gtktest::test]
fn a_regular_expression_reaches_both_panes() {
    let app = test_app("com.extollit.scribobulate.integrationtest.findregex");
    let win = crate::window::new_window(&app, "IT-findregex", MD, None);

    let mut counts = Vec::new();
    for mode in ["edit", "preview"] {
        set_mode(&win, mode);
        search(&win, "note");
        set_option(&win, "find-regex", true);
        let st = state(&win).expect("a tab");
        st.chrome().find_entry.set_text(r"note\w*");
        let wildcard = count(&win);
        st.chrome().find_entry.set_text("notebook|NOTE");
        let alternation = count(&win);
        counts.push((wildcard, alternation));
        set_option(&win, "find-regex", false);
    }
    let editor = counts[0];
    let preview = counts[1];
    assert_eq!(
        editor, preview,
        "the same pattern counted differently in the two panes ({editor:?} vs {preview:?}) \
         — they must compile it with the same engine"
    );
    assert_eq!(
        editor.0,
        Some(ALL),
        "`note\\w*` must match every occurrence, `notebook` included"
    );
    assert!(
        editor.1.is_some_and(|n| n > 0),
        "an alternation must match something; it counted {:?}",
        editor.1
    );
    win.destroy();
}

/// **A pattern that does not compile is reported, not answered** (TDD 11.15).
///
/// Both panes, because both have their own way of failing to compile — the editor's
/// engine reports a `regex-error` and the preview's matcher returns one — and a readout
/// that only knows about one of them is silently wrong in the other pane.
///
/// Next/Prev doing nothing is asserted with the rest: there is no list to step, and
/// stepping the previous pattern's list would move the reader through matches that are
/// not what the readout describes.
#[gtktest::test]
fn a_malformed_pattern_is_reported_rather_than_counted_as_zero() {
    let app = test_app("com.extollit.scribobulate.integrationtest.findbadregex");
    let win = crate::window::new_window(&app, "IT-findbadregex", MD, None);

    for mode in ["edit", "preview"] {
        set_mode(&win, mode);
        search(&win, "note");
        set_option(&win, "find-regex", true);
        let st = state(&win).expect("a tab");
        st.chrome().find_entry.set_text("(unclosed");
        assert_eq!(
            readout(&win),
            "Invalid pattern",
            "{mode}: a malformed pattern must say so — `No matches` reports on the \
             document when what happened is that nobody was able to ask"
        );
        assert!(
            st.chrome().match_count_label.tooltip_text().is_some(),
            "{mode}: the engine's own diagnostic must be reachable — a fixed \
             `Invalid pattern` with nothing behind it leaves the reader with no way to \
             find out WHICH construct it objected to"
        );

        st.find_cursor.set(crate::window::FindCursor::None);
        // What the Next chevron, Enter in the field and Edit ▸ Find Next all call.
        super::find_step(&win, &st.search_context, super::SearchDir::Forward);
        assert_eq!(
            readout(&win),
            "Invalid pattern",
            "{mode}: Next must do nothing while the pattern does not compile"
        );
        assert_eq!(
            st.find_cursor.get(),
            crate::window::FindCursor::None,
            "{mode}: Next must not land on a match of the PREVIOUS pattern"
        );

        // Finishing the pattern recovers in place — no closing and reopening the bar.
        st.chrome().find_entry.set_text("(unclosed)?note");
        assert_eq!(
            count(&win),
            Some(ALL),
            "{mode}: completing the pattern must recover without reopening the bar"
        );
        set_option(&win, "find-regex", false);
    }
    win.destroy();
}

/// A tab's options travel with it, and every control that shows them follows (TDD
/// 15.12).
///
/// The two halves are asserted together on purpose: the behaviour reverting while the
/// toggles stay put, and the toggles reverting while the behaviour stays put, are both
/// real and each looks fine from the other's vantage point. This is the
/// `show-unsafe-images` lying-mirror defect's shape, in a second place.
#[gtktest::test]
fn a_tab_keeps_its_own_match_options() {
    let app = test_app("com.extollit.scribobulate.integrationtest.findtabopts");
    let win = crate::window::new_window(&app, "IT-findtabopts", MD, None);
    set_mode(&win, "preview");
    search(&win, "note");
    set_option(&win, "find-match-case", true);
    let first = state(&win).expect("a tab").id;
    // Read the count here for its WAIT, not for its value: `GtkSearchEntry` debounces
    // `search-changed`, and that handler is what records the query against this tab. A
    // tab switched away from before it has fired keeps an empty query and comes back
    // with nothing to count, which reads as "the options were lost" and is not.
    assert_eq!(count(&win), Some(CASE_EXACT));

    let second = crate::window::create_tab_in_window(&win, MD, None, false, false)
        .expect("the second tab is created");
    // The query is per tab too (§15.12), so the new tab starts with an empty field and
    // has to be given the same term before its COUNT says anything about its options.
    search(&win, "note");
    assert_eq!(
        count(&win),
        Some(ALL),
        "a fresh tab starts at the default options, not the previous tab's"
    );
    assert!(
        !option_state(&win, "find-match-case"),
        "the toggle must follow the newly active tab, not keep the other tab's tick"
    );

    crate::window::actions::change_action_state(
        &win,
        "select-tab",
        &first.to_string().to_variant(),
    );
    crate::testpump::drain_for(
        crate::testpump::Clock::Frame,
        std::time::Duration::from_millis(200),
    );
    assert_eq!(
        count(&win),
        Some(CASE_EXACT),
        "switching back must restore the tab's own options, not only its query"
    );
    assert!(
        option_state(&win, "find-match-case"),
        "…and the control that shows them must say so too"
    );
    let _ = second;
    win.destroy();
}

/// The current state of one option `GAction`, read through the action map the toggle
/// button and the Edit-menu item both read.
fn option_state(window: &ApplicationWindow, name: &str) -> bool {
    window
        .lookup_action(name)
        .and_then(|a| a.state())
        .and_then(|v| v.get::<bool>())
        .expect("the option action is registered and stateful")
}

/// Select `[start, end)` characters of whichever pane the window is showing.
fn select_range(win: &ApplicationWindow, start: i32, end: i32) {
    let st = state(win).expect("a tab");
    let buf: gtk::TextBuffer = match super::find_target(win) {
        super::FindTarget::Preview(view) => view.buffer(),
        _ => st.editor_buf.clone().upcast(),
    };
    buf.select_range(&buf.iter_at_offset(start), &buf.iter_at_offset(end));
    crate::testpump::drain_for(
        crate::testpump::Clock::Idle,
        std::time::Duration::from_millis(100),
    );
}

fn action_enabled(window: &ApplicationWindow, name: &str) -> bool {
    window
        .lookup_action(name)
        .expect("the action is registered")
        .is_enabled()
}

/// **Search in selection confines the search in the editor, and Replace All with it**
/// (TDD 11.16, 11.17).
///
/// The scope is a pair of marks precisely so it survives the replacements: the
/// assertion that it still covers the same passage AFTERWARDS is the one a pair of
/// offsets fails, and it fails silently — the first Replace All looks right and the
/// second one reaches text outside the selection.
#[gtktest::test]
fn search_in_selection_confines_the_editor_search_and_its_replacements() {
    const DOC: &str = "one target\ntwo target\nthree target\nfour target\n";
    let app = test_app("com.extollit.scribobulate.integrationtest.findscope");
    let win = crate::window::new_window(&app, "IT-findscope", DOC, None);
    set_mode(&win, "edit");
    search(&win, "target");
    assert_eq!(count(&win), Some(4), "unscoped, every occurrence counts");

    // The first two lines only: "one target\ntwo target\n" is characters 0..22.
    let bound_end = DOC.find("three").expect("the fixture has a third line") as i32;
    select_range(&win, 0, bound_end);
    assert!(
        action_enabled(&win, super::super::findbar::FIND_IN_SELECTION),
        "a live selection must make the control usable"
    );
    set_option(&win, super::super::findbar::FIND_IN_SELECTION, true);
    assert_eq!(
        count(&win),
        Some(2),
        "the count must describe only the occurrences inside the selected passage"
    );

    // Stepping wraps WITHIN the passage rather than around the document.
    let st = state(&win).expect("a tab");
    for expected in [1, 2, 1, 2] {
        super::find_step(&win, &st.search_context, super::SearchDir::Forward);
        assert_eq!(
            st.find_cursor.get(),
            crate::window::FindCursor::Editor(expected),
            "stepping must wrap inside the passage, not out of it"
        );
    }

    super::replace_all_matches(&win, &st, "TOKEN");
    let after = crate::saferizer::BufferText::of(&st.editor_buf).into_string();
    assert_eq!(
        after, "one TOKEN\ntwo TOKEN\nthree target\nfour target\n",
        "Replace All must reach only the occurrences the SEARCH covered — not because \
         Replace All is scoped, but because the search it acts on is"
    );
    assert_eq!(
        st.chrome().match_count_label.text().as_str(),
        "2 replaced",
        "Replace All reports how many it made, not how many are left"
    );

    // The passage still covers the same text even though the replacements changed its
    // length — the marks moved with it. Searching the new word finds both, and only
    // both.
    st.chrome().find_entry.set_text("TOKEN");
    assert_eq!(
        count(&win),
        Some(2),
        "the scope must still cover the same passage after its length changed"
    );
    win.destroy();
}

/// Replace acts on the match the reader is LOOKING at (TDD 11.17; ScrAP-27).
///
/// The fixture puts the caret past a match on purpose: re-finding forward from the
/// caret and replacing what it lands on is right only while the caret happens to sit on
/// the highlighted match, and the whole point is the case where it does not.
#[gtktest::test]
fn replace_acts_on_the_current_match_and_then_advances() {
    const DOC: &str = "aaa target bbb target ccc target\n";
    let app = test_app("com.extollit.scribobulate.integrationtest.findreplacecur");
    let win = crate::window::new_window(&app, "IT-findreplacecur", DOC, None);
    set_mode(&win, "edit");
    search(&win, "target");
    // Settle first: the editor's engine answers its scanning sentinel until it has
    // swept the buffer, and a step taken before then lands on the right match while
    // honestly reporting no position for it (§11.3). This test is about WHICH match is
    // replaced, so it waits rather than measuring through that state.
    assert_eq!(count(&win), Some(3));
    let st = state(&win).expect("a tab");

    // Step onto the SECOND match, so "the current match" and "the first match after the
    // start of the document" are different answers.
    super::find_step(&win, &st.search_context, super::SearchDir::Forward);
    super::find_step(&win, &st.search_context, super::SearchDir::Forward);
    assert_eq!(st.find_cursor.get(), crate::window::FindCursor::Editor(2));

    super::replace_current_match(&win, &st, "DONE");
    let after = crate::saferizer::BufferText::of(&st.editor_buf).into_string();
    assert_eq!(
        after, "aaa target bbb DONE ccc target\n",
        "the SECOND occurrence — the one that was highlighted — must be the one replaced"
    );
    // …and the selection advanced to the next one rather than staying put.
    let (sel_start, sel_end) = st
        .editor_buf
        .selection_bounds()
        .expect("Replace advances to the next match and selects it");
    assert_eq!(
        crate::saferizer::BufferText::of_range(&st.editor_buf, &sel_start, &sel_end).as_str(),
        "target",
        "Replace must advance to the next match"
    );
    assert!(
        sel_start.offset() > "aaa target bbb DONE".len() as i32,
        "…the next one FORWARD, not back to the first"
    );
    win.destroy();
}

/// **A preview scope that no longer resolves makes the holder re-derive** (TDD 11.16).
///
/// The toggle turns itself off and the search covers the whole pane. Reinterpreting the
/// range against the new render would confine the search to whatever now happens to sit
/// at those offsets, which is a confident answer about a passage the reader never chose
/// — the arm the retired `fold_epoch` got wrong and `PreviewFindCache` got right.
#[gtktest::test]
fn a_preview_scope_that_no_longer_resolves_turns_itself_off() {
    let app = test_app("com.extollit.scribobulate.integrationtest.findscopepreview");
    let win = crate::window::new_window(&app, "IT-findscopepreview", MD, None);
    set_mode(&win, "preview");
    search(&win, "note");
    assert_eq!(count(&win), Some(ALL));

    // The first line of the rendered preview only.
    select_range(&win, 0, 23);
    set_option(&win, super::super::findbar::FIND_IN_SELECTION, true);
    let confined = count(&win);
    assert!(
        confined.is_some_and(|n| n > 0 && n < ALL),
        "the selection must confine the count to part of the pane, got {confined:?}"
    );
    assert!(option_state(&win, super::super::findbar::FIND_IN_SELECTION));

    // A theme switch re-renders the preview beneath the bound.
    crate::app::re_render_all_windows(&app);
    crate::testpump::drain_for(
        crate::testpump::Clock::Frame,
        std::time::Duration::from_millis(400),
    );
    assert_eq!(
        count(&win),
        Some(ALL),
        "an unresolvable bound must make the search cover the whole pane again"
    );
    assert!(
        !option_state(&win, super::super::findbar::FIND_IN_SELECTION),
        "…and the toggle must say so, rather than claiming a confinement that is gone"
    );
    assert!(
        state(&win).expect("a tab").find_scope.borrow().is_none(),
        "the bound itself must be released, not merely ignored"
    );
    win.destroy();
}

/// With nothing selected and nothing captured, the control is unavailable and says why
/// (TDD 11.16).
#[gtktest::test]
fn the_in_selection_control_is_unavailable_with_nothing_to_confine() {
    let app = test_app("com.extollit.scribobulate.integrationtest.findscopesens");
    let win = crate::window::new_window(&app, "IT-findscopesens", MD, None);
    set_mode(&win, "edit");
    search(&win, "note");
    let name = super::super::findbar::FIND_IN_SELECTION;
    assert!(
        !action_enabled(&win, name),
        "nothing is selected, so there is nothing to confine the search to"
    );
    select_range(&win, 0, 10);
    assert!(action_enabled(&win, name), "a selection makes it usable");
    set_option(&win, name, true);
    // Collapsing the selection must NOT take the control away: a captured bound has to
    // stay releasable, or the reader is stuck inside it.
    select_range(&win, 5, 5);
    assert!(
        action_enabled(&win, name),
        "a captured bound must stay releasable after the selection that made it is gone"
    );
    win.destroy();
}

/// **The find bar's boundary refresh describes the pane in front of the reader, not
/// only the preview** (TDD 11.13).
///
/// `refresh_preview_find_highlight` is what every boundary that rebuilds a pane calls —
/// a mode switch, a theme re-render, an external reload. It used to act on the Preview
/// arm alone, on the reasoning that only a rebuilt preview loses anything. True of the
/// highlights; false of the READOUT, which was left showing the count of the pane the
/// reader had just left. The two panes legitimately count differently — the preview
/// searches three texts, so an anchored pattern matches inside each — so the stale
/// number was true of neither the query nor the pane.
///
/// Reported by the macOS seat, ratifying batch A.
///
/// **The assertion is made against the refresh ITSELF, not against a live mode
/// switch**, and the difference is the whole reliability of this guard. A mode switch
/// has side effects that also happen to recount — the editor takes focus, the caret
/// moves, and its engine re-emits `occurrences-count` — so a test that switches modes
/// and then reads the label is measuring whichever of several paths got there first,
/// and may pass on a build where this refresh does nothing at all. Poisoning the label
/// and invoking the boundary refresh asks the one question that matters: does the call
/// every pane-rebuilding boundary makes answer for the pane the reader can see?
///
/// Mutation check: narrow `findbar::refresh_preview_find_highlight` back to its
/// `if let FindTarget::Preview(..)` arm → the poison survives in edit mode
/// (`"999 matches"` against an owed `"2 matches"`). VERIFIED, and worth saying how it
/// was nearly not: the first attempt at this check edited a pattern that did not exist
/// in the file, so it mutated nothing and the test "passed" — a green mutation run is
/// indistinguishable from a mutation that never happened unless the edit asserts it
/// landed.
#[gtktest::test]
fn the_boundary_refresh_recounts_for_whichever_pane_is_visible() {
    let app = test_app("com.extollit.scribobulate.integrationtest.findmodereadout");
    let win = crate::window::new_window(&app, "IT-findmodereadout", MD, None);

    set_mode(&win, "edit");
    search(&win, "note");
    set_option(&win, "find-regex", true);
    let st = state(&win).expect("a tab");
    st.chrome().find_entry.set_text("^note");
    let in_editor = count(&win).expect("the editor counts the anchored pattern");

    set_mode(&win, "preview");
    let in_preview = count(&win).expect("the preview counts it too");
    assert_ne!(
        in_editor, in_preview,
        "precondition: the fixture must make the two panes disagree, or a stale readout \
         is indistinguishable from a fresh one"
    );

    let label = st.chrome().match_count_label.clone();
    for (mode, owed) in [("preview", in_preview), ("edit", in_editor)] {
        set_mode(&win, mode);
        // Whatever the switch itself did, put a number on screen that is true of
        // neither pane, then ask the boundary refresh to answer.
        label.set_text("999 matches");
        crate::window::refresh_preview_find_highlight(&win);
        assert_eq!(
            label.text().as_str(),
            format!("{owed} matches"),
            "in {mode} the boundary refresh must recount FOR {mode} — it is the one \
             call every pane-rebuilding boundary makes, so an arm it does not cover is \
             a readout nothing corrects"
        );
    }
    win.destroy();
}

/// **Whole word bounds an alternation as one group, in the editor too** (TDD 11.14).
///
/// `notebook` starts with one branch and ends with the other, so it matches an
/// ungrouped `\b…\b` wrapper and not a grouped one. GtkSourceView's own wrapper is
/// ungrouped, so this passes only because the application wraps the pattern before
/// handing it over — which is the whole of `matcher::editor_pattern`.
///
/// Driven through the find bar rather than through two engines directly, because the
/// wrapping happens in `findbar::refresh_find` and a probe that set the raw query on a
/// `SearchSettings` of its own would measure an engine the application never drives.
#[gtktest::test]
fn whole_word_bounds_an_alternation_the_same_way_in_both_panes() {
    const SPANNING: &str = "note notebook book\n";
    let app = test_app("com.extollit.scribobulate.integrationtest.findwordalt");
    let win = crate::window::new_window(&app, "IT-findwordalt", SPANNING, None);
    for mode in ["edit", "preview"] {
        set_mode(&win, mode);
        search(&win, "note|book");
        set_option(&win, "find-regex", true);
        set_option(&win, "find-whole-word", true);
        assert_eq!(
            count(&win),
            Some(2),
            "{mode}: `notebook` is neither whole word, so only the two standalone \
             words match — an ungrouped wrapper counts it as 4 here"
        );
        set_option(&win, "find-whole-word", false);
        set_option(&win, "find-regex", false);
    }
    win.destroy();
}
