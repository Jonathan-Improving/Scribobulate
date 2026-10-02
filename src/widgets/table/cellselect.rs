//! **Mouse selection in a table cell** — a drag always selects, it never drags text.
//!
//! A cell is a selectable `GtkLabel`, and a selectable label has a built-in text
//! drag-and-drop: a press that lands **inside the label's current selection** arms it
//! (`gtk_label_drag_gesture_begin` sets `in_drag`, GTK 4.6.9 `gtklabel.c:4491-4497`),
//! and the first motion past the drag threshold hands the selected text to
//! `gdk_drag_begin` instead of moving the selection (`:4524-4546`). Nothing in this
//! application consumes that drop, so to a reader it is a swipe that does nothing: they
//! cannot re-select part of a cell they have already selected. It also swallows the
//! follow-through of a double- or triple-click, because the word or cell those presses
//! select always contains the pointer, so a hand that moves while the last press is
//! still down starts a drag. GTK offers no switch for it.
//!
//! Two gestures of our own on the label repair it. They take a press over from the
//! label exactly when the label would have armed its drag-and-drop: a single press
//! **inside** the current selection, or any double or triple press (whose word or cell
//! always contains the pointer). Once the pointer passes the drag threshold — the one
//! GTK arms its own drag with — the label's drag gesture is DENIED, before its own
//! handler sees that motion, and the selection is redrawn from the press point:
//! characters for a single press, whole words for a double (GTK's label never extended
//! a double press by words at all — its word branch sits behind the drag-and-drop one,
//! `:4524`, `:4556`), the whole cell for a triple.
//!
//! Every other press is left to GTK, and so is every press that never reaches the
//! threshold: a click inside a selection collapses it on release (`:4394-4398`) and does
//! NOT follow a link it lands on, because the label found no active link under a
//! selection (`gtk_label_update_active_link`) — which is also why its cursor is an
//! I-beam there. An earlier version cleared the selection at the press instead; that
//! made a click on a selected link follow it under an I-beam cursor.
//!
//! Both are in the label's BUBBLE phase beside its own gestures, and run before them
//! because a widget's controllers run newest first (`gtk_widget_add_controller`
//! prepends — GTK4Rs/AP-302). They are grouped with the label's own click gesture: the
//! label CLAIMS every primary press, and a claim DENIES every other gesture on the widget
//! outside the claimant's group (`_gtk_widget_set_sequence_state_internal`,
//! `gtkwidget.c:2244-2252`), so ungrouped they would hear the press and nothing after it.
//! Nothing here claims or consumes an event, so the label's own gestures still see every
//! one. A Shift-press is left to GTK: it extends the selection and never arms the drag.

use gtk::prelude::*;
use gtk::{gdk, glib, pango, EventSequenceState, GestureClick, GestureDrag, Label};
use std::cell::Cell;
use std::rc::Rc;

/// Make `label` a selectable table cell whose mouse drags select rather than drag.
///
/// The one way a cell label becomes selectable: a cell made selectable any other way
/// would carry GTK's drag-and-drop back with it.
pub(crate) fn make_cell_selectable(label: &Label) {
    label.set_selectable(true);

    // The label's own gestures, which `set_selectable` has just created. Found by type
    // because GTK exposes them no other way; a label that ever stops carrying them has
    // no drag-and-drop to suppress either, so the cell degrades to GTK's behaviour.
    let (Some(own_click), Some(own_drag)) = (
        own_gesture::<GestureClick>(label),
        own_gesture::<GestureDrag>(label),
    ) else {
        log::error!(
            "table cell: GtkLabel carries no click/drag gesture of its own, so its text \
             drag-and-drop cannot be suppressed"
        );
        return;
    };

    // Press count of the press in progress, read by the drag handler.
    let presses = Rc::new(Cell::new(1));
    // Whether this press is one the label would drag out (so ours to take over), and
    // whether its drag has been taken yet.
    let armed = Rc::new(Cell::new(false));
    let taken = Rc::new(Cell::new(false));

    let click = GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    click.connect_pressed(glib::clone!(
        #[weak]
        label,
        #[strong]
        presses,
        #[strong]
        armed,
        #[strong]
        taken,
        move |gesture, n_press, x, y| {
            let shift = gesture
                .current_event_state()
                .contains(gdk::ModifierType::SHIFT_MASK);
            let (ox, oy) = label.layout_offsets();
            let at = char_at(&label.layout(), ox, oy, x, y);
            let inside = press_inside_selection(label.selection_bounds(), at);
            presses.set(n_press);
            armed.set(takes_over(n_press, shift, inside));
            taken.set(false);
        }
    ));
    label.add_controller(click.clone());

    let drag = GestureDrag::new();
    drag.set_button(gdk::BUTTON_PRIMARY);
    drag.connect_drag_update(glib::clone!(
        #[weak]
        label,
        #[weak]
        own_drag,
        #[weak]
        own_click,
        #[strong]
        presses,
        #[strong]
        armed,
        #[strong]
        taken,
        move |gesture, dx, dy| {
            let Some((sx, sy)) = gesture.start_point() else {
                return;
            };
            let (px, py) = (sx + dx, sy + dy);
            let step = drag_step(
                armed.get(),
                taken.get(),
                label.drag_check_threshold(sx as i32, sy as i32, px as i32, py as i32),
                px >= 0.0
                    && py >= 0.0
                    && px < f64::from(label.width())
                    && py < f64::from(label.height()),
            );
            if step == DragStep::Ignore {
                return;
            }
            if step != DragStep::Extend {
                // Deny the label's drag on every armed update short of a real in-label
                // drag. From GTK 4.8 a double/triple press does not set the label's
                // `in_drag`, and any update whose index differs from its
                // `selection_anchor` CLAIMS (4.22.4 `gtklabel.c:4822`), cancelling the
                // click group and zeroing its press count. GTK delivers such an update
                // with no pointer motion: the release itself (`gtkgesture.c:675-687`),
                // or a synthesized `gdk_surface_ensure_motion` on a frame-clock flush.
                // So a 120 ms triple read its third press as a single (macOS seat,
                // 4.22.4/Quartz). A point outside the label (a parked anchor) is
                // denied too, so it never takes the `reset()` below.
                own_drag.set_state(EventSequenceState::Denied);
                if step != DragStep::TakeOver {
                    return;
                }
                // And its click gesture, so its release handler never runs: on a single
                // press it would collapse the selection to the release point (`:4394`),
                // undoing the one drawn here. GTK resets the same gesture itself after a
                // triple press (`:4368-4369`).
                own_click.reset();
                taken.set(true);
            }
            let layout = label.layout();
            let (ox, oy) = label.layout_offsets();
            let start = char_at(&layout, ox, oy, sx, sy);
            let current = char_at(&layout, ox, oy, sx + dx, sy + dy);
            let words = word_edges(&layout.text());
            let (anchor, end) = drag_selection(presses.get(), start, current, &words);
            label.select_region(anchor, end);
        }
    ));
    label.add_controller(drag.clone());

    click.group_with(&own_click);
    drag.group_with(&own_click);
}

/// Whether a press at character `at` falls inside the label's current selection
/// `bounds` (either order), inclusive at both ends.
fn press_inside_selection(bounds: Option<(i32, i32)>, at: usize) -> bool {
    bounds.is_some_and(|(a, b)| {
        let (lo, hi) = (a.min(b), a.max(b));
        usize::try_from(lo).is_ok_and(|lo| lo <= at) && usize::try_from(hi).is_ok_and(|hi| at <= hi)
    })
}

/// What one `drag-update` on an armed press does to GTK's gestures and the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragStep {
    /// The press was not one this module takes over; GTK's label handles it.
    Ignore,
    /// Short of the drag threshold, or outside the label: deny the label's own drag and
    /// nothing else. From GTK 4.8 such an update would otherwise CLAIM and zero the
    /// click group's press count, so a quick triple read as a single.
    DenyOnly,
    /// The first update past the threshold inside the label: deny its drag, reset its
    /// click gesture (whose release would collapse the selection), and take over.
    TakeOver,
    /// Already taken over: extend the selection to the pointer.
    Extend,
}

/// The decision [`DragStep`] names, from the four facts the update handler reads.
fn drag_step(armed: bool, taken: bool, past_threshold: bool, inside: bool) -> DragStep {
    match (armed, taken, past_threshold && inside) {
        (false, _, _) => DragStep::Ignore,
        (true, true, _) => DragStep::Extend,
        (true, false, false) => DragStep::DenyOnly,
        (true, false, true) => DragStep::TakeOver,
    }
}

/// Whether a press is one this module takes over from the label once it moves past the
/// drag threshold: exactly the presses on which GTK's label arms its text drag-and-drop.
/// A single press arms it only inside the current selection; a double or triple press
/// always does, because the word or cell it selects contains the pointer. A Shift-press
/// extends the selection and never arms it.
pub(crate) fn takes_over(presses: i32, shift: bool, inside_selection: bool) -> bool {
    !shift && (presses >= 2 || inside_selection)
}

/// The first controller of type `T` on `label` — called before any of ours is added,
/// so it is the label's own.
fn own_gesture<T: IsA<glib::Object>>(label: &Label) -> Option<T> {
    let controllers = label.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|i| controllers.item(i))
        .find_map(|c| c.downcast::<T>().ok())
}

/// The character offset under a widget-space point, as GTK's own label hit-test
/// computes it (`get_layout_index`: the layout index plus its trailing characters).
/// A point outside the text answers the nearest position, which is what a drag that
/// leaves the cell should extend to.
fn char_at(layout: &pango::Layout, ox: i32, oy: i32, x: f64, y: f64) -> usize {
    let scale = f64::from(pango::SCALE);
    let (_, index, trailing) = layout.xy_to_index(
        ((x - f64::from(ox)) * scale) as i32,
        ((y - f64::from(oy)) * scale) as i32,
    );
    let text = layout.text();
    let index = usize::try_from(index).unwrap_or(0).min(text.len());
    let before = text.get(..index).map_or(0, |s| s.chars().count());
    before + usize::try_from(trailing).unwrap_or(0)
}

/// Whether a word starts and/or ends at one character position — the only thing about
/// the text [`drag_selection`] needs, kept as plain data so it tests without a layout.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WordEdge {
    pub(crate) start: bool,
    pub(crate) end: bool,
}

/// What a drag selects, as `(anchor, end)` character offsets for
/// `gtk_label_select_region`, given the press count, the character under the press,
/// the character under the pointer now, and the text's word edges (one per position,
/// the text's length plus one).
///
/// * 1 press — characters from the press point to the pointer.
/// * 2 presses — whole words: the word under the press, grown to the word under the
///   pointer in whichever direction it lies.
/// * 3 or more — the whole cell, however the pointer moves.
pub(crate) fn drag_selection(
    presses: i32,
    start: usize,
    current: usize,
    words: &[WordEdge],
) -> (i32, i32) {
    let off = |i: usize| i32::try_from(i).unwrap_or(i32::MAX);
    match presses {
        p if p >= 3 => (0, -1),
        2 => {
            let (start_lo, start_hi) = word_around(words, start);
            let (cur_lo, cur_hi) = word_around(words, current);
            if current < start {
                (off(start_hi), off(cur_lo))
            } else {
                (off(start_lo), off(cur_hi))
            }
        }
        _ => (off(start), off(current)),
    }
}

/// Word edges for every position of `text` (its character count plus one).
///
/// Pango's own (`pango_layout_get_log_attrs`) has no Rust binding, so words are
/// letters and digits here, with an apostrophe or hyphen between two of them kept
/// inside the word — `don't` and `well-known` are one word each, as a reader
/// double-clicking them expects (GTK4Rs/AP-162).
pub(crate) fn word_edges(text: &str) -> Vec<WordEdge> {
    let chars: Vec<char> = text.chars().collect();
    let inner = |i: usize| {
        let c = chars[i];
        c.is_alphanumeric()
            || c == '_'
            || (matches!(c, '\'' | '\u{2019}' | '-')
                && i > 0
                && i + 1 < chars.len()
                && chars[i - 1].is_alphanumeric()
                && chars[i + 1].is_alphanumeric())
    };
    let is_word: Vec<bool> = (0..chars.len()).map(inner).collect();
    (0..=chars.len())
        .map(|i| {
            let before = i > 0 && is_word[i - 1];
            let after = i < chars.len() && is_word[i];
            WordEdge {
                start: after && !before,
                end: before && !after,
            }
        })
        .collect()
}

/// The word containing position `at`: back to the nearest word start at or before it,
/// forward to the nearest word end at or after it. A position between words answers
/// the gap it sits in, as GTK's label does.
fn word_around(words: &[WordEdge], at: usize) -> (usize, usize) {
    let last = words.len().saturating_sub(1);
    let at = at.min(last);
    let lo = (0..=at).rev().find(|&i| words[i].start).unwrap_or(0);
    let hi = (at..=last).find(|&i| words[i].end).unwrap_or(last);
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::{
        drag_selection, drag_step, press_inside_selection, takes_over, word_edges as edges,
        DragStep,
    };

    /// Every combination of the four facts, so deleting any arm of the takeover (the
    /// below-threshold deny, the outside-the-label deny, the takeover itself, or the
    /// extend once taken) changes a row here.
    #[test]
    fn a_drag_update_denies_takes_over_or_extends_by_the_table() {
        use DragStep::*;
        for (armed, taken, past, inside, want) in [
            (false, false, true, true, Ignore),
            (false, true, true, true, Ignore),
            (true, false, false, true, DenyOnly),
            (true, false, true, false, DenyOnly),
            (true, false, false, false, DenyOnly),
            (true, false, true, true, TakeOver),
            (true, true, false, false, Extend),
            (true, true, true, true, Extend),
        ] {
            assert_eq!(
                drag_step(armed, taken, past, inside),
                want,
                "armed={armed} taken={taken} past_threshold={past} inside={inside}"
            );
        }
    }

    /// A press is inside the selection at either end and in either order, and never
    /// with no selection.
    #[test]
    fn a_press_inside_the_selection_includes_both_ends() {
        assert!(press_inside_selection(Some((2, 5)), 2));
        assert!(press_inside_selection(Some((2, 5)), 5));
        assert!(press_inside_selection(Some((5, 2)), 3));
        assert!(!press_inside_selection(Some((2, 5)), 6));
        assert!(!press_inside_selection(Some((2, 5)), 1));
        assert!(!press_inside_selection(None, 0));
    }

    /// Exactly the presses GTK's label would turn into a text drag-and-drop are taken
    /// over; a single press outside the selection is the label's own ordinary drag-select,
    /// and a Shift-press is GTK's extension.
    #[test]
    fn only_a_press_the_label_would_drag_out_is_taken_over() {
        assert!(
            takes_over(1, false, true),
            "single press inside the selection"
        );
        assert!(!takes_over(1, false, false), "single press outside it");
        assert!(takes_over(2, false, false), "double press");
        assert!(takes_over(3, false, false), "triple press");
        assert!(!takes_over(1, true, true), "shift-press");
        assert!(!takes_over(2, true, false), "shift double press");
    }

    #[test]
    fn a_single_press_drag_selects_characters_from_the_press_point() {
        let w = edges("alpha beta gamma");
        assert_eq!(drag_selection(1, 2, 9, &w), (2, 9));
        // Dragging leftwards keeps the press point as the anchor.
        assert_eq!(drag_selection(1, 9, 2, &w), (9, 2));
    }

    #[test]
    fn a_double_press_drag_extends_by_whole_words_in_either_direction() {
        let w = edges("alpha beta gamma");
        // Pressed in "beta" (7), dragged into "gamma" (13): "beta gamma".
        assert_eq!(drag_selection(2, 7, 13, &w), (6, 16));
        // Pressed in "beta", dragged into "alpha": anchored at beta's end.
        assert_eq!(drag_selection(2, 7, 1, &w), (10, 0));
        // Pointer still inside the pressed word: just that word.
        assert_eq!(drag_selection(2, 7, 8, &w), (6, 10));
    }

    #[test]
    fn a_triple_press_keeps_the_whole_cell_however_the_pointer_moves() {
        let w = edges("alpha beta gamma");
        assert_eq!(drag_selection(3, 7, 1, &w), (0, -1));
        assert_eq!(drag_selection(4, 0, 16, &w), (0, -1));
    }

    #[test]
    fn positions_past_the_text_are_clamped_not_indexed() {
        let w = edges("ab");
        assert_eq!(drag_selection(2, 0, 99, &w), (0, 2));
    }

    #[test]
    fn an_apostrophe_or_hyphen_between_letters_stays_inside_the_word() {
        let w = edges("don't well-known x-");
        // Double-press in "don't" (1) and hold: the whole contraction.
        assert_eq!(drag_selection(2, 1, 2, &w), (0, 5));
        // In "well-known" (8): one word, both halves.
        assert_eq!(drag_selection(2, 8, 9, &w), (6, 16));
        // A trailing hyphen is not inside a word.
        assert_eq!(drag_selection(2, 17, 17, &w), (17, 18));
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_integration_tests {
    use super::make_cell_selectable;
    use gtk::prelude::*;
    use gtk::{GestureClick, GestureDrag, Label};

    /// Every controller of type `T` on `label`, in dispatch order (newest first).
    fn gestures<T: IsA<gtk::glib::Object>>(label: &Label) -> Vec<T> {
        let list = label.observe_controllers();
        (0..list.n_items())
            .filter_map(|i| list.item(i))
            .filter_map(|c| c.downcast::<T>().ok())
            .collect()
    }

    /// A cell's own gestures sit in the label's click-gesture group, and run ahead of
    /// the label's.
    ///
    /// Ungrouped, the label's claim on every primary press denies them the moment the
    /// press lands, so they hear the press and nothing after it: the drag handler never
    /// runs and a double- or triple-press drag goes back to dragging the text out
    /// (MEASURED on a driven cell before the grouping existed). Mutation: dropping either
    /// `group_with` fails the matching assertion.
    #[gtktest::test]
    fn a_cells_gestures_join_the_labels_own_click_group() {
        let label = Label::new(Some("alpha beta gamma"));
        make_cell_selectable(&label);
        assert!(label.is_selectable());

        let clicks = gestures::<GestureClick>(&label);
        let drags = gestures::<GestureDrag>(&label);
        assert_eq!(
            (clicks.len(), drags.len()),
            (2, 2),
            "the label's own click and drag gesture, and one of each of ours"
        );
        // Newest first: ours lead, the label's own trail.
        let (ours_click, own_click) = (&clicks[0], &clicks[1]);
        let ours_drag = &drags[0];
        let group = own_click.group();
        assert!(
            group
                .iter()
                .any(|g| g == ours_click.upcast_ref::<gtk::Gesture>()),
            "our click gesture is outside the label's group, so the label's claim denies it"
        );
        assert!(
            group
                .iter()
                .any(|g| g == ours_drag.upcast_ref::<gtk::Gesture>()),
            "our drag gesture is outside the label's group, so the label's claim denies it \
             and a double/triple-press drag drags the text out again"
        );
    }

    /// A press never clears the cell's selection itself. Clearing it at the press is what
    /// made a click on a SELECTED link follow the link under an I-beam cursor (MEASURED on
    /// macOS 4.22.4 and read off 4.6.9's `gtk_label_update_active_link`): the label then
    /// saw no selection, found the link active, and activated it on release. A click in a
    /// selection is the label's to collapse on release, without following anything.
    ///
    /// Mutation: reinstating `select_region(0, 0)` on a single press fails this.
    #[gtktest::test]
    fn a_press_leaves_the_cells_selection_to_the_label() {
        let label = Label::new(Some("alpha beta gamma"));
        make_cell_selectable(&label);
        let ours = gestures::<GestureClick>(&label)
            .into_iter()
            .next()
            .expect("our click gesture");
        label.select_region(0, 5);
        for n in 1..=3 {
            ours.emit_by_name::<()>("pressed", &[&n, &1.0f64, &1.0f64]);
            assert_eq!(
                label.selection_bounds(),
                Some((0, 5)),
                "press {n} changed the selection before the label saw it"
            );
        }
        // Release PRIMARY before the label dies: a selection claims the display's
        // PRIMARY clipboard, and an UNREALIZED label keeps owning it past its death (GTK
        // releases it only in `unrealize`, which a never-realized label never runs — so
        // the app's own realized cells are safe). A label finalized still owning it is called back
        // by the NEXT claim — a later test focusing a label — as a dead widget
        // (`gtk_widget_queue_draw: assertion 'GTK_IS_WIDGET (widget)' failed`, fatal
        // under the suite's criticals; measured, intermittent on CI).
        label.select_region(0, 0);
    }
}
