//! Modifier+wheel over the preview steps the zoom ladder instead of scrolling.
//!
//! ## Why this is an input, not a command
//!
//! The zoom ladder already has one command per direction (`win.zoom-in` /
//! `win.zoom-out`), reachable from the View menu, the toolbar and an accelerator.
//! This module adds a fourth *input* to those same actions and nothing else — it
//! activates them by name, so the ladder bounds, the edit-mode lockout and the
//! greyed-out toolbar button all keep deciding whether the gesture does anything
//! (POLICY's single-`GAction` rule; ScrAP-9). It is the same shape as
//! `navhistory`'s two thumb-button gestures, and it is here for the same reason
//! they are not accelerators: a wheel is not expressible as an accelerator string.
//!
//! ## The modifier is the platform's own
//!
//! Ctrl on Linux and Windows, **Command** on macOS — the same swap
//! [`crate::accel`] performs on every declared accelerator, asked for as a mask
//! rather than a string ([`crate::accel::primary_modifier_for_host`]). Two facts
//! make that the right choice rather than a tidy one, both source-traced:
//! `<Primary>` maps to `GDK_CONTROL_MASK` *unconditionally* in `gtkaccelgroup.c`
//! (GTK4 has no built-in "Primary means Command on macOS"), and the Quartz backend
//! reports Command as `GDK_META_MASK` in an event's own modifier state
//! (`gdkmacosdisplay-translate.c:146-147`), not only when matching accelerators.
//! macOS additionally reserves Ctrl+scroll for the system's Accessibility zoom.
//!
//! ## Why capture phase, and why after construction
//!
//! Two separate races, and only one of them is what "capture" buys — the
//! distinction matters because the loose version of this paragraph is refutable by
//! a two-minute mutation and the precise one is not.
//!
//! `GtkScrolledWindow` installs **two** scroll controllers, and they are
//! complementary rather than redundant: the bubble one
//! (`gtkscrolledwindow.c:2145-2153`) scrolls only `if (!priv->smooth_scroll)`
//! (`:1461-1472`), and the capture one (`:2155-2164`) scrolls only when
//! `priv->smooth_scroll` is set (`:1285-1301`). Discrete wheels are serviced by
//! the first, smooth sequences by the second.
//!
//! - Against the **bubble** controller, being added last is what wins:
//!   `gtk_widget_add_controller` PREPENDS (`gtkwidget.c:11461`) and
//!   `gtk_widget_run_controllers` walks head-first (`:4523`), so the controller
//!   added LAST in a phase runs FIRST. Installing this one after the scroller is
//!   built is therefore load-bearing — the same fact `wheelcoalesce` depends on.
//! - Against the **capture** controller, only the phase wins. `smooth_scroll` is
//!   set by the bubble controller's `scroll-begin` on the first event of a
//!   sequence (`:1377`), so from the SECOND event onward the capture handler is
//!   the one that moves the view and claims the event. A `Stop` in the bubble
//!   phase arrives after it has already scrolled — the pane would scroll AND
//!   zoom.
//!
//! ⚠️ **MEASURED, and it is a FALSE NEGATIVE worth knowing about:** rebuilding
//! this module with `PropagationPhase::Bubble` and driving a classic wheel under
//! Xvfb on GTK 4.6.9 changes *nothing* — the ladder still steps, the pane still
//! does not scroll. A discrete wheel never sets `smooth_scroll`
//! (`gtk_event_controller_scroll_begin` is reached only inside the smooth branch,
//! `gtkeventcontrollerscroll.c:373`), so it never enters the branch where the two
//! phases differ. Do not re-run that mutation and conclude the phase is free; it
//! needs a touchpad to discriminate.
//!
//! `Propagation::Stop` from the capture phase suppresses the scroll completely:
//! `gtk_widget_run_controllers` breaks the loop for a handled non-gesture
//! controller (`gtkwidget.c:4580-4587`), so the scroller's own controllers never
//! run — no `scroll-begin`, no `scroll_history_push`, and so no `::decelerate` at
//! the end of the sequence. A bubble `Stop` would leave all three happening, and
//! every touchpad zoom would end in a kinetic glide.
//!
//! ⚠️ **One consequence of that suppression is UNMEASURED.** The scroller's
//! capture handler opens by cancelling any deceleration still running from an
//! earlier fling (`gtkscrolledwindow.c:1293`), and stopping the event means that
//! cancel never happens — so a touchpad fling followed, *while it is still
//! gliding*, by a modified scroll could keep gliding while the zoom steps. It is
//! unreachable with a classic wheel, which has no kinetic phase, so this seat
//! cannot test it. If it reproduces, the remedy is a real
//! `set_kinetic_scrolling(false)` → `(true)` toggle, whose `else` branch calls
//! `cancel_deceleration` (`:1293`) — a toggle, because the setter early-returns on
//! an unchanged value, and never mid-touch-drag, because it also re-phases the
//! drag/swipe/long-press/pan gestures. `tests/MANUAL-TEST.md` §13.12 carries the
//! probe.
//!
//! The whole gesture is MEASURED end to end under Xvfb on GTK 4.6.9, both ways:
//! removing the `install` call from `preview::render` leaves Ctrl+wheel doing
//! nothing at all, and restoring it steps the ladder to both of its ends.
//!
//! One behaviour difference falls out of the phase and is deliberate: capture runs
//! this controller BEFORE `CodePreviewView`'s own descendant scroll controller, so
//! that view's `user_scrolling` flag is not set during a zoom. The reader is
//! zooming, not scrolling, and the flag decides where a later zoom re-anchors.

use super::*;

/// Wheel travel that completes one rung of the zoom ladder.
///
/// One unit is one wheel *notch*, on every device GDK reports: a classic wheel's
/// discrete click arrives as `dy = ±1` exactly (`gdkdevicemanager-xi2.c:1749-1782`
/// admits an event to the discrete path only on that exact magnitude), and a
/// hi-res wheel or a touchpad delivers the same notch as a run of fractional
/// smooth deltas. Accumulating travel rather than counting events is what makes
/// those two devices agree; a step per *event* would run the whole nine-rung
/// ladder in one flick of a hi-res wheel, which reports about four events a notch.
const TRAVEL_PER_STEP: f64 = 1.0;

/// How many ladder rungs `dy` completes, given `residue` carried from earlier
/// events of the same gesture, and what residue is left over.
///
/// **At most one rung per event, and travel beyond it is dropped rather than
/// banked.** Both halves are deliberate. The cap bounds the work one event can
/// order — each rung is a full preview re-render — so a single coarse touchpad
/// delta cannot queue five of them into one main-loop turn; dropping the excess is
/// what stops that same travel arriving one event later as the rungs the cap just
/// refused. A fast gesture still climbs quickly, one rung per event, because a
/// fast gesture delivers many events.
///
/// Pure, and separated from the controller for that reason: everything that can be
/// got wrong here — losing sub-rung travel, stepping twice on one fast event, a
/// residue that never drains — is decidable from two numbers.
///
/// Positive `dy` is **down**, which the caller reads as zoom OUT.
fn steps_from(residue: f64, dy: f64) -> (i32, f64) {
    let travel = residue + dy;
    let whole = (travel / TRAVEL_PER_STEP).trunc();
    if whole == 0.0 {
        return (0, travel);
    }
    // Clamped, and the residue goes with the clamp: the rungs this event did not
    // take are travel the reader no longer gets to spend.
    (whole.clamp(-1.0, 1.0) as i32, 0.0)
}

/// The `win.*` action that steps the ladder in the direction `steps` names.
///
/// Wheeling **up** (`dy` negative) zooms **in**, which is what every application
/// that binds this gesture does.
fn action_for(steps: i32) -> &'static str {
    if steps < 0 {
        "win.zoom-in"
    } else {
        "win.zoom-out"
    }
}

/// Install the modifier+wheel zoom gesture on a preview `GtkScrolledWindow`.
///
/// Called from [`crate::preview::render`], the single site a preview scroller is
/// built — so a preview cannot be created without it, which is the enforcement
/// mechanism POLICY's "Typed GTK seams" asks for. The controller dies with the
/// scroller (the preview is freed and rebuilt on every mode switch), so no
/// accumulated travel survives a render.
pub(crate) fn install(scroller: &gtk::ScrolledWindow) {
    let controller = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    // Deliberately NOT `EventControllerScrollFlags::DISCRETE`. Its accumulator
    // (`gtkeventcontrollerscroll.c:375-396`) is never reset — not on scroll-begin,
    // not on scroll-end, not on a device change, and no public call zeroes it — so
    // sub-threshold nudges from unrelated gestures minutes apart still add up. It
    // also emits `trunc(accumulated)`, which is an INTEGER delta and not a UNIT
    // one, so it does not answer the question it looks like it answers. The
    // accumulator below is the same arithmetic with a reset this module controls.
    let residue = std::rc::Rc::new(std::cell::Cell::new(0.0f64));
    controller.connect_scroll(glib::clone!(
        #[weak]
        scroller,
        #[strong]
        residue,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |controller, _dx, dy| {
            if !controller
                .current_event_state()
                .contains(crate::accel::primary_modifier_for_host())
            {
                // Read per event, never cached (`gtkeventcontroller.c:669-678` is a
                // straight `gdk_event_get_modifier_state`), so releasing the
                // modifier mid-gesture hands the rest of it back to the scroller —
                // and the travel banked for a zoom does not survive to bias it.
                residue.set(0.0);
                return glib::Propagation::Proceed;
            }
            let (steps, left) = steps_from(residue.get(), dy);
            residue.set(left);
            if steps != 0 {
                let action = action_for(steps);
                // Through the action, never straight to `apply_zoom`: the action
                // owns whether the command is available right now, so the wheel is
                // as disabled at a ladder end, and in edit mode, as the greyed-out
                // toolbar button is. Activating a disabled action is a silent
                // no-op, which is the wanted behaviour here.
                WidgetExt::activate_action(&scroller, action, None)
                    .unwrap_or_else(|e| log::warn!("zoom wheel: no {action} action: {e}"));
            }
            // Claimed whether or not a whole rung completed: the reader is holding
            // the zoom modifier, so this gesture is a zoom even on the events that
            // only accumulate travel, and letting one through would scroll the pane
            // in the middle of a zoom.
            glib::Propagation::Stop
        }
    ));
    scroller.add_controller(controller);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A classic wheel's discrete click is `dy = ±1` exactly, so it is one rung in
    /// the direction the wheel turned, with nothing left over.
    #[test]
    fn one_wheel_click_is_one_rung() {
        assert_eq!(steps_from(0.0, -1.0), (-1, 0.0));
        assert_eq!(steps_from(0.0, 1.0), (1, 0.0));
    }

    /// Up is in. Stated as its own case because the sign convention is the one
    /// thing here a reader cannot check by reasoning about the arithmetic.
    #[test]
    fn wheeling_up_zooms_in() {
        assert_eq!(action_for(-1), "win.zoom-in");
        assert_eq!(action_for(1), "win.zoom-out");
    }

    /// A hi-res wheel and a touchpad deliver a notch as a run of fractional
    /// deltas — about four to the notch on the hi-res mice that fail GDK's exact
    /// `±1.0` discrete test. One rung per notch of travel, not one per event.
    #[test]
    fn fractional_travel_accumulates_to_whole_rungs() {
        let mut residue = 0.0;
        let mut total = 0;
        for _ in 0..8 {
            let (steps, left) = steps_from(residue, -0.25);
            total += steps;
            residue = left;
        }
        assert_eq!(total, -2, "8 × 0.25 of travel is two notches, so two rungs");
        assert_eq!(residue, 0.0);
    }

    /// One event never orders more than one rung, however much travel it carries —
    /// a rung is a full preview re-render, and a coarse touchpad delta must not
    /// queue five of them into one main-loop turn.
    #[test]
    fn a_single_event_never_steps_more_than_one_rung() {
        assert_eq!(steps_from(0.0, -5.0), (-1, 0.0));
        assert_eq!(steps_from(0.0, 5.0), (1, 0.0));
    }

    /// …and the rungs it refused are dropped, not banked: the next event must not
    /// collect what the cap just declined, or the cap only delays the leap.
    #[test]
    fn refused_travel_is_dropped_not_banked() {
        let (_steps, residue) = steps_from(0.0, -5.0);
        assert_eq!(
            steps_from(residue, -0.1),
            (0, -0.1),
            "the four unspent rungs must not reappear on the next event"
        );
    }

    /// Travel that reverses inside a rung cancels rather than compounding, so a
    /// reader who overshoots and comes back does not owe a rung in each direction.
    #[test]
    fn reversing_within_a_rung_cancels() {
        let (steps, residue) = steps_from(0.0, 0.6);
        assert_eq!(steps, 0);
        assert_eq!(steps_from(residue, -0.6), (0, 0.0));
    }
}
