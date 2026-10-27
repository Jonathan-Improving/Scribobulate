//! macOS: whether a window is still zoomed (maximized) while another window's full-screen
//! Space is current — the one question GDK answers wrongly there.
//!
//! # What goes wrong, traced (GTK 4.22.4)
//!
//! GDK-macOS has one writer for the maximized bit: `windowDidMove` in
//! `gdk/macos/GdkMacosWindow.c:297-304`, which copies `[NSWindow isZoomed]`. AppKit
//! answers `isZoomed` by comparing the frame with the window's *standard frame*, which
//! GDK's `windowWillUseStandardFrame:defaultFrame:` (`:641-646`) returns unchanged from
//! AppKit — a frame built from the screen's `visibleFrame`. While a full-screen Space is
//! current the menu bar and Dock are hidden, `visibleFrame` grows, and a zoomed window
//! whose frame has not moved stops matching it. MEASURED: `isZoomed` and
//! `gtk_window_is_maximized` both turn false ~1.7 s after a sibling enters full screen,
//! and both turn true again within ~35 ms of it leaving. A session saved at that moment
//! — quitting from the full-screen window — recorded the zoomed sibling as plain.
//!
//! # The signal used instead
//!
//! Remember a window's frame whenever AppKit says it is zoomed. At capture, a window
//! reads as zoomed if AppKit says so now **or its frame is still exactly the remembered
//! one**. A real unzoom moves the frame back to its previous size, so it still reads
//! false; only a frame that has not moved since it was last zoomed reads true.
//!
//! A full-screen window is left out: AppKit calls it zoomed in its own Space (MEASURED
//! — `isZoomed` true at quit for a window that was never zoomed), and its maximized bit
//! is GTK's to report.
//!
//! The frame is refreshed on `notify::maximized` and on the surface's `layout` signal
//! (a display change re-zooms the window with no `notify`), and only while `isZoomed`
//! holds — a remembered frame is never one AppKit did not call zoomed.

use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::ffi::{c_char, c_void};

type Id = *mut c_void;
type Sel = *const c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn sel_registerName(name: *const c_char) -> Sel;
    /// Untyped on purpose: on `aarch64-apple-darwin` it must be called through a pointer
    /// of the selector's exact signature, never through a variadic declaration.
    fn objc_msgSend();
    /// The struct-return variant x86-64 uses for a 32-byte `NSRect`; arm64 has none.
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_stret();
}

unsafe extern "C" {
    /// Public GDK API since 4.8 (`gdk/macos/gdkmacossurface.h`): the surface's
    /// `NSWindow*`. Resolved from the already-linked `libgtk-4`.
    fn gdk_macos_surface_get_native_window(surface: *mut c_void) -> *mut c_void;
}

/// `NSRect` (`CGRect`): origin then size, four `CGFloat`s (`f64` on 64-bit).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

thread_local! {
    /// The frame each window had when AppKit last called it zoomed. A weak key, pruned
    /// on every write, so a closed window leaves nothing behind.
    static ZOOMED_FRAMES: RefCell<Vec<(glib::WeakRef<gtk::Window>, Rect)>> =
        const { RefCell::new(Vec::new()) };
}

/// Keep `window`'s remembered zoomed frame current for as long as it lives.
pub(crate) fn watch(window: &gtk::Window) {
    window.connect_notify_local(Some("maximized"), |window, _| remember(window));
    if window.is_realized() {
        watch_surface(window);
    }
    window.connect_realize(watch_surface);
}

/// Refresh on every layout of `window`'s surface. The surface lives no longer than the
/// realization, so each realize connects its own.
fn watch_surface(window: &gtk::Window) {
    let Some(surface) = window.surface() else {
        return;
    };
    let weak = window.downgrade();
    surface.connect_layout(move |_, _, _| {
        if let Some(window) = weak.upgrade() {
            if window.is_maximized() {
                remember(&window);
            }
        }
    });
}

/// Record `window`'s frame if AppKit calls it zoomed right now.
fn remember(window: &gtk::Window) {
    let Some((zoomed, frame)) = native_state(window) else {
        return;
    };
    if !zoomed {
        return;
    }
    ZOOMED_FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        frames.retain(|(w, _)| w.upgrade().is_some_and(|w| &w != window));
        frames.push((window.downgrade(), frame));
    });
}

/// Whether `window` is zoomed, by AppKit's answer or by a frame that has not moved since
/// AppKit last gave it.
pub(crate) fn still_zoomed(window: &gtk::Window) -> bool {
    let Some((zoomed, frame)) = native_state(window) else {
        return false;
    };
    zoomed
        || ZOOMED_FRAMES.with(|frames| {
            frames
                .borrow()
                .iter()
                .any(|(w, f)| w.upgrade().is_some_and(|w| &w == window) && *f == frame)
        })
}

/// `[NSWindow isZoomed]` and `[NSWindow frame]`, or `None` before the window has a
/// native window.
fn native_state(window: &gtk::Window) -> Option<(bool, Rect)> {
    let surface = window.surface()?;
    // SAFETY: `surface` is a live `GdkSurface` for the call; on the macOS backend every
    // surface is a `GdkMacosSurface`, and the function returns NULL rather than failing
    // for one without a native window.
    let ns = unsafe { gdk_macos_surface_get_native_window(surface.as_ptr().cast()) };
    if ns.is_null() {
        return None;
    }
    let is_zoomed = selector(c"isZoomed")?;
    let frame = selector(c"frame")?;
    // SAFETY: `ns` is the surface's live `NSWindow*`. Each send goes through a pointer of
    // the selector's exact signature: `-isZoomed` returns `BOOL` (one byte on both
    // architectures), `-frame` an `NSRect` — in registers on arm64 (a homogeneous
    // aggregate of four doubles), through the struct-return entry on x86-64.
    unsafe {
        let send_bool: unsafe extern "C" fn(Id, Sel) -> u8 =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let zoomed = send_bool(ns, is_zoomed) != 0;
        #[cfg(not(target_arch = "x86_64"))]
        let rect = {
            let send_rect: unsafe extern "C" fn(Id, Sel) -> Rect =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            send_rect(ns, frame)
        };
        #[cfg(target_arch = "x86_64")]
        let rect = {
            let mut rect = Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            };
            let send_rect: unsafe extern "C" fn(*mut Rect, Id, Sel) =
                std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
            send_rect(&mut rect, ns, frame);
            rect
        };
        Some((zoomed, rect))
    }
}

/// An interned selector; `None` only if the runtime returns none.
fn selector(name: &std::ffi::CStr) -> Option<Sel> {
    // SAFETY: `name` is a live NUL-terminated C string for the call; the result is a
    // process-lifetime interned pointer.
    let sel = unsafe { sel_registerName(name.as_ptr()) };
    (!sel.is_null()).then_some(sel)
}
