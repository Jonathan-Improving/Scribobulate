//! The preview's one-selection rule for table cells, and the action card's surface.
//!
//! In its own file because `overlay.rs` is past the soft line limit.

use super::{action_card_of, claim_selection_for_press};
use crate::codeview::CodePreviewView;
use crate::testpump::{destroy_after_tooltip_delay, drain_for, until, Clock};
use gtk::prelude::*;
use std::time::Duration;

const MD: &str = "Body prose to select before the table.\n\n\
    | Name | Value |\n|------|-------|\n| alpha cell | beta cell |\n| gamma cell | delta cell |\n";

/// Render [`MD`] into a presented window and return it with its view and its cells.
fn table_window() -> (gtk::Window, CodePreviewView, Vec<gtk::Label>) {
    let win = gtk::Window::new();
    let pane = crate::preview::render(
        None,
        MD,
        None,
        1.0,
        false,
        &crate::fold::FoldState::default(),
    );
    let view = pane
        .downcast_ref::<gtk::Overlay>()
        .expect("preview pane is a GtkOverlay")
        .child()
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("overlay wraps the scroller")
        .child()
        .and_downcast::<CodePreviewView>()
        .expect("scroller holds the CodePreviewView");
    // The action card only shows with an annotation sink (the split view wires one).
    view.set_annotation_sink(std::rc::Rc::new(|_| {}));
    win.set_default_size(700, 400);
    win.set_child(Some(&pane));
    win.present();
    until(Clock::Frame, "the table's cells to lay out", || {
        crate::preview::scrib_labels(&view).is_some_and(|l| {
            let l = l.borrow();
            l.len() >= 4 && l.iter().all(|c| c.width() > 0)
        })
    });
    let cells = crate::preview::scrib_labels(&view)
        .expect("the table registered its cells")
        .borrow()
        .clone();
    (win, view, cells)
}

/// The cell whose text is `text`.
fn cell(cells: &[gtk::Label], text: &str) -> gtk::Label {
    cells
        .iter()
        .find(|l| l.text() == text)
        .cloned()
        .unwrap_or_else(|| panic!("no cell reads {text:?}"))
}

/// The centre of `label`, in `view`'s coordinates.
fn centre_of(label: &gtk::Label, view: &CodePreviewView) -> (f64, f64) {
    label
        .translate_coordinates(
            view,
            f64::from(label.width()) / 2.0,
            f64::from(label.height()) / 2.0,
        )
        .expect("the cell is inside the view")
}

/// **A press in another cell clears the first cell's selection.**
///
/// Each cell is its own selectable label, and a label's selection is sticky: before the
/// fix, clicking a different cell left the first cell highlighted, and the Annotate card
/// its selection raised stayed up over it (TDD 17.63). Mutation: drop
/// the clearing loop from `claim_selection_for_press` and this fails.
#[gtktest::test]
fn a_press_in_another_cell_clears_the_first_cells_selection() {
    let (win, view, cells) = table_window();
    let alpha = cell(&cells, "alpha cell");
    let delta = cell(&cells, "delta cell");
    alpha.select_region(0, 5);
    assert!(
        alpha.selection_bounds().is_some(),
        "precondition: the first cell holds a selection"
    );
    let (x, y) = centre_of(&delta, &view);
    claim_selection_for_press(&view, x, y);
    assert!(
        alpha.selection_bounds().is_none(),
        "a press in another cell must clear the first cell's selection"
    );
    destroy_after_tooltip_delay(&win, || ());
}

/// **A press in a cell clears the body text's selection.** A cell press claims no
/// selection of its own, so without this the body's selection — and the Annotate card
/// over it — stayed. Mutation: drop the body collapse and this fails.
#[gtktest::test]
fn a_press_in_a_cell_clears_the_body_selection() {
    let (win, view, cells) = table_window();
    let delta = cell(&cells, "delta cell");
    let buf = view.buffer();
    buf.select_range(&buf.iter_at_offset(0), &buf.iter_at_offset(4));
    assert!(
        buf.has_selection(),
        "precondition: the body holds a selection"
    );
    let (x, y) = centre_of(&delta, &view);
    claim_selection_for_press(&view, x, y);
    assert!(
        !buf.has_selection(),
        "a press in a cell must clear the body text's selection"
    );
    destroy_after_tooltip_delay(&win, || ());
}

/// **A press in body text or empty page space clears a cell's selection, and leaves the
/// body's own selection to GTK.** The cell half used to ride on the text view moving its
/// cursor, which a click GTK swallows (inside the double-click time, see
/// `wire_cell_press_exclusivity`) never does. The body half must stay GTK's, or a
/// Shift-press extending a body selection would be collapsed before GTK saw it.
/// Mutation: return early for a body press before the clearing loop and the first
/// assertion fails; collapse the body there and the second does.
#[gtktest::test]
fn a_press_in_body_text_clears_the_cells_and_leaves_the_body_to_gtk() {
    let (win, view, cells) = table_window();
    let alpha = cell(&cells, "alpha cell");
    let buf = view.buffer();
    let rect = view.iter_location(&buf.iter_at_offset(2));
    let (x, y) = view.buffer_to_window_coords(gtk::TextWindowType::Widget, rect.x(), rect.y());
    let (x, y) = (f64::from(x), f64::from(y + 2));

    alpha.select_region(0, 5);
    assert!(
        alpha.selection_bounds().is_some(),
        "precondition: the cell holds a selection"
    );
    claim_selection_for_press(&view, x, y);
    assert!(
        alpha.selection_bounds().is_none(),
        "a press in body text must clear a cell's selection"
    );

    buf.select_range(&buf.iter_at_offset(0), &buf.iter_at_offset(4));
    claim_selection_for_press(&view, x, y);
    assert!(
        buf.has_selection(),
        "a body press leaves the body selection to GTK"
    );
    destroy_after_tooltip_delay(&win, || ());
}

/// **The Annotate action card is part of the window's own surface (TDD 17.62).** A popover is its
/// own window-system surface, which the window manager left on screen when the window was
/// minimized or moved to another desktop. Mutation: build the action card as a popover
/// again and this fails.
#[gtktest::test]
fn the_action_card_lives_in_the_windows_own_surface() {
    let (win, view, _) = table_window();
    let buf = view.buffer();
    buf.select_range(&buf.iter_at_offset(0), &buf.iter_at_offset(4));
    let card = action_card_of(&view).expect("the action card is wired onto the view");
    until(Clock::Frame, "the action card to show", || {
        card.is_visible()
    });
    drain_for(Clock::Frame, Duration::from_millis(100));
    assert_eq!(
        card.native(),
        win.native(),
        "the action card must share the window's native surface"
    );
    destroy_after_tooltip_delay(&win, || card.set_visible(false));
}
