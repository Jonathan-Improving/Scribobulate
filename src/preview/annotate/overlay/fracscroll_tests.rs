//! The action popover's scroll-hide against a fractional wheel-scroll value.
//!
//! In its own file because `overlay.rs` is past the soft line limit.

use crate::codeview::CodePreviewView;
use crate::testpump::{destroy_after_tooltip_delay, drain_for, until, until_or_for, Clock};
use gtk::prelude::*;
use std::time::Duration;

/// The action popover (`.annotation-overlay`) parented to `view`.
fn action_popover(view: &CodePreviewView) -> gtk::Popover {
    let mut child = view.first_child();
    while let Some(c) = child {
        if let Ok(pop) = c.clone().downcast::<gtk::Popover>() {
            if pop.has_css_class("annotation-overlay") {
                return pop;
            }
        }
        child = c.next_sibling();
    }
    panic!("the action popover is wired and parented to the preview view");
}

/// A wheel scroll leaves the preview's vadjustment at a fraction. Showing the action
/// popover allocates the view it is parented to, and that allocation writes the truncated
/// value back: a `value-changed` that moves no text. The scroll-hide must not take it for
/// a scroll, or the first selection after a wheel scroll shows no Annotate popover.
///
/// The fraction is planted with a plain adjustment write (`#[gtktest::test]` cannot
/// synthesise a wheel event), and asserted still present just before the selection, so
/// the test cannot pass on a value an earlier frame already snapped. It also asserts the
/// write-back happened, so it cannot pass on a run that never exercised it.
#[gtktest::test]
fn the_first_selection_after_a_fractional_scroll_keeps_the_annotate_popover() {
    let mut md = String::new();
    for i in 0..300 {
        md.push_str(&format!(
            "Paragraph {i} carries some selectable prose here.\n\n"
        ));
    }
    let win = gtk::Window::new();
    let pane = crate::preview::render(
        None,
        &md,
        None,
        1.0,
        false,
        &crate::fold::FoldState::default(),
    );
    let scroller = pane
        .downcast_ref::<gtk::Overlay>()
        .expect("preview pane is a GtkOverlay")
        .child()
        .and_downcast::<gtk::ScrolledWindow>()
        .expect("overlay wraps the scroller");
    let view = scroller
        .child()
        .and_downcast::<CodePreviewView>()
        .expect("scroller holds the CodePreviewView");
    // The popover only shows with an annotation sink (the split view wires one).
    view.set_annotation_sink(std::rc::Rc::new(|_| {}));
    win.set_default_size(700, 500);
    win.set_child(Some(&pane));
    win.present();

    let vadj = scroller.vadjustment();
    until(
        Clock::Frame,
        "the preview to map with a scroll range",
        || view.is_mapped() && vadj.page_size() > 0.0 && vadj.upper() > vadj.page_size() + 600.0,
    );
    drain_for(Clock::Frame, Duration::from_millis(300));

    // Where five wheel steps left the editor in the live reproduction.
    crate::saferizer::scrollpos::jump(&vadj, 416.401_609_200_426_4);
    drain_for(Clock::Frame, Duration::from_millis(300));
    assert!(
        vadj.value().fract() != 0.0,
        "precondition: the preview must still hold the fractional value when the text is \
         selected, or nothing below is tested (it reads {})",
        vadj.value()
    );

    // A few words a third of the way down the viewport: on screen, clear of the edges.
    let vy = view.visible_rect().y() + WidgetExt::height(&view) / 3;
    let (iter, _) = view.line_at_y(vy);
    let buf = view.buffer();
    let a = buf.iter_at_offset(iter.offset());
    let b = buf.iter_at_offset((iter.offset() + 9).min(buf.end_iter().offset()));
    buf.select_range(&a, &b);

    let pop = action_popover(&view);
    // Not a fatal wait: with the defect the popover opens and closes within one frame, so
    // it may never be seen open, and the assertion below names that failure.
    until_or_for(Clock::Frame, Duration::from_secs(2), || pop.is_visible());
    drain_for(Clock::Frame, Duration::from_millis(400));
    assert!(
        pop.is_visible(),
        "the Annotate popover closed on the first selection after a fractional scroll"
    );
    // GTK 4.6 writes the truncated value back when the popover's opening re-lays-out the
    // view; GTK 4.22 (macOS and Windows CI) does not, so there the case this guards never
    // arises and the half above is all that is checked.
    if vadj.value().fract() != 0.0 {
        crate::testsymlink::skipped(
            "TDD 17.5 truncated write-back",
            "this GTK left the fractional scroll value in place when the popover opened, so \
             the write-back the fix guards against did not happen",
        );
    }

    // And a scroll that does move the text still hides it, so the test cannot pass on a
    // scroll-hide that was simply switched off.
    crate::saferizer::scrollpos::jump(&vadj, vadj.value() + 120.0);
    assert!(
        !pop.is_visible(),
        "a scroll that moves the text must still hide the Annotate popover"
    );

    destroy_after_tooltip_delay(&win, || pop.popdown());
}
