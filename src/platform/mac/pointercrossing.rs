//! Give a toplevel the pointer crossing events GDK's Quartz backend otherwise never
//! delivers to it — the bookkeeping every other backend gets from its display server
//! for free, and without which no cursor this application sets is ever applied.
//!
//! # What happens, traced
//!
//! A cursor set with `gtk_widget_set_cursor_from_name` was never applied — the pointer
//! kept the default arrow over body text, over links, over a `GtkEntry`, and over
//! `GtkSourceView`'s own built-in text beam — for the whole life of a window that
//! **mapped while the pointer was outside the rectangle it came up in**. Park the
//! pointer inside that rectangle and every cursor applies, always.
//!
//! The last GDK frame before the backend drops the push on the floor
//! (`gdk/gdksurface.c:1741`, `gdk_surface_set_cursor_internal`):
//!
//! ```text
//! if (surface == pointer_info->surface_under_pointer)
//!   update_cursor (surface->display, device);          // gdksurface.c:1754
//! ```
//!
//! No `else`. Not equal and the whole push is discarded, silently, before it reaches
//! `gdk_macos_device_set_surface_cursor`. That is why setting the cursor on the
//! toplevel rather than the hovered child changes nothing: the comparison is per
//! `GdkSurface`, one per toplevel.
//!
//! `surface_under_pointer` has exactly two writers. **Crossing events**
//! (`gdksurface.c:2203/2205` — enter sets it, leave clears it; motion never touches
//! it), and **the ungrab branch of `switch_to_pointer_grab`**
//! (`gdkdisplay.c:727/737`), which resolves the surface geometrically and is reached
//! when an implicit button grab ends.
//!
//! On Quartz the first never fires. `GdkMacosBaseView`'s `NSTrackingArea` is created
//! `NSMakeRect(0,0,0,0)` with no `NSTrackingInVisibleRect`
//! (`gdk/macos/GdkMacosBaseView.c:39-53`), and a 0×0 area generates no crossing, ever.
//! The only enter a macOS toplevel receives is a **synthetic** one posted once at map
//! time (`gdk/macos/GdkMacosWindow.c:193-220`), gated on:
//!
//! ```text
//! if (NSPointInRect ([NSEvent mouseLocation], [self frame]))
//! ```
//!
//! Miss that test and `surface_under_pointer` stays NULL forever. Hit it and it is set
//! — and since the same 0×0 area never yields a *leave* either, nothing ever clears it,
//! which is why a window that starts working keeps working. Both halves of the observed
//! behaviour fall out of one `if`.
//!
//! # The correction
//!
//! `gdk_surface_set_input_region` is the only public route to `-setInputArea:`
//! (`gdkmacossurface.c:156-170`), which **replaces** that 0×0 tracking area with a real
//! one. After it the window receives genuine `mouseEntered:`/`mouseExited:` and GDK
//! maintains `surface_under_pointer` correctly in *both* directions — this is a repair,
//! not a latch, and it does not care where the pointer was at map time.
//!
//! GTK never does this for a toplevel itself: `gtkwindow.c:4222` gates its own
//! `set_input_region` call on `use_client_shadow`, which is hardcoded FALSE on macOS
//! (`gdkmacosdisplay.c:631`) whether or not the window is client-side decorated. So
//! **every** macOS toplevel is in the failing class.
//!
//! ⚠ **This is load-bearing on an implementation detail and the next reader deserves to
//! know it.** Input regions exist to describe where a surface accepts input; the
//! coupling to tracking areas is an internal choice of the macOS backend, made in commit
//! `f207402228` ("macos: use input_region to specify tracking areas", Feb 2022). If that
//! coupling is ever undone this module stops working silently — the cursor simply goes
//! back to being an arrow, with nothing failing anywhere.
//!
//! ⚠ **NOTHING IN THIS REPOSITORY CAN CATCH THAT, and the only defence is a human at a
//! real desktop running `tests/MANUAL-TEST.md` 7.25m (TDD 7.25).** Said plainly because
//! the alternative is worse than silence: a reader told that a guard exists will not
//! write one. The unit tests below cover [`region_for`]'s arithmetic and nothing more —
//! they pass whether or not the region ever reaches an `NSTrackingArea`, because there is
//! no `gdk_surface_get_input_region` to read back and the effect is only observable as a
//! cursor changing shape on a live Quartz session. Xvfb cannot see it, the GTK suite
//! cannot see it, and neither can any other seat.
//!
//! Reported upstream as [GTK issue #6134][], mis-titled "without client-side decoration
//! (CSD)" — which is why it drew no fix, since `use_client_shadow` is FALSE for every
//! macOS window and the defect is universal there rather than a CSD edge case. That
//! issue was CLOSED on 2023-11-24 and **the closure reason is unread** — its notes need
//! a GNOME GitLab login this project does not have, so do not assume it was rejected,
//! and do not assume it was fixed either: the 0×0 tracking area is still there. Verified
//! present at 4.22.4 and unchanged through 4.23.2 and `main`; the original report was
//! 4.13.1 on macOS 13, this was measured on 4.22.4 on macOS 26/arm64, so it is gated by
//! neither version nor architecture.
//!
//! [GTK issue #6134]: https://gitlab.gnome.org/GNOME/gtk/-/issues/6134
//!
//! # Why `realize`, and why the whole surface
//!
//! At `realize` the surface exists and the window is not yet on screen. That ordering was
//! chosen because `-addTrackingArea:` is called without `NSTrackingAssumeInside`, so an
//! area installed while the pointer is *already* inside it may not post `mouseEntered:`
//! until the pointer next crosses in, and installing before the window is shown was meant
//! to keep that case from arising.
//!
//! ⚠ **That argument does not actually hold, and the seam does not depend on it.** The
//! region installed at `realize` is a 100×100 corner (next section); the one that matters
//! is installed by the first `layout`, which is not before the window is shown. So the
//! resident-pointer case is *not* dodged by ordering. Measured instead: with the full
//! remedy armed and the pointer parked INSIDE the map rect, the cursor applies, 3 of 3.
//! **That is a non-regression result and not proof of the mechanism** — the same
//! configuration also works with no input region at all, via the synthetic map-time enter,
//! so it cannot distinguish a tracking area that posted an enter from one that did not.
//! Whether `mouseEntered:` fires for a pointer already resident when the real region lands
//! is UNMEASURED; nothing here relies on it, because the map-time enter covers exactly
//! that case and it is the pointer-outside case that needs this module at all.
//!
//! The region is the surface's full extent because this seam is buying crossing events,
//! not shaping input — a smaller region would carve real holes in the window.
//!
//! # The `layout` re-application is LOAD-BEARING, not maintenance
//!
//! ⚠ **Deleting [`gdk::Surface::connect_layout`] below does not degrade this seam, it
//! DEFEATS it** — and the reason is not obvious from the code, which is why it is written
//! here. **At `realize` the surface has not been sized yet and reports a 100×100
//! placeholder**, measured in this application (`REALIZE surface=100x100`, then
//! `LAYOUT 1233x720`, then `LAYOUT 1292x720`) and reproduced in
//! `probes/macos-cursor-map-latch.c`. So the `realize`-time call installs a real tracking
//! area covering only the surface's top-left **100×100 corner**. The first `layout` pass
//! is what replaces it with one that covers the window.
//!
//! Measured on GTK 4.22.4 / Quartz with the pointer parked outside the map rect, three
//! trials per arm, hovering the middle of the window:
//!
//! ```text
//! no input region at all                     arrow    3 of 3
//! realize + layout (what this module does)   pointer  3 of 3
//! realize only, layout re-application cut    arrow    3 of 3
//! no input region at all, re-run             arrow    3 of 3
//! ```
//!
//! The corner is genuinely live, which is what makes the failure mode dangerous rather
//! than merely wrong. With the `layout` half cut, hovering **inside** that 100×100 corner
//! still yields `pointer` (2 of 2) while hovering outside it yields `arrow` (2 of 2) — same
//! binary, same arm, only the hover position differing. So a regression here produces a
//! window whose cursor works in one corner and nowhere else, and **any check that happens
//! to sample near the top-left origin passes**. `tests/MANUAL-TEST.md` 7.25m hovers body
//! text and a link well away from that corner, so it does catch this; a future check
//! written more conveniently might not.
//!
//! `gdk_surface_set_input_region` early-returns on an unchanged region
//! (`gdksurface.c:2062`), so the steady-state layout passes after the first cost nothing.
//! That cheapness is why the call is harmless to keep, NOT why it is there.

use gtk::prelude::*;
use gtk::{cairo, gdk};

/// Arm every toplevel, present and future, with a real tracking area.
///
/// Subscribes to the toplevel list GTK already maintains, the same mechanism
/// [`fullscreen::track_transient_windows`](super::fullscreen::track_transient_windows)
/// uses, so a window opened later in the session is covered too — the defect is per
/// `GdkSurface`, so a second window is its own throw of the dice and a fix that only
/// reached the first would leave every later one broken.
pub(crate) fn track_pointer_crossing() {
    let toplevels = gtk::Window::toplevels();
    for i in 0..toplevels.n_items() {
        if let Some(window) = toplevels.item(i).and_downcast::<gtk::Window>() {
            arm(&window);
        }
    }
    toplevels.connect_items_changed(|model, position, _removed, added| {
        for i in position..position + added {
            if let Some(window) = model.item(i).and_downcast::<gtk::Window>() {
                arm(&window);
            }
        }
    });
}

/// Install the tracking area at `window`'s realize, and keep it sized to the surface.
fn arm(window: &gtk::Window) {
    // A window already realized when this runs would never see the signal again, so it
    // is handled directly. `track_pointer_crossing` runs before the first window is
    // built, making this the empty case in practice — it is here so the function is
    // correct wherever it is called from, not because a caller needs it today.
    if window.is_realized() {
        install(window);
        return;
    }
    window.connect_realize(install);
}

/// Give `window`'s surface an input region covering the whole surface, and re-apply it
/// whenever the surface is laid out at a new size.
fn install(window: &gtk::Window) {
    let Some(surface) = window.native().and_then(|native| native.surface()) else {
        return;
    };
    apply(&surface, surface.width(), surface.height());
    surface.connect_layout(apply);
}

/// Set the surface's input region from [`region_for`], if that yields one.
fn apply(surface: &gdk::Surface, width: i32, height: i32) {
    if let Some(region) = region_for(width, height) {
        surface.set_input_region(&region);
    }
}

/// The region a surface of `width` × `height` should claim, or `None` to claim nothing.
///
/// The decision core, split out from [`apply`] so it can be tested without a display —
/// the GTK wiring around it cannot be, and a guard that needs a live surface would be
/// testing GDK rather than this choice.
///
/// A non-positive extent yields `None` rather than an empty region, and the distinction
/// is the whole point: an empty region IS the 0×0 tracking area this module exists to
/// replace (`GdkMacosBaseView.c:39-53`), so writing one would restore the defect while
/// looking like the fix.
fn region_for(width: i32, height: i32) -> Option<cairo::Region> {
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(cairo::Region::create_rectangle(&cairo::RectangleInt::new(
        0, 0, width, height,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extents_of(region: &cairo::Region) -> (i32, i32, i32, i32) {
        let extents = cairo::RectangleInt::new(0, 0, 0, 0);
        region.extents(&extents);
        (extents.x(), extents.y(), extents.width(), extents.height())
    }

    /// The region must cover the WHOLE surface from its origin, because its real job is
    /// to become the view's `NSTrackingArea` — a partial one would buy crossing events
    /// for part of the window and leave the rest reading an arrow.
    ///
    /// Mutation check: insetting the rectangle, or anchoring it anywhere but (0, 0),
    /// fails this.
    #[test]
    fn a_positive_surface_claims_its_whole_extent_from_the_origin() {
        let region = region_for(400, 300).expect("a positive size yields a region");
        assert_eq!(extents_of(&region), (0, 0, 400, 300));
    }

    /// A surface with no area yields NO region rather than an empty one.
    ///
    /// This is the case that matters most and it is easy to get wrong by "clamping":
    /// an empty region is not a harmless no-op, it is the exact broken state, so a
    /// clamp would silently re-arm the defect on any surface that reports a zero
    /// dimension mid-layout.
    ///
    /// Mutation check: dropping the guard makes `region_for` return an empty region
    /// here instead of `None`, and each of these fails.
    #[test]
    fn a_surface_with_no_area_claims_nothing() {
        assert!(region_for(0, 300).is_none(), "zero width");
        assert!(region_for(400, 0).is_none(), "zero height");
        assert!(region_for(0, 0).is_none(), "zero both");
        assert!(region_for(-1, 300).is_none(), "negative width");
        assert!(region_for(400, -1).is_none(), "negative height");
    }
}
