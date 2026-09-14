//! `TickHandle` — a frame-clock tick callback you cannot leak by forgetting it.
//!
//! `gtk::TickCallbackId` carries **no `Drop`**: dropping it leaves GTK's registration
//! installed and throws away the only thing that could remove it. The callback then runs
//! forever, and — because an installed tick holds `gdk_frame_clock_begin_updating` — it
//! pins the WHOLE TOPLEVEL's clock at display rate, not just its own animation. Two
//! sites in this module wrote `self.tick = None` where they meant "stop", and one of them
//! was reachable from a failed frame decode, i.e. from ordinary malformed input (QA
//! finding, 2026-09-12). The mistake is invisible at the call site and invisible to a
//! test that asks the owner whether it is ticking — the owner's own field says "no".
//!
//! So the contract is promoted into a type rather than restated in a comment
//! (POLICY § Typed GTK seams): dropping a `TickHandle` REMOVES the registration, so
//! `self.tick = None`, a `take()`, an early return and a panic all do the right thing.
//!
//! ⚠ **The one case that is NOT a drop**: returning `glib::ControlFlow::Break` from
//! inside the callback — GTK has already unregistered it by then, and removing it again
//! would be a second removal of one registration. That path calls [`TickHandle::forget`],
//! which is deliberately noisy to read.

/// A live frame-clock tick registration. Removing it on drop is the whole point.
pub(crate) struct TickHandle(Option<gtk::TickCallbackId>);

impl TickHandle {
    pub(crate) fn new(id: gtk::TickCallbackId) -> Self {
        TickHandle(Some(id))
    }

    /// Give up ownership WITHOUT removing the registration — correct only where GTK has
    /// already unregistered the callback, which is exactly and only the path that returns
    /// [`glib::ControlFlow::Break`] from inside it.
    pub(crate) fn forget(mut self) {
        let _ = self.0.take();
    }
}

impl Drop for TickHandle {
    fn drop(&mut self) {
        if let Some(id) = self.0.take() {
            id.remove();
        }
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_tests {
    use super::*;
    use gtk::prelude::*;
    use std::cell::Cell;
    use std::rc::Rc;

    /// The guard for the whole point of this type: dropping the handle must REMOVE the
    /// GTK registration, not merely forget it.
    ///
    /// **Driven on a bare widget with nothing that could re-arm it**, which is what makes
    /// the observation mean something: in the real drivers a paint legitimately installs a
    /// fresh tick, so "ticks are still arriving" there cannot distinguish a leaked
    /// registration from a re-armed one. Here the only registration in existence is the
    /// one under test, so a tick after the drop can only be the leak.
    #[gtktest::test]
    fn dropping_the_handle_removes_the_registration() {
        let window = gtk::Window::new();
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        window.set_child(Some(&widget));
        window.present();
        crate::testpump::until(crate::testpump::Clock::Frame, "the widget maps", || {
            widget.is_mapped()
        });

        let ticks = Rc::new(Cell::new(0u64));
        let handle = {
            let ticks = Rc::clone(&ticks);
            TickHandle::new(widget.add_tick_callback(move |_, _| {
                ticks.set(ticks.get() + 1);
                gtk::glib::ControlFlow::Continue
            }))
        };
        crate::testpump::drain_for(
            crate::testpump::Clock::Frame,
            std::time::Duration::from_millis(150),
        );
        let while_installed = ticks.get();
        assert!(
            while_installed > 0,
            "precondition: the frame clock must be running, or this test asserts nothing"
        );

        drop(handle);
        crate::testpump::drain_for(
            crate::testpump::Clock::Frame,
            std::time::Duration::from_millis(50),
        );
        let settled = ticks.get();
        crate::testpump::drain_for(
            crate::testpump::Clock::Frame,
            std::time::Duration::from_millis(250),
        );

        assert_eq!(
            ticks.get(),
            settled,
            "dropping a TickHandle must remove the registration: GTK ran the callback {} \
             more time(s) over 250ms after the only handle to it was dropped, which is an \
             unremovable callback holding the toplevel's frame clock at display rate",
            ticks.get() - settled
        );
        window.close();
    }

    /// **QA finding (2026-09-12), MEASURED**: GTK keeps RUNNING a widget's tick
    /// callback once the widget is unmapped but still realized — the
    /// background-tab/hidden-pane shape `animation::visibility::gtk_tests`'s own
    /// `claim_3_a_background_tab_and_a_hidden_pane_do_unmap` proves genuinely
    /// unmaps (while `farscroll.rs`'s own doc comment records that such a widget
    /// stays REALIZED, so it is not torn down). MEASURED here: 19 further calls
    /// in 300ms after unmapping, on a widget with no other tick registration —
    /// GTK does NOT stop calling a tick callback merely because its widget
    /// unmapped.
    ///
    /// This brackets the fix `animation::sprites`'s visibility watch performs:
    /// the RETENTION half was already **CERTAIN** regardless of this
    /// measurement, because the only PRE-EXISTING pruning
    /// (`SpriteTable::end_pass`) runs from INSIDE a paint an
    /// unmapped widget never gets. What this measurement settles is the OTHER
    /// half: a background tab does not merely retain a sprite's decoder and
    /// canvas idly — its tick callback (and therefore `SpriteAnim::on_tick`'s
    /// own schedule poll/decode dispatch) keeps running at display rate for a
    /// widget nobody can see, so the CPU cost compounds on top of the memory
    /// cost rather than sitting dormant beside it. This is exactly what makes
    /// the fix's visibility-driven drop of every entry — which also removes the tick
    /// registration, via each `SpriteAnim`'s own `Drop` — matter for CPU, not
    /// only for memory.
    ///
    /// Uses the counting instrument [`dropping_the_handle_removes_the_registration`]
    /// established above, on a widget moved through a `GtkStack` exactly as
    /// `claim_3` does — a widget with nothing else that could re-arm a tick, so
    /// any count observed after the unmap can only be GTK still running the
    /// registration.
    #[gtktest::test]
    fn measurement_tick_callbacks_on_a_realized_but_unmapped_widget() {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let other_page = gtk::Label::new(Some("other"));
        let stack = gtk::Stack::new();
        stack.add_titled(&widget, Some("w"), "Widget");
        stack.add_titled(&other_page, Some("o"), "Other");
        stack.set_visible_child_name("w");

        let window = gtk::Window::new();
        window.set_default_size(200, 200);
        window.set_child(Some(&stack));
        window.present();
        crate::testpump::until(crate::testpump::Clock::Frame, "the widget maps", || {
            widget.is_mapped()
        });

        let ticks = Rc::new(Cell::new(0u64));
        let handle = {
            let ticks = Rc::clone(&ticks);
            TickHandle::new(widget.add_tick_callback(move |_, _| {
                ticks.set(ticks.get() + 1);
                gtk::glib::ControlFlow::Continue
            }))
        };
        crate::testpump::drain_for(
            crate::testpump::Clock::Frame,
            std::time::Duration::from_millis(150),
        );
        assert!(
            ticks.get() > 0,
            "precondition: the frame clock must be running while mapped, or this \
             test asserts nothing"
        );

        stack.set_visible_child_name("o");
        crate::testpump::until(
            crate::testpump::Clock::Idle,
            "the background tab to unmap its content",
            || !widget.is_mapped(),
        );
        assert!(
            widget.is_realized(),
            "precondition: a background tab's content stays REALIZED, not torn \
             down (farscroll.rs's own doc comment records this)"
        );

        let at_unmap = ticks.get();
        crate::testpump::drain_for(
            crate::testpump::Clock::Frame,
            std::time::Duration::from_millis(300),
        );
        let after = ticks.get();

        // MEASURED direction (2026-09-12, this host): GTK keeps calling the
        // callback. If a future GTK version changes this, this assertion goes
        // red — invert it and update the doc comment above rather than
        // deleting the test; the memory-retention half of the finding is
        // unaffected either way, since it never depended on this answer.
        assert!(
            after > at_unmap,
            "MEASURED: GTK stopped running the tick callback once the widget \
             unmapped (count stayed at {at_unmap} for 300ms) — if this is now \
             true on this GTK version, animation::sprites's visibility-watch \
             fix still stands (it releases the decoder either way), but the \
             CPU half of this finding's rationale needs updating"
        );

        drop(handle);
        window.close();
    }
}
