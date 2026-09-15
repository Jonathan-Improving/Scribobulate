//! Where a point on a GDK surface lies on the X11 screen — the one piece of X11 plumbing
//! the popover anchor guard needs and GTK 4.6 does not expose publicly.
//!
//! GTK 4.6–4.12's X11 popup layout looks a popover's monitor up from its anchor in ROOT
//! coordinates and asserts when no monitor contains it (GTK4Rs/AP-26's assertion, reached
//! through an anchor that is inside its widget). The call it makes is
//! `XTranslateCoordinates` from the parent surface's XID to the display's root window,
//! with the point scaled by the surface scale going in and divided by it coming out
//! (`gdk_x11_surface_get_root_coords`). This reproduces that call so
//! `saferizer::popover_anchor` can ask the same question before GTK does. It owns no
//! decision.
//!
//! Hand-rolled rather than a `gdk4-x11` dependency: four GDK getters, exported from
//! `libgtk-4`, and one Xlib call.

use gtk::gdk;
use gtk::glib;
use gtk::glib::translate::{FromGlib, ToGlibPtr};
use gtk::prelude::*;
use std::os::raw::{c_int, c_ulong, c_void};

extern "C" {
    fn gdk_x11_display_get_type() -> glib::ffi::GType;
    fn gdk_x11_display_get_xdisplay(display: *mut gdk::ffi::GdkDisplay) -> *mut c_void;
    fn gdk_x11_display_get_xrootwindow(display: *mut gdk::ffi::GdkDisplay) -> c_ulong;
    fn gdk_x11_surface_get_xid(surface: *mut gdk::ffi::GdkSurface) -> c_ulong;
}

#[link(name = "X11")]
extern "C" {
    fn XTranslateCoordinates(
        display: *mut c_void,
        src_w: c_ulong,
        dest_w: c_ulong,
        src_x: c_int,
        src_y: c_int,
        dest_x: *mut c_int,
        dest_y: *mut c_int,
        child: *mut c_ulong,
    ) -> c_int;
}

/// `(x, y)` on `surface`, in the root window's coordinates — the space `gdk::Monitor`
/// geometry is in — or `None` when the display is not X11 or the surface has no window.
pub(crate) fn surface_to_screen(surface: &gdk::Surface, x: i32, y: i32) -> Option<(i32, i32)> {
    let display = surface.display();
    // SAFETY: a GType getter with no preconditions.
    let x11_display = unsafe { glib::Type::from_glib(gdk_x11_display_get_type()) };
    if !display.type_().is_a(x11_display) {
        return None;
    }
    let scale = surface.scale_factor().max(1);
    let display_ptr: *mut gdk::ffi::GdkDisplay = display.to_glib_none().0;
    let surface_ptr: *mut gdk::ffi::GdkSurface = surface.to_glib_none().0;
    // SAFETY: both pointers are borrowed from GObjects held alive for this call, and the
    // display was just checked to be a GdkX11Display, so its surfaces are X11 surfaces —
    // the one precondition all three getters have.
    let (xdisplay, root, xid) = unsafe {
        (
            gdk_x11_display_get_xdisplay(display_ptr),
            gdk_x11_display_get_xrootwindow(display_ptr),
            gdk_x11_surface_get_xid(surface_ptr),
        )
    };
    if xdisplay.is_null() || xid == 0 {
        return None;
    }
    let (mut tx, mut ty, mut child): (c_int, c_int, c_ulong) = (0, 0, 0);
    // SAFETY: `xdisplay` is GDK's own live connection and the out-parameters are locals.
    let same_screen = unsafe {
        XTranslateCoordinates(
            xdisplay,
            xid,
            root,
            x.saturating_mul(scale),
            y.saturating_mul(scale),
            &mut tx,
            &mut ty,
            &mut child,
        )
    };
    (same_screen != 0).then_some((tx / scale, ty / scale))
}
