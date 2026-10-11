//! TDD 3.2a (and 12.16) across a LIVE reload: the reader's place, and the outline row that
//! names it, both come back — in every view mode, with the outline filtered or not.
//!
//! Two defects answered this one report, one per kind of pane:
//!
//! * **Edit and Split** put the editor caret on the first line (`load_into_editor`) and
//!   left the editor's viewport at the top, and the outline follows the caret in those
//!   modes, so it moved to its top entry and stayed there.
//! * **Preview** restored the reader's line into the adjustment as its buffer y, which is
//!   one `top_margin` short of the value that shows it at the top
//!   (`ViewportTopIter::value_for_top_y`). A reader parked ON a heading — where any
//!   outline navigation leaves them — came back with the last line of the previous
//!   section at the top, and the scroll-spy moved the highlight back a section; under a
//!   filter hiding that section, to its nearest shown ancestor.
//!
//! The filter is not needed for either; it only changes which row the wrong section
//! falls back to. Cases run both ways regardless, because the report came from a
//! filtered outline.
//!
//! What is asserted is what the reader sees: the outline's selected row, the line at the
//! top of the reading pane, and in the editor modes the caret — never the restore's own
//! bookkeeping, which was right all along in the preview case.

use super::outline_nav::outline_tree_model;
use super::*;
use crate::outline_view::HeadingObject;
use crate::testpump::{drain_for, until, until_stable, Clock};
use std::time::Duration;

/// The heading the reader parks on, and the filter query that narrows the outline to it.
const TARGET: &str = "Target heading";
const QUERY: &str = "target";

/// Sixty sections of a few paragraphs each — far enough down that the reload's
/// progressive restore has heights to validate above the target — with the target
/// two-thirds of the way down, its predecessor's section filtered out by [`QUERY`].
fn document() -> String {
    let mut s = String::from("# Book\n\n");
    for chapter in 0..6 {
        s.push_str(&format!("## Chapter {chapter}\n\n"));
        for section in 0..10 {
            let title = if chapter == 4 && section == 3 {
                TARGET.to_owned()
            } else {
                format!("Section {chapter}.{section}")
            };
            s.push_str(&format!("### {title}\n\n"));
            for para in 0..4 {
                s.push_str(&format!(
                    "Paragraph {para} of {chapter}.{section}, long enough to wrap onto a \
                     second display row in a pane of ordinary width, so that line heights \
                     differ from a bare line count.\n\n"
                ));
            }
        }
    }
    s
}

fn target_line(doc: &str) -> i32 {
    doc.lines()
        .position(|l| l == format!("### {TARGET}"))
        .expect("the fixture carries the target heading") as i32
}

fn open(suffix: &str, mode: &str, doc: &str) -> ApplicationWindow {
    let app = crate::window::testkit::test_app_suffixed(suffix);
    let window = crate::window::new_window(&app, "IT", doc, None);
    window.set_default_size(1000, 700);
    window.present();
    until(
        Clock::Idle,
        "the window to map with its outline built",
        || window.is_mapped() && outline_tree_model(&window).is_some(),
    );
    window.change_action_state("view-mode", &mode.to_variant());
    drain_for(Clock::Frame, Duration::from_millis(300));
    assert_eq!(
        current_mode(&window).as_str(),
        mode,
        "precondition: the view mode took"
    );
    window
}

/// The title of the outline's selected row, if any.
fn selected_title(window: &ApplicationWindow) -> Option<String> {
    let st = state(window)?;
    super::sidebar::list_view_of(&st.chrome().outline_scroller)
        .and_then(|lv| lv.model())
        .and_then(|m| m.downcast::<gtk::SingleSelection>().ok())
        .and_then(|s| s.selected_item())
        .and_downcast::<gtk::TreeListRow>()
        .and_then(|r| r.item())
        .and_downcast::<HeadingObject>()
        .and_then(|h| h.path().last().map(|step| step.title.clone()))
}

fn filter_outline(window: &ApplicationWindow) {
    let st = state(window).expect("tab");
    let ch = st.chrome();
    window
        .lookup_action(SidebarPaneKind::Outline.filter_action())
        .expect("the filter command is registered")
        .activate(None);
    until(Clock::Idle, "the filter bar to open", || {
        ch.outline_filter.bar.is_search_mode()
    });
    ch.outline_filter.entry.set_text(QUERY);
    until(
        Clock::Frame,
        "the filter text to reach the document",
        || st.outline_filter.borrow().text == QUERY,
    );
}

/// The line at the top of the pane the reader is reading in this mode.
fn reading_pane_top(window: &ApplicationWindow) -> i32 {
    let st = state(window).expect("tab");
    if current_mode(window) == ViewMode::Preview {
        let view = st
            .split
            .preview_scroller()
            .and_then(|sw| sw.child())
            .and_then(|c| c.downcast::<crate::codeview::CodePreviewView>().ok())
            .expect("a preview view");
        crate::saferizer::viewport::ViewportTopIter::of(&view).line()
    } else {
        crate::saferizer::viewport::ViewportTopIter::of(&st.editor).line()
    }
}

fn caret_line(window: &ApplicationWindow) -> i32 {
    let st = state(window).expect("tab");
    st.editor_buf
        .iter_at_mark(&st.editor_buf.get_insert())
        .line()
}

/// Wait for the reload's restore and the scroll-spy to stop moving, and report where
/// they left the outline, the reading pane and the caret.
fn settled(window: &ApplicationWindow) -> (Option<String>, i32, i32) {
    let s = until_stable(
        Clock::Frame,
        Duration::from_secs(15),
        Duration::from_millis(800),
        || {
            (
                selected_title(window),
                reading_pane_top(window),
                caret_line(window),
            )
        },
    );
    assert!(s.converged, "the restore never settled");
    s.value.expect("sampled at least once")
}

/// Park the reader on the target the way the report did: the filter box's Enter goes to
/// the first match. Unfiltered, the filter is then closed — the place stays, the outline
/// is whole again.
fn park_in_preview(window: &ApplicationWindow, filtered: bool) {
    filter_outline(window);
    state(window)
        .expect("tab")
        .chrome()
        .outline_filter
        .entry
        .emit_by_name::<()>("activate", &[]);
    if !filtered {
        window
            .lookup_action(SidebarPaneKind::Outline.filter_action())
            .expect("the filter command is registered")
            .activate(None);
        until(Clock::Frame, "the filter to clear", || {
            state(window).is_some_and(|st| st.outline_filter.borrow().query().is_none())
        });
    }
}

/// The editor modes: the reader puts the caret in the target section and scrolls it
/// into view — the outline follows the caret here (TDD 12.14).
fn park_in_editor(window: &ApplicationWindow, doc: &str, filtered: bool) {
    if filtered {
        filter_outline(window);
    }
    let st = state(window).expect("tab");
    let it = st
        .editor_buf
        .iter_at_line(target_line(doc) + 2)
        .expect("a line inside the target section");
    st.editor_buf.place_cursor(&it);
    crate::farscroll::scroll_to_mark_when_ready(
        st.editor.upcast_ref(),
        &st.editor_buf.get_insert(),
        0.0,
        true,
        0.0,
        0.0,
    );
}

fn run(suffix: &str, mode: &str, filtered: bool) {
    let doc = document();
    let window = open(suffix, mode, &doc);
    if mode == "preview" {
        park_in_preview(&window, filtered);
    } else {
        park_in_editor(&window, &doc, filtered);
    }
    let (before_row, before_top, before_caret) = settled(&window);
    assert_eq!(
        before_row.as_deref(),
        Some(TARGET),
        "precondition: the outline reflects the reader's place before the reload"
    );
    assert!(
        before_top > 100,
        "precondition: the reader is far down the document (top line {before_top})"
    );
    assert_eq!(
        state(&window)
            .unwrap()
            .outline_filter
            .borrow()
            .query()
            .is_some(),
        filtered,
        "precondition: the filter is as this case needs it"
    );

    // Another process appends to the file; the monitor's decision core hands the new
    // text to this same entry point.
    super::reload::apply_external_reload(&window, &format!("{doc}\nappended by another process\n"));
    let (after_row, after_top, after_caret) = settled(&window);

    assert_eq!(
        after_row.as_deref(),
        Some(TARGET),
        "{mode} (filtered: {filtered}): the outline's selection comes back to the reader's \
         heading (pane top {before_top} → {after_top})"
    );
    assert_eq!(
        after_top, before_top,
        "{mode} (filtered: {filtered}): the reading pane comes back to the same top line"
    );
    if mode != "preview" {
        assert_eq!(
            after_caret, before_caret,
            "{mode} (filtered: {filtered}): the caret stays where the reader put it"
        );
    }
    window.destroy();
}

/// **3.2 / 12.16, Preview.** Mutation-checked: writing the line's y as the adjustment
/// value (dropping `value_for_top_y`'s margin) brings the top line back one short and
/// the selection back to the previous section — or, filtered, to its shown ancestor.
#[gtktest::test]
fn a_live_reload_keeps_the_outline_on_the_readers_heading_in_preview() {
    run("outline_reload.preview", "preview", false);
}

#[gtktest::test]
fn a_live_reload_keeps_the_filtered_outline_on_the_readers_heading_in_preview() {
    run("outline_reload.preview_filtered", "preview", true);
}

/// **3.2 / 12.16, Edit and Split.** Mutation-checked: without `restore_editor_place` the
/// caret comes back on the first line, the editor at the top, and the selection on the
/// document's first heading.
#[gtktest::test]
fn a_live_reload_keeps_the_caret_and_the_outline_in_edit() {
    run("outline_reload.edit", "edit", false);
}

#[gtktest::test]
fn a_live_reload_keeps_the_caret_and_the_outline_in_split() {
    run("outline_reload.split", "split", false);
}

#[gtktest::test]
fn a_live_reload_keeps_the_caret_and_the_filtered_outline_in_split() {
    run("outline_reload.split_filtered", "split", true);
}

/// **3.2, Preview, a reader who scrolled by hand.** The reload restores the line the
/// view TRACKED while the reader scrolled, so that line must be the one at the top
/// EDGE — the space the restore writes back into. The reader is left a few pixels short
/// of a heading, with the last line of the previous section at the top.
///
/// Mutation-checked: tracking `line_at_y(value)` (a margin below the edge) records the
/// heading instead, and the reload brings the heading to the top — one line on, and the
/// outline a section on.
#[gtktest::test]
fn a_live_reload_returns_a_hand_scrolled_preview_to_the_same_line() {
    use gtk::subclass::prelude::*;
    let doc = document();
    let window = open("outline_reload.preview_hand", "preview", &doc);
    park_in_preview(&window, false);
    settled(&window);
    let view = state(&window)
        .unwrap()
        .split
        .preview_scroller()
        .and_then(|sw| sw.child())
        .and_then(|c| c.downcast::<crate::codeview::CodePreviewView>().ok())
        .expect("a preview view");
    let vadj = view.vadjustment().expect("the view is in a scroller");
    // Scroll the way a reader does — `user_scrolling` is what every hand-scroll input
    // source sets — to just above the heading the navigation parked at the top.
    view.imp().user_scrolling.set(true);
    crate::saferizer::scrollpos::jump(&vadj, vadj.value() - 4.0);
    let (before_row, before_top, _) = settled(&window);
    assert_ne!(
        before_row.as_deref(),
        Some(TARGET),
        "precondition: the line at the top is the previous section's"
    );

    super::reload::apply_external_reload(&window, &format!("{doc}\nappended by another process\n"));
    let (after_row, after_top, _) = settled(&window);
    assert_eq!(
        after_top, before_top,
        "the hand-scrolled reading position comes back to the same top line"
    );
    assert_eq!(after_row, before_row, "and the outline with it");
    window.destroy();
}
