//! Popover anchoring — the rectangle on the way **in** ([`ViewportRect`]) and on the way
//! **out** ([`pointing_to`]).
//!
//! **Contract, inbound (GTK4Rs/AP-26).** A `GtkPopover` pointed at a rectangle outside the visible
//! viewport trips a `GDK_IS_MONITOR` assertion and lays out at a negative height. Any
//! selection- or content-anchored popover must therefore prove its anchor is on screen
//! *before* pointing at it. [`ViewportRect`]'s only constructor runs that proof and clamps
//! the anchor into the viewport, so the check cannot be skipped at a call site: there is no
//! way to obtain the value except by passing.
//!
//! **Both axes, and that was learned the expensive way.** This gate checked the y alone for
//! its first two rounds, on the reasoning — written into its own doc comment — that "only
//! the y can leave the viewport under scrolling". That is false. The editor is
//! `WrapMode::Word` (`window/tabs/lifecycle.rs`), so a long unbroken token overflows
//! horizontally; selecting that line puts the anchor midpoint thousands of pixels to the
//! right of a 600 px view, the y-only gate passes, and the identical
//! `gdk_monitor_get_geometry: assertion 'GDK_IS_MONITOR (monitor)' failed` + negative-height
//! allocation fires **through** the guard (QA round 5, H-1). A safety boundary that is
//! trusted and half-blind is worse than no boundary, because every call site has stopped
//! looking. Hence [`Viewport`]: the extent arrives as one value carrying **both** axes, so a
//! gate for one axis that silently omits the other is no longer expressible.
//!
//! **Contract, outbound.** `gtk_popover_get_pointing_to` is a C out-parameter function: it
//! returns a boolean *and* writes a rectangle, and **the rectangle is undefined when the
//! boolean is false**. gtk-rs binds it as a bare `(bool, gdk::Rectangle)` tuple, which
//! offers a caller an undefined value with no type-level warning that reading it is wrong.
//! [`pointing_to`] is the sanctioned reader; the raw method is banned crate-wide in
//! `clippy.toml`, since a type cannot hide an inherent trait method.

use gtk::gdk;
use gtk::prelude::*;

/// The visible extent of the widget an anchor is measured against — **both axes together**.
///
/// The type exists to make H-1 unrepresentable rather than merely fixed. Passing the two
/// extents as bare `i32`s invites both failures this module has now had: writing a gate for
/// one axis and forgetting the other, and (once the second axis is added) transposing the
/// two adjacent integers at a call site, which produces a gate that is wrong but still
/// plausible. One value, built from the widget itself, admits neither.
///
/// **Bounds, not a size.** The extent is `[left, right) × [top, bottom)` in the widget's
/// own coordinates. For a widget wholly on a monitor that is `[0, width) × [0, height)`;
/// for one hanging past a monitor's edge — a window wider than its screen, or partly off
/// it — it is the part that is on the monitor, which need not start at the widget's
/// origin. GTK looks a popover's monitor up from its ANCHOR (GTK4Rs/AP-26's assertion,
/// reached through an anchor that is inside the widget), so an anchor inside these bounds
/// is one GTK can place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Viewport {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Viewport {
    /// The current visible extent of `widget` — its allocation, narrowed to the part that
    /// lies on a monitor wherever GTK places popovers by screen coordinates.
    ///
    /// The **only** constructor reachable from outside this module, deliberately (GTK4Rs/AP-130):
    /// every production anchor is measured against a real widget, and taking the pair from
    /// the widget removes the call site's opportunity to supply the wrong two numbers.
    pub(crate) fn of(widget: &impl IsA<gtk::Widget>) -> Self {
        let widget = widget.as_ref();
        let whole = Self::new(widget.width(), widget.height());
        match on_monitor(widget) {
            Some(clip) => whole.intersect(clip),
            None => whole,
        }
    }

    /// Module-private so the transposition hazard stays inside this file; the unit tests
    /// below are the only other user, and they are testing this file's own arithmetic.
    fn new(width: i32, height: i32) -> Self {
        Self {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        }
    }

    /// The overlap of two viewports in the same coordinate space; empty when they do not
    /// meet, and an empty viewport anchors nothing.
    fn intersect(self, other: Self) -> Self {
        Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        }
    }
}

/// The part of a widget that lies on the monitor it overlaps most, in the widget's own
/// coordinates — or an empty viewport when it is on no monitor at all.
///
/// `widget` and `monitors` are in the same screen space. The largest overlap rather than
/// every monitor, because a viewport is one rectangle: an anchor inside the result is on
/// that monitor by construction, so GTK's monitor lookup cannot come back empty. The cost
/// is that an anchor on a SECOND monitor the widget also spans is refused; a popover not
/// shown is recoverable, a GDK assertion is not.
fn monitor_clip(widget: &gdk::Rectangle, monitors: &[gdk::Rectangle]) -> Viewport {
    let area = |r: &gdk::Rectangle| i64::from(r.width()) * i64::from(r.height());
    match monitors
        .iter()
        .filter_map(|m| widget.intersect(m))
        .max_by_key(area)
    {
        Some(on) => Viewport {
            left: on.x() - widget.x(),
            top: on.y() - widget.y(),
            right: on.x() + on.width() - widget.x(),
            bottom: on.y() + on.height() - widget.y(),
        },
        None => Viewport::new(0, 0),
    }
}

/// The part of `widget` that is on a monitor, in its own coordinates, or `None` where the
/// platform does not place popovers by screen coordinates (`platform::surface_to_screen`)
/// or the widget is not on screen yet.
///
/// The widget's screen rectangle is built exactly as GTK 4.6.9's `create_popup_layout`
/// builds a popover's anchor (`gtkpopover.c:437-457`, via the private
/// `gtk_widget_get_surface_allocation`): the widget's bounds in its native, floored, plus
/// the native's surface transform — integer arithmetic, no graphene transform — then
/// translated to the screen. An anchor offset added to that origin lands where GTK's does.
///
/// A display reporting no monitors yields `None` rather than an empty clip: refusing every
/// popover because a headless server lists nothing would be a worse failure than the one
/// this guards.
fn on_monitor(widget: &gtk::Widget) -> Option<Viewport> {
    let native = widget.native()?;
    let surface = native.surface()?;
    let bounds = widget.compute_bounds(&native)?;
    let (nx, ny) = native.surface_transform();
    let (sx, sy) = crate::platform::surface_to_screen(
        &surface,
        bounds.x().floor() as i32 + nx as i32,
        bounds.y().floor() as i32 + ny as i32,
    )?;
    let monitors: Vec<gdk::Rectangle> = widget
        .display()
        .monitors()
        .iter::<gdk::Monitor>()
        .filter_map(Result::ok)
        .map(|m| m.geometry())
        .collect();
    if monitors.is_empty() {
        return None;
    }
    let on_screen = gdk::Rectangle::new(
        sx,
        sy,
        bounds.width().ceil() as i32,
        bounds.height().ceil() as i32,
    );
    Some(monitor_clip(&on_screen, &monitors))
}

/// Whether an anchor spanning `[pos, pos + extent)` on ONE axis lies within `[lo, hi)` on
/// that axis.
///
/// Partial visibility counts: an anchor straddling an edge still has an on-screen point to
/// aim at. Fully before (`pos + extent <= lo`), fully after (`pos >= hi`), or an empty
/// extent (`hi <= lo`, e.g. an unallocated view) has none.
///
/// `saturating_add` rather than `+`: `pos` is derived from document content by way of
/// `buffer_to_window_coords`, and the whole reason this gate exists is that content can put
/// it absurdly far outside the viewport. An overflow here would wrap to a negative sum and
/// report an off-viewport anchor as visible — the exact failure, arrived at through the
/// arithmetic instead of through the missing axis.
fn axis_visible(pos: i32, extent: i32, lo: i32, hi: i32) -> bool {
    hi > lo && pos.saturating_add(extent) > lo && pos < hi
}

/// Whether a whole anchor rectangle (`gdk::Rectangle` argument order: x, y, w, h) is
/// visible in `vp` — **the** predicate, applied to both axes in one place.
///
/// **Module-private on purpose.** This is the *predicate*; every caller wants the
/// predicate **and** the clamp that follows it, and the two were separable for exactly as
/// long as it took a caller to take one without the other. [`ViewportRect::at`] and
/// [`on_viewport`] are the two public shapes, and both apply both halves — so the
/// half-application is now unrepresentable rather than merely discouraged (GTK4Rs/AP-130: seal
/// the exit API once every caller is inside one module).
fn anchor_visible(vp: Viewport, x: i32, y: i32, w: i32, h: i32) -> bool {
    axis_visible(x, w, vp.left, vp.right) && axis_visible(y, h, vp.top, vp.bottom)
}

/// Clamp a widget-space coordinate into `[lo, hi)` — the other half of the guard.
fn clamp_into(v: i32, lo: i32, hi: i32) -> i32 {
    v.clamp(lo, (hi - 1).max(lo))
}

/// Width of the caret-sliver rectangle [`pin_above`] actually points at.
///
/// Shared with [`ViewportRect::at`]'s gate so the rectangle that is *checked* has the same
/// extent as the rectangle that is *pointed at*. Checking one shape and pointing at another
/// is how a guard passes and GTK still receives something off-viewport.
const CARET_SLIVER_W: i32 = 1;

/// The on-viewport form of a full anchor **rectangle**, with its origin clamped into the
/// viewport, or `None` when the rectangle is off-viewport entirely.
///
/// The shape for a popover anchored to something with real extent — a drawn chip, a
/// marker, a badge — as opposed to [`ViewportRect`]'s caret-sliver anchor. Width and height
/// pass through untouched; **both** the x and the y are gated and clamped (see the module
/// header for why "only the y can leave the viewport" was wrong).
///
/// Returning the clamped rectangle rather than a bare `bool` is the point: a caller that
/// merely *asks* whether its rect is visible and then points at the **unclamped** original
/// still hands GTK a negative origin whenever the anchor straddles an edge — passing the
/// guard and tripping the assertion anyway. Here the only value you can point at is the
/// safe one.
pub(crate) fn on_viewport(vp: Viewport, rect: &gdk::Rectangle) -> Option<gdk::Rectangle> {
    if !anchor_visible(vp, rect.x(), rect.y(), rect.width(), rect.height()) {
        return None;
    }
    Some(gdk::Rectangle::new(
        clamp_into(rect.x(), vp.left, vp.right),
        clamp_into(rect.y(), vp.top, vp.bottom),
        rect.width(),
        rect.height(),
    ))
}

/// Pin `pop` above a proven on-viewport anchor — **the sole sink for a selection-anchored
/// popover.**
///
/// Takes a [`ViewportRect`], so the GTK4Rs/AP-26 guard cannot be skipped: there is no way to call
/// this with an unproven anchor. It also owns the caret-sliver rectangle shape
/// (`width = 1`, height = the line) and the `Top` placement, which three call sites
/// previously hand-rolled — one of them re-deriving the guard and the clamp inline as well.
pub(crate) fn pin_above(pop: &impl IsA<gtk::Popover>, vr: &ViewportRect) {
    let pop = pop.as_ref();
    pop.set_pointing_to(Some(&gdk::Rectangle::new(
        vr.x(),
        vr.y(),
        CARET_SLIVER_W,
        vr.line_h(),
    )));
    pop.set_position(gtk::PositionType::Top);
}

/// A popover anchor point **proven on-viewport** (GTK4Rs/AP-26).
///
/// The only constructor is [`at`](Self::at), which runs [`anchor_visible`] on both axes and
/// clamps the anchor into the viewport, returning `None` when it is off-viewport — so
/// holding one of these *is* the proof, and a caller that has none must decide what to do
/// instead (dismiss, hide, or skip) rather than pointing at nowhere.
///
/// Holds plain `i32`s, not a `gdk::Rectangle`, so it stays display-free and unit-testable;
/// the rectangle is built at the sink that consumes it.
pub(crate) struct ViewportRect {
    x: i32,
    y: i32,
    line_h: i32,
}

impl ViewportRect {
    /// The on-viewport anchor for a point at widget coords `(x, y)` with line height
    /// `line_h`, in viewport `vp`, or `None` when it is off-viewport on **either** axis.
    ///
    /// The anchor gated here is the [`CARET_SLIVER_W`]-wide rectangle [`pin_above`] will
    /// point at, not the selection it was derived from. So a selection whose visible left
    /// half is on screen but whose *midpoint* is not — the long-unbroken-token case that
    /// produced H-1 — yields `None` and the caller declines to show the popover. That is
    /// the deliberate reading: the sliver is what GTK is handed, so the sliver is what has
    /// to be visible. A caller wanting to keep the popover in that case must supply an
    /// anchor x that is genuinely on screen; it may not ask this to bless one that is not.
    pub(crate) fn at(vp: Viewport, x: i32, y: i32, line_h: i32) -> Option<Self> {
        if !anchor_visible(vp, x, y, CARET_SLIVER_W, line_h) {
            return None;
        }
        Some(Self {
            // Clamped on both axes, structurally identically. On x the clamp provably
            // cannot bite — an extent of 1 makes the predicate exactly `0 <= x < width` —
            // and it is kept anyway so the two axes have the same shape. The asymmetry
            // between them is the entire content of H-1; a reader comparing the two lines
            // should find nothing to explain.
            x: clamp_into(x, vp.left, vp.right),
            y: clamp_into(y, vp.top, vp.bottom),
            line_h,
        })
    }

    /// The x, already clamped into `[0, width)`.
    pub(crate) fn x(&self) -> i32 {
        self.x
    }

    /// The y, already clamped into `[0, height)`.
    pub(crate) fn y(&self) -> i32 {
        self.y
    }

    pub(crate) fn line_h(&self) -> i32 {
        self.line_h
    }
}

/// The rectangle `popover` is currently pointing at, or `None` when it has none.
///
/// The sanctioned reader for `gtk_popover_get_pointing_to` (see the module contract): the
/// raw binding hands back a `(bool, gdk::Rectangle)` whose bool is `priv->has_pointing_to`
/// ("an explicit anchor was set"), not "the rectangle is meaningful" — and nothing about
/// that tuple stops a caller reading the second element regardless. When the bool is false
/// the rectangle is a **fallback value, not garbage** (gtk/gtkpopover.c:2264-2292,
/// researcher-confirmed against GTK 4.22.4/main, 2026-07-30): with a parent widget it is
/// the parent's own bounds; with none (as in a never-`set_parent`'d popover) computing
/// those bounds fails and it is `memset` to a zero rect — deterministically, not
/// undefined — and that failure itself fires a `GTK_IS_WIDGET` `CRITICAL` (`:2278/:2280`),
/// which is therefore expected log noise from this call, not a bug in it. Either way the
/// rectangle means nothing as an anchor once the bool is false, so `None` here discards it
/// unread rather than trying to characterise which fallback it was.
#[allow(clippy::disallowed_methods)]
pub(crate) fn pointing_to(popover: &impl IsA<gtk::Popover>) -> Option<gdk::Rectangle> {
    let (is_set, rect) = popover.as_ref().pointing_to();
    is_set.then_some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 600×400 viewport, the size QA round 5 reproduced H-1 in.
    fn vp() -> Viewport {
        Viewport::new(600, 400)
    }

    #[test]
    fn anchor_visibility_tracks_the_viewport_edges() {
        // Fully inside.
        assert!(anchor_visible(vp(), 250, 100, 1, 18));
        // Straddling the TOP edge (partly visible) still counts — there is a point on
        // screen to aim at.
        assert!(anchor_visible(vp(), 250, -5, 1, 18));
        // Scrolled fully above the top: no visible anchor.
        assert!(!anchor_visible(vp(), 250, -30, 1, 18));
        // On the bottom edge / below it: hidden.
        assert!(!anchor_visible(vp(), 250, 400, 1, 18));
        assert!(!anchor_visible(vp(), 250, 500, 1, 18));
        // A zero-height (unallocated) view never anchors.
        assert!(!anchor_visible(Viewport::new(600, 0), 250, 10, 1, 18));
    }

    /// **H-1.** The x axis is gated exactly as the y is — this is the axis the guard
    /// was blind to, through which the assertion it exists to prevent actually fired.
    ///
    /// Mutation guard: drop the `axis_visible(x, w, vp.width)` conjunct from
    /// `anchor_visible` and every assertion in this test fails. That is the whole
    /// point — the y-only predicate passed all of these.
    #[test]
    fn the_x_axis_is_gated_exactly_as_the_y_axis_is() {
        // Straddling the LEFT edge: partly visible, so it still anchors.
        assert!(anchor_visible(vp(), -3, 100, 14, 18));
        // Scrolled fully off to the left: no visible anchor.
        assert!(!anchor_visible(vp(), -30, 100, 14, 18));
        // On the right edge and past it: hidden.
        assert!(!anchor_visible(vp(), 600, 100, 14, 18));
        assert!(!anchor_visible(vp(), 900, 100, 14, 18));
        // A zero-WIDTH (unallocated) view never anchors, exactly as a zero-height one
        // does not.
        assert!(!anchor_visible(Viewport::new(0, 400), 10, 10, 14, 18));
    }

    /// The measured H-1 reproduction, as a predicate case.
    ///
    /// The editor is `WrapMode::Word`, so a long unbroken token overflows horizontally.
    /// Selecting that line gave `x0 = 2`, `x1 = 36002` in a 600 px view — a midpoint of
    /// 18002. The y-only gate returned `Some`, `pin_above` pointed there, and GTK
    /// answered with `gdk_monitor_get_geometry: assertion 'GDK_IS_MONITOR (monitor)'
    /// failed` ×3 and a negative-height allocation.
    #[test]
    fn a_horizontally_overflowing_selection_midpoint_does_not_anchor() {
        let (x0, x1) = (2, 36002);
        let midpoint = (x0 + x1) / 2;
        assert_eq!(midpoint, 18002, "the measured reproduction's anchor x");
        assert!(
            ViewportRect::at(vp(), midpoint, 100, 18).is_none(),
            "an anchor 18002 px into a 600 px view must never reach set_pointing_to — \
             it is the exact input that fired GDK_IS_MONITOR through this guard"
        );
        // …while the same selection's genuinely on-screen left edge still anchors, so
        // the fix is a gate and not a blanket refusal.
        assert!(ViewportRect::at(vp(), x0, 100, 18).is_some());
    }

    /// An absurd anchor x cannot wrap the extent addition into a "visible" verdict.
    #[test]
    fn an_overflowing_anchor_coordinate_saturates_rather_than_wrapping() {
        assert!(
            !axis_visible(i32::MAX, 1, 0, 600),
            "far right, no wrap to negative"
        );
        assert!(
            !axis_visible(i32::MIN, 1, 0, 600),
            "far left, no wrap to positive"
        );
        assert!(ViewportRect::at(vp(), i32::MAX, 100, 18).is_none());
        assert!(ViewportRect::at(vp(), i32::MIN, 100, 18).is_none());
    }

    /// The clamp is the half a caller drops when it asks a bare predicate.
    ///
    /// This exists because a caller did exactly that: it tested visibility and then pointed
    /// its popover at the UNCLAMPED rectangle, so a chip straddling the top edge passed the
    /// guard and still handed GTK a negative y. Returning the clamped rect is what makes
    /// that unrepresentable, so the clamp is the assertion.
    #[test]
    fn on_viewport_gates_a_rect_and_clamps_its_origin() {
        // Fully inside: passes through unchanged, extent intact.
        let r = on_viewport(vp(), &gdk::Rectangle::new(560, 100, 14, 18)).expect("on-viewport");
        assert_eq!((r.x(), r.y(), r.width(), r.height()), (560, 100, 14, 18));

        // Straddling the TOP edge: visible, and the NEGATIVE y is clamped to 0 — the whole
        // point. Width and height are untouched; only the origin is clamped.
        let r = on_viewport(vp(), &gdk::Rectangle::new(560, -5, 14, 18)).expect("straddling");
        assert_eq!(r.y(), 0, "a negative anchor y never reaches GTK");
        assert_eq!((r.x(), r.width(), r.height()), (560, 14, 18));

        // Straddling the LEFT edge: the same story on the axis H-1 was blind to.
        let r = on_viewport(vp(), &gdk::Rectangle::new(-5, 100, 14, 18)).expect("straddling");
        assert_eq!(r.x(), 0, "a negative anchor x never reaches GTK either");
        assert_eq!((r.y(), r.width(), r.height()), (100, 14, 18));

        // Straddling the BOTTOM / RIGHT edges: still visible, origin clamped inside.
        let r = on_viewport(vp(), &gdk::Rectangle::new(560, 395, 14, 18)).expect("straddling");
        assert!(r.y() < 400, "clamped inside the height, got {}", r.y());
        let r = on_viewport(vp(), &gdk::Rectangle::new(595, 100, 14, 18)).expect("straddling");
        assert!(r.x() < 600, "clamped inside the width, got {}", r.x());

        // Off-viewport in ANY direction, and unallocated on EITHER axis: no rect.
        assert!(on_viewport(vp(), &gdk::Rectangle::new(560, -30, 14, 18)).is_none());
        assert!(on_viewport(vp(), &gdk::Rectangle::new(560, 500, 14, 18)).is_none());
        assert!(on_viewport(vp(), &gdk::Rectangle::new(-30, 100, 14, 18)).is_none());
        assert!(on_viewport(vp(), &gdk::Rectangle::new(900, 100, 14, 18)).is_none());
        assert!(
            on_viewport(Viewport::new(600, 0), &gdk::Rectangle::new(560, 10, 14, 18)).is_none()
        );
        assert!(on_viewport(Viewport::new(0, 400), &gdk::Rectangle::new(10, 10, 14, 18)).is_none());
    }

    #[test]
    fn viewport_rect_gates_and_clamps_the_anchor() {
        // On-viewport: constructs, x/y unchanged, line_h carried through.
        let r = ViewportRect::at(vp(), 250, 100, 18).expect("on-viewport anchor");
        assert_eq!((r.x(), r.y(), r.line_h()), (250, 100, 18));
        // Straddling the TOP edge: passes the guard, y CLAMPED to 0 — never negative,
        // which is what would drive the off-viewport GDK assertion.
        let r = ViewportRect::at(vp(), 30, -5, 18).expect("top-straddling anchor is visible");
        assert_eq!(r.y(), 0, "negative y is clamped into the viewport");
        assert_eq!(r.x(), 30);
        // Off-viewport (above / below / left / right) and unallocated on either axis:
        // no rect at all, so there is nothing a caller could point a popover at.
        assert!(ViewportRect::at(vp(), 30, -30, 18).is_none(), "fully above");
        assert!(ViewportRect::at(vp(), 30, 500, 18).is_none(), "fully below");
        assert!(
            ViewportRect::at(vp(), -1, 100, 18).is_none(),
            "left of the view"
        );
        assert!(
            ViewportRect::at(vp(), 600, 100, 18).is_none(),
            "right of the view"
        );
        assert!(
            ViewportRect::at(Viewport::new(600, 0), 30, 10, 18).is_none(),
            "unallocated height"
        );
        assert!(
            ViewportRect::at(Viewport::new(0, 400), 30, 10, 18).is_none(),
            "unallocated width"
        );
    }

    /// The measured off-monitor case (TDD 17.48a): a 1336 px window at screen x=0 on a
    /// single 1280 px monitor. Its right 56 px are on no monitor, so a chip at x=1303 must
    /// not anchor while one at x=1265 still does — the two chips GTK 4.6.9 refused and
    /// accepted.
    #[test]
    fn a_widget_wider_than_its_monitor_anchors_only_on_the_monitor() {
        let window = gdk::Rectangle::new(0, 0, 1336, 720);
        let monitor = [gdk::Rectangle::new(0, 0, 1280, 1024)];
        let vp = Viewport::new(1336, 720).intersect(monitor_clip(&window, &monitor));
        assert_eq!(
            vp,
            Viewport {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720
            }
        );
        assert!(on_viewport(vp, &gdk::Rectangle::new(1303, 116, 14, 18)).is_none());
        assert!(on_viewport(vp, &gdk::Rectangle::new(1265, 116, 14, 18)).is_some());
    }

    /// A window placed past the monitor's LEFT or TOP edge clips from the widget's own
    /// origin inward, and the clamp keeps an anchor inside that clip — the case a
    /// size-only viewport could not express.
    #[test]
    fn a_widget_hanging_off_the_left_and_top_clips_from_its_origin() {
        let widget = gdk::Rectangle::new(-200, -50, 800, 600);
        let monitor = [gdk::Rectangle::new(0, 0, 1920, 1080)];
        let vp = Viewport::new(800, 600).intersect(monitor_clip(&widget, &monitor));
        assert_eq!(
            vp,
            Viewport {
                left: 200,
                top: 50,
                right: 800,
                bottom: 600
            }
        );
        assert!(
            on_viewport(vp, &gdk::Rectangle::new(100, 100, 14, 18)).is_none(),
            "left of the monitor"
        );
        let r =
            on_viewport(vp, &gdk::Rectangle::new(195, 45, 14, 18)).expect("straddles both edges");
        assert_eq!((r.x(), r.y()), (200, 50), "clamped onto the monitor");
    }

    /// Two monitors: the clip is the one the widget overlaps most, and a widget on no
    /// monitor at all anchors nothing.
    #[test]
    fn the_clip_is_the_most_overlapped_monitor_and_nowhere_is_empty() {
        let monitors = [
            gdk::Rectangle::new(0, 0, 1920, 1080),
            gdk::Rectangle::new(1920, 0, 2560, 1440),
        ];
        let widget = gdk::Rectangle::new(1700, 100, 1000, 500);
        assert_eq!(
            monitor_clip(&widget, &monitors),
            Viewport {
                left: 220,
                top: 0,
                right: 1000,
                bottom: 500
            },
            "780 px on the second monitor beats 220 px on the first"
        );
        let nowhere = monitor_clip(&gdk::Rectangle::new(9000, 9000, 100, 100), &monitors);
        assert!(on_viewport(nowhere, &gdk::Rectangle::new(10, 10, 14, 18)).is_none());
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_integration_tests {
    use super::*;

    /// The wrapped call, not just the type (POLICY): an unset popover must read as `None`,
    /// and a set one must read back the rectangle it was given.
    ///
    /// The first half is the one that matters — it is the case where the raw binding hands
    /// out a rectangle that means nothing. Mutation guard: replace the body with
    /// `Some(rect)` and the unset assertion fails.
    ///
    /// The popover is deliberately **parented**, which makes the unset case STRONGER rather
    /// than weaker: per [`pointing_to`]'s contract, a parentless popover's fallback is a
    /// zeroed rect, while a parented one's is the parent's own bounds — so this asserts the
    /// seam discards a *plausible-looking* rectangle, not an obviously-empty one.
    ///
    /// It is also what lets the suite run under `G_DEBUG=fatal-criticals` at all
    /// (POLICY § Logging). The parentless variant fires a `GTK_IS_WIDGET` `CRITICAL` from
    /// GTK's own fallback path — harmless log noise, documented on [`pointing_to`], and
    /// fatal under that flag, which would make the gate unarmable for one line of noise.
    #[gtktest::test]
    fn an_unset_anchor_reads_as_none_and_a_set_one_round_trips() {
        let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let pop = gtk::Popover::new();
        pop.set_parent(&parent);
        assert!(
            pointing_to(&pop).is_none(),
            "a popover that was never pointed anywhere has no anchor — the raw binding \
             hands back a fallback rectangle here (the PARENT's bounds, since this popover \
             has one), and that fallback must be discarded regardless"
        );

        let rect = gdk::Rectangle::new(11, 22, 33, 44);
        pop.set_pointing_to(Some(&rect));
        let got = pointing_to(&pop).expect("a pointed popover has an anchor");
        assert_eq!(
            (got.x(), got.y(), got.width(), got.height()),
            (11, 22, 33, 44)
        );
        // A parented popover must be unparented before it is dropped (GTK4Rs/AP-80).
        pop.unparent();
    }
}
