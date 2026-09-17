//! "Can anyone see this picture right now?" (TDD 27.3 — an animation nobody
//! can see costs nothing, and never animate what is not visible).
//!
//! This is a small decision core — [`current`] — plus the wiring that re-asks it
//! from exactly the signals that can change its answer, so a picture can arrange
//! its own visibility watching from its own host widget ([`watch`]). Nothing here
//! decides what to DO with the answer; [`super::paintable::AnimatedPaintable`]
//! ANDs it with [`super::policy`]'s play/pause decision and owns the consequence
//! (drop the decoder and canvas, or rebuild them from frame 0).
//!
//! # What "visible" means here, and what it deliberately does not cover
//!
//! Per the plan's table, "not visible" is the union of five distinct GTK
//! situations, and no single predicate answers all of them — using the wrong one
//! for a given situation is the dark pattern the plan calls out by name:
//!
//! - **Scrolled out of the preview's viewport.** An anchored `GtkTextView` child
//!   stays MAPPED and is still snapshotted when scrolled away — only its
//!   allocation moves (verified below, [`gtk_tests::claim_1_scrolled_out_stays_mapped_and_geometry_is_the_right_predicate`]).
//!   `is_mapped()` is therefore the WRONG predicate; [`geometry_visible`] is the
//!   right one, and it must compare allocations in WIDGET space
//!   (`compute_bounds`), never `visible_rect()` — that is buffer space and lags
//!   paint (`GTK4Rs/AP-142`).
//! - **Nested inside another widget** (every anchored picture in this renderer:
//!   `renderer::start::anchor_image` always wraps the `GtkPicture` in a
//!   `GtkOverlay` before anchoring it, so `host`'s own allocation is relative to
//!   that `GtkOverlay`, never to the preview's `GtkScrolledWindow` directly).
//!   [`geometry_visible`]'s use of `compute_bounds(host, view)` is what makes this
//!   transparent — it walks whatever ancestor chain actually exists.
//! - **Collapsed `<details>`.** The plan's table describes GTK's generic
//!   mechanism for this (an invisible tag over the child's placeholder character
//!   parks it at `(-w, -h)` without unmapping it — `GTK4Rs/AP-166`). **This
//!   project does not use that mechanism.** `renderer::start::inside_collapsed_body`
//!   means a collapsed body's images are never anchored in the first place, and
//!   `preview::splice` collapses a LIVE body by deleting its buffer range outright
//!   — which, per `GTK4Rs/AP-320`, unparents every anchored child in that range.
//!   So collapsing an expanded `<details>` around a playing animation DESTROYS its
//!   `GtkPicture` (and, once nothing else holds a ref, the `AnimatedPaintable`
//!   itself) rather than parking it — verified below
//!   ([`gtk_tests::claim_2_deleting_a_details_bodys_buffer_range_drops_its_anchored_picture_rather_than_parking_it`]),
//!   and [`AnimatedPaintable`](super::paintable::AnimatedPaintable)'s existing
//!   `dispose()` already handles that case with no help from this module. This
//!   module's [`geometry_visible`] is still the correct general mechanism (and is
//!   exercised by the ordinary scroll/nesting cases above), it is simply not what
//!   makes THIS row of the table pass.
//! - **Background tab / hidden pane / hidden window.** These genuinely unmap the
//!   widget (verified below,
//!   [`gtk_tests::claim_3_a_background_tab_and_a_hidden_pane_do_unmap`]), so
//!   `is_mapped()` is the RIGHT predicate here, and only here.
//! - **Minimized window.** `GdkToplevelState::MINIMIZED` on the toplevel surface,
//!   watched via `notify::state` — never inferred from the frame clock, which does
//!   not necessarily freeze on X11 (claim 4 below; this leg is a runtime skip
//!   under the plain `Xvfb` this pipeline's `#[gtktest::test]` bodies run under,
//!   which has no window manager to honour an iconify request — see
//!   [`gtk_tests::claim_4_and_table_minimized_window`]).
//! - **Covered by another window.** Out of scope per the plan — not observable at
//!   GTK 4.6.

use gtk::glib;
use gtk::prelude::*;

mod watch;

pub(crate) use watch::{watch, VisibilityWatch};

/// Whether `host` — an `AnimatedPaintable`'s host widget — is visible right now,
/// by every mechanism this module knows how to ask about. The conjunction with
/// the Play Animations policy is [`super::paintable`]'s job, not this function's.
pub(crate) fn current(host: &gtk::Widget) -> bool {
    if !host.is_mapped() {
        return false;
    }
    if is_minimized(host) {
        return false;
    }
    match host
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_then(|w| w.downcast::<gtk::ScrolledWindow>().ok())
    {
        Some(view) => geometry_visible(host, view.upcast_ref()),
        // No scrolled ancestor at all (a test fixture with no viewport, or a
        // future host this renderer never anchors inside one) — nothing can clip
        // it, so there is nothing for this leg to refuse.
        None => true,
    }
}

/// Whether `host` is hidden for a reason no geometry can change — unmapped, or in a
/// minimized window. Such a host gets no paint, so anything waiting for its next paint
/// to decide waits forever; a caller that can defer the geometry half of [`current`] to
/// a paint must still act on this half itself.
pub(crate) fn hidden_regardless_of_geometry(host: &gtk::Widget) -> bool {
    !host.is_mapped() || is_minimized(host)
}

/// Is `host` within `view`'s own allocated rectangle, both in WIDGET space?
///
/// `compute_bounds` (`gtk_widget_compute_bounds`) reports `host`'s allocation
/// translated into `view`'s coordinate system, walking whatever ancestor chain
/// actually separates them — one level (a bare anchored picture) or several (a
/// picture inside the `GtkOverlay` this renderer always wraps one in). `None`
/// means no common ancestor, or `host` has no allocation at all (unrealized) —
/// either way, not visible.
///
/// **Never `visible_rect()`** — that reports the buffer-space rectangle the view
/// THINKS it has painted, which lags a live scroll (`GTK4Rs/AP-142`); allocation
/// is the widget's actual on-screen rectangle right now.
pub(crate) fn geometry_visible(host: &gtk::Widget, view: &gtk::Widget) -> bool {
    let Some(bounds) = host.compute_bounds(view) else {
        return false;
    };
    rect_visible(bounds, view.width() as f32, view.height() as f32)
}

/// Pure intersection test: does `bounds` (already in the viewport's own
/// coordinate space) overlap the viewport's own rectangle, `(0, 0, w, h)`? Split
/// out from [`geometry_visible`] so the arithmetic is checkable with no display
/// (`mod tests` below) — everything display-dependent is in producing `bounds` in
/// the first place, not in this comparison.
fn rect_visible(bounds: gtk::graphene::Rect, viewport_w: f32, viewport_h: f32) -> bool {
    if viewport_w <= 0.0 || viewport_h <= 0.0 {
        return false;
    }
    let viewport = gtk::graphene::Rect::new(0.0, 0.0, viewport_w, viewport_h);
    bounds.intersection(&viewport).is_some()
}

/// Is `host`'s toplevel surface currently `GdkToplevelState::MINIMIZED`?
///
/// Never inferred from the frame clock freezing — it does not necessarily do so
/// on X11 (claim 4). `None` at any step (no root yet, the root is not a
/// `GtkWindow`, not yet realized so no surface exists, or the surface is not a
/// toplevel) answers `false`: an unrealized/unrooted widget is already caught by
/// [`current`]'s `is_mapped()` gate before this is ever asked.
fn is_minimized(host: &gtk::Widget) -> bool {
    toplevel_of(host).is_some_and(|t| t.state().contains(gtk::gdk::ToplevelState::MINIMIZED))
}

/// `host`'s toplevel surface, if one exists yet.
fn toplevel_of(host: &gtk::Widget) -> Option<gtk::gdk::Toplevel> {
    host.root()
        .and_then(|r| r.downcast::<gtk::Window>().ok())
        .and_then(|w| w.surface())
        .and_then(|s| s.downcast::<gtk::gdk::Toplevel>().ok())
}

/// Re-resolve `host`'s toplevel and connect `f` to its `notify::state` — the
/// live minimize/restore watch. Returns the connected `(Toplevel,
/// SignalHandlerId)` so the caller can disconnect it later; `None` if `host` has
/// no toplevel surface yet (unrealized) — [`watch::VisibilityWatch`] re-tries this
/// from `host`'s own `map` signal, since a surface always exists by the time a
/// widget is mapped.
fn connect_toplevel_state(
    host: &gtk::Widget,
    f: impl Fn(&gtk::gdk::Toplevel) + 'static,
) -> Option<(gtk::gdk::Toplevel, glib::SignalHandlerId)> {
    let toplevel = toplevel_of(host)?;
    let id = toplevel.connect_state_notify(f);
    Some((toplevel, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::graphene::Rect;

    /// A viewport-space rect entirely inside the viewport overlaps it.
    #[test]
    fn a_rect_fully_inside_the_viewport_is_visible() {
        assert!(rect_visible(
            Rect::new(10.0, 10.0, 50.0, 50.0),
            200.0,
            200.0
        ));
    }

    /// A rect entirely past the viewport's bottom edge (the ordinary
    /// scrolled-away shape: positive y, well past `viewport_h`) does not overlap.
    #[test]
    fn a_rect_entirely_below_the_viewport_is_not_visible() {
        assert!(!rect_visible(
            Rect::new(0.0, 500.0, 100.0, 50.0),
            200.0,
            300.0
        ));
    }

    /// A rect entirely above the viewport (scrolled DOWN past it, so its widget-space
    /// y in the view's own coordinates is negative) does not overlap either — the
    /// intersection test is symmetric, not just a "hasn't arrived yet" check.
    #[test]
    fn a_rect_entirely_above_the_viewport_is_not_visible() {
        assert!(!rect_visible(
            Rect::new(0.0, -200.0, 100.0, 50.0),
            200.0,
            300.0
        ));
    }

    /// A rect straddling the viewport's edge (half on screen) counts as visible —
    /// this is a picture in the middle of scrolling into or out of view, and it
    /// really is partly paintable.
    #[test]
    fn a_rect_straddling_the_bottom_edge_is_visible() {
        assert!(rect_visible(
            Rect::new(0.0, 280.0, 100.0, 50.0),
            200.0,
            300.0
        ));
    }

    /// Mutation-relevant: a degenerate (zero-size) viewport — the shape a
    /// not-yet-allocated `GtkScrolledWindow` reports — never counts as containing
    /// anything, however the bounds compare.
    #[test]
    fn a_zero_size_viewport_is_never_visible() {
        assert!(!rect_visible(Rect::new(0.0, 0.0, 10.0, 10.0), 0.0, 0.0));
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_tests;
