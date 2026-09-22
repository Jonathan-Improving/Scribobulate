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
