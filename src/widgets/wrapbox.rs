//! `ToolbarWrapBox` — a small custom `GtkWidget` that lays its children out
//! left-to-right, wrapping onto additional rows when a child would overflow the
//! allocated width, and packing every row flush against the LEFT edge.
//!
//! Built to replace an earlier `GtkFlowBox`-based attempt at this (the
//! behavioural contract is TDD 9.38): `GtkFlowBox` computes a shared column grid
//! across every line it lays out, so a later row whose items differ in size
//! from the row(s) that set that grid gets offset/padded rather than packed
//! tightly left — reported as odd gaps opening up on the left of a wrapped
//! row. This widget does the placement itself with a plain greedy
//! left-to-right, top-to-bottom bin-pack (no shared grid, no per-line
//! justification), so there is nothing to misalign: `size_allocate` runs the
//! exact same arithmetic `measure` used to decide the required height, so the
//! two can never disagree.
//!
//! Children keep their own natural size — never stretched, never shrunk. A
//! child that would overflow the current row moves whole to the next one,
//! never squeezed to fit. Invisible children (`set_visible(false)`) are
//! skipped entirely and take no space, exactly like a plain `GtkBox`, so
//! hiding a toolbar section still costs it nothing here.

use gtk::prelude::*;
use gtk::{gdk, glib};

mod imp {
    use super::*;
    use gtk::subclass::prelude::*;
    use std::cell::Cell;

    #[derive(Default)]
    pub(crate) struct ToolbarWrapBox {
        pub(crate) column_spacing: Cell<i32>,
        pub(crate) row_spacing: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ToolbarWrapBox {
        const NAME: &'static str = "ScribToolbarWrapBox";
        type Type = super::ToolbarWrapBox;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("toolbarwrap");
        }
    }

    impl ObjectImpl for ToolbarWrapBox {
        fn dispose(&self) {
            // Children are parented directly (no layout manager owns them) — same
            // contract as `ScribTableWidget`; see `widgets::unparent_all_children`.
            crate::widgets::unparent_all_children(&*self.obj());
        }
    }

    impl WidgetImpl for ToolbarWrapBox {
        // Height depends on the width we're given (more width ⇒ fewer wrapped
        // rows ⇒ less height) — the opposite of `ScribTableWidget`'s
        // `ConstantSize`, which exists specifically to defeat this negotiation.
        // Here the negotiation IS the wrap.
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let children = visible_children(&self.obj());
            let col_sp = self.column_spacing.get();
            let row_sp = self.row_spacing.get();
            match orientation {
                gtk::Orientation::Horizontal => {
                    // Minimum: the widest single child — with unlimited height,
                    // every child could get its own row. Natural: every child on
                    // one row (what a wide-enough window shows), matching what a
                    // non-wrapping GtkBox would have reported.
                    let mut min_w = 0;
                    let mut sum_w = 0;
                    for (i, child) in children.iter().enumerate() {
                        let (_, nat_w, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
                        min_w = min_w.max(nat_w);
                        if i > 0 {
                            sum_w += col_sp;
                        }
                        sum_w += nat_w;
                    }
                    (min_w, sum_w, -1, -1)
                }
                gtk::Orientation::Vertical => {
                    let h = if for_size <= 0 {
                        // No width to wrap against yet — the natural, one-row case.
                        children
                            .iter()
                            .map(|c| c.measure(gtk::Orientation::Vertical, -1).1)
                            .max()
                            .unwrap_or(0)
                    } else {
                        pack(&children, for_size, col_sp, row_sp).1
                    };
                    (h, h, -1, -1)
                }
                _ => (0, 0, -1, -1),
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, baseline: i32) {
            let children = visible_children(&self.obj());
            let col_sp = self.column_spacing.get();
            let row_sp = self.row_spacing.get();
            let (rects, _) = pack(&children, width.max(1), col_sp, row_sp);
            for (child, rect) in children.iter().zip(rects.iter()) {
                child.size_allocate(rect, baseline);
            }
        }
    }

    fn visible_children(widget: &super::ToolbarWrapBox) -> Vec<gtk::Widget> {
        let mut out = Vec::new();
        let mut next = widget.first_child();
        while let Some(child) = next {
            if child.is_visible() {
                out.push(child.clone());
            }
            next = child.next_sibling();
        }
        out
    }

    /// Greedy left-to-right, top-to-bottom pack at `avail_width`: place each
    /// child at its natural size, starting a new row whenever the next child
    /// would overflow. Returns each child's allocation (parallel to `children`)
    /// and the total height used.
    ///
    /// The one seam `measure(Vertical, for_size)` and `size_allocate` share, so
    /// the height `measure` promises and the rows `size_allocate` actually lays
    /// out can never drift apart from each other.
    fn pack(
        children: &[gtk::Widget],
        avail_width: i32,
        col_spacing: i32,
        row_spacing: i32,
    ) -> (Vec<gdk::Rectangle>, i32) {
        let mut rects = Vec::with_capacity(children.len());
        let mut x = 0;
        let mut y = 0;
        let mut row_h = 0;

        for child in children {
            let (_, nat_w, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
            let (_, nat_h, _, _) = child.measure(gtk::Orientation::Vertical, -1);
            let would_overflow = x > 0 && x + col_spacing + nat_w > avail_width;
            if would_overflow {
                y += row_h + row_spacing;
                x = 0;
                row_h = 0;
            }
            let cx = if x == 0 { 0 } else { x + col_spacing };
            rects.push(gdk::Rectangle::new(cx, y, nat_w, nat_h));
            x = cx + nat_w;
            row_h = row_h.max(nat_h);
        }
        let total_h = if rects.is_empty() { 0 } else { y + row_h };
        (rects, total_h)
    }
}

glib::wrapper! {
    pub(crate) struct ToolbarWrapBox(ObjectSubclass<imp::ToolbarWrapBox>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ToolbarWrapBox {
    pub(crate) fn new(column_spacing: i32, row_spacing: i32) -> Self {
        use gtk::subclass::prelude::*;
        let obj: Self = glib::Object::new();
        obj.imp().column_spacing.set(column_spacing);
        obj.imp().row_spacing.set(row_spacing);
        obj
    }

    /// Append `child` as the next flow item, in order — parented directly
    /// (this widget has no layout manager of its own; see `imp::dispose`).
    pub(crate) fn append(&self, child: &impl IsA<gtk::Widget>) {
        child.set_parent(self);
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_integration_tests {
    use super::ToolbarWrapBox;
    use gtk::prelude::*;

    /// A child too wide to share a row with the previous one drops to its own
    /// row, flush against the LEFT edge — never centered/offset the way the
    /// `GtkFlowBox` predecessor was reported doing.
    #[gtktest::test]
    fn a_child_that_does_not_fit_wraps_to_a_new_row_at_x_zero() {
        let wrap = ToolbarWrapBox::new(2, 0);
        let a = gtk::Button::with_label("AAAA");
        let b = gtk::Button::with_label("BBBB");
        wrap.append(&a);
        wrap.append(&b);

        let (_, a_nat_w, _, _) = a.measure(gtk::Orientation::Horizontal, -1);
        let (_, b_nat_w, _, _) = b.measure(gtk::Orientation::Horizontal, -1);
        // Wide enough for `a` alone, too narrow for both side by side.
        let width = a_nat_w + 2;
        assert!(
            width < a_nat_w + 2 + b_nat_w,
            "precondition: the two buttons must not fit on one row at this width"
        );

        let win = gtk::Window::new();
        win.set_child(Some(&wrap));
        win.present();
        crate::testpump::drain_for(
            crate::testpump::Clock::Frame,
            std::time::Duration::from_millis(200),
        );
        // Drive the wrap box's own allocation directly at the exact tight
        // width under test, rather than trusting the window to shrink that
        // far on its own (its own chrome/minimum can keep it wider).
        wrap.size_allocate(&gtk::Allocation::new(0, 0, width, 100), -1);

        let a_alloc = a.allocation();
        let b_alloc = b.allocation();
        assert_eq!(a_alloc.x(), 0, "the first row's item starts at x=0");
        assert_eq!(
            b_alloc.x(),
            0,
            "a wrapped row starts flush at x=0 — no inherited offset from the \
             previous row's column grid (the exact `GtkFlowBox` symptom this \
             widget replaces)"
        );
        assert!(
            b_alloc.y() > a_alloc.y(),
            "the overflowing child must land on a NEW row, below the first"
        );
    }

    /// A hidden child is skipped entirely — no gap, no reserved row.
    #[gtktest::test]
    fn a_hidden_child_takes_no_space() {
        let wrap = ToolbarWrapBox::new(2, 0);
        let a = gtk::Button::with_label("AAAA");
        let hidden = gtk::Button::with_label("HIDDEN");
        hidden.set_visible(false);
        let b = gtk::Button::with_label("BBBB");
        wrap.append(&a);
        wrap.append(&hidden);
        wrap.append(&b);

        let (_, nat_w, _, _) = wrap.measure(gtk::Orientation::Horizontal, -1);

        let win = gtk::Window::new();
        win.set_child(Some(&wrap));
        win.present();
        crate::testpump::drain_for(
            crate::testpump::Clock::Frame,
            std::time::Duration::from_millis(200),
        );
        wrap.size_allocate(&gtk::Allocation::new(0, 0, nat_w, 100), -1);

        let a_alloc = a.allocation();
        let b_alloc = b.allocation();
        assert_eq!(
            a_alloc.y(),
            b_alloc.y(),
            "both visible children share one row"
        );
        assert_eq!(
            b_alloc.x(),
            a_alloc.x() + a_alloc.width() + 2,
            "the hidden middle child must not leave a gap or shift `b` further \
             than `a`'s own width plus spacing"
        );
    }
}
