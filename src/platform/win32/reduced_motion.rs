//! The system "reduce animations" preference — the source GTK does not supply on
//! Windows.
//!
//! **`GtkSettings:gtk-enable-animations` has no Windows source at our floor.**
//! `gdk_win32_display_get_setting` (`gdk/win32/gdkwin32misc.c`) has no arm for
//! `gtk-enable-animations`, so the property keeps its compiled-in default of `TRUE`
//! regardless of what the user has asked Windows for. This module supplies that
//! missing source directly to [`crate::animation::policy`], which is the one place
//! that decides whether an animation may play — this module owns no such decision
//! itself (POLICY § Platform seams).
//!
//! **The right setting, and why not its neighbour.** `SPI_GETCLIENTAREAANIMATION`
//! is what Chromium's `animation_win.cc` and WPF's
//! `SystemParameters.ClientAreaAnimation` both read for "reduce motion"; the
//! similarly-named `SPI_GETANIMATION` (`ANIMATIONINFO.iMinAnimate`) is a different,
//! older setting (window minimize/restore animation) and is deliberately not read
//! here.
//!
//! **Why this polls, exactly like `appearance::track_system_dark_mode`.** The
//! precise signal would be `WM_SETTINGCHANGE`, but receiving it means owning a
//! window procedure, and GDK owns the toplevel's. This app runs entirely on the
//! GLib main loop with no worker threads (TECH.md), so it polls at the same
//! two-second cadence `appearance.rs` already established for the identical
//! constraint rather than inventing a second one for the same reason.

use gtk::glib;
use std::ffi::c_void;

#[link(name = "user32")]
extern "system" {
    /// `SystemParametersInfoW`, `winuser.h` —
    /// `BOOL SystemParametersInfoW(UINT uiAction, UINT uiParam, PVOID pvParam, UINT fWinIni);`.
    /// For a `SPI_GET*` action, `pvParam` is an out-pointer and `uiParam`/`fWinIni` are
    /// unused. The doc sits on the item, not the block: rustdoc ignores an `extern`
    /// block, so a `///` there trips `unused_doc_comments` under `-D warnings`.
    fn SystemParametersInfoW(action: u32, param: u32, data: *mut c_void, flags: u32) -> i32;
}

/// `SPI_GETCLIENTAREAANIMATION`, `winuser.h`. Documented value `0x1042`; retrieves a
/// `BOOL` — nonzero means client-area animations (the setting Chromium and WPF both
/// key "reduce motion" off) are enabled.
const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;

/// How often to re-read the setting. Identical in value to `appearance.rs`'s own
/// theme-poll interval and for the identical reason (GDK owns the toplevel's
/// `WndProc`, so `WM_SETTINGCHANGE` is not ours to filter) — kept as its own
/// constant because the two causes are independent (POLICY's "children split by
/// cause" rule for this directory) even though the number matches today.
const REDUCED_MOTION_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether Windows is currently set to reduce client-area animation, or `None` if
/// the setting cannot be read.
///
/// `0` means REDUCED (client-area animation disabled) — this is the polarity
/// `SPI_GETCLIENTAREAANIMATION` documents and the one Chromium/WPF both read.
fn windows_reduced_motion() -> Option<bool> {
    let mut enabled: i32 = 0;
    // SAFETY: `enabled` is a live, correctly-sized (`BOOL` is 4 bytes) destination
    // for the duration of the call; `uiParam` and `fWinIni` are unused for a
    // `SPI_GET*` action per the documented contract.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            std::ptr::addr_of_mut!(enabled).cast::<c_void>(),
            0,
        )
    };
    (ok != 0).then_some(enabled == 0)
}

/// The current value, as [`crate::platform::system_reduced_motion`] reports it for
/// this platform.
pub(crate) fn system_reduced_motion() -> Option<bool> {
    windows_reduced_motion()
}

/// Subscribe to the setting changing, as
/// [`crate::platform::watch_reduced_motion`] reports it for this platform. Polls at
/// [`REDUCED_MOTION_POLL_INTERVAL`]; the returned guard removes the GLib timeout
/// source on drop, so a caller that stops watching does not leave a tick running
/// forever the way `appearance::track_system_dark_mode` deliberately does for its
/// own, never-torn-down, startup-only subscription.
pub(crate) fn watch_reduced_motion(
    f: impl Fn(Option<bool>) + 'static,
) -> crate::platform::ReducedMotionWatch {
    let mut last = windows_reduced_motion();
    let source_id = glib::timeout_add_local(REDUCED_MOTION_POLL_INTERVAL, move || {
        let now = windows_reduced_motion();
        if now != last {
            last = now;
            f(now);
        }
        glib::ControlFlow::Continue
    });
    crate::platform::ReducedMotionWatch::new(move || source_id.remove())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the POLARITY of `SPI_GETCLIENTAREAANIMATION`, which is the one thing
    /// here that is easy to get exactly backwards and impossible to notice in
    /// review: the raw value is "client-area animation enabled", so **0 means
    /// reduced motion**. Inverting it would make the app animate when the user
    /// asked for less motion and freeze when they asked for none, both silently.
    ///
    /// Reads the raw flag independently and asserts the mapping against it, so the
    /// test follows the machine's real setting instead of assuming one.
    #[test]
    fn reduced_is_the_inverse_of_client_area_animation_enabled() {
        let mut raw: i32 = -1;
        // SAFETY: as in `windows_reduced_motion` — a live 4-byte destination and
        // unused `uiParam`/`fWinIni`.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                std::ptr::addr_of_mut!(raw).cast::<c_void>(),
                0,
            )
        };

        if ok != 0 {
            assert_eq!(
                windows_reduced_motion(),
                Some(raw == 0),
                "ClientAreaAnimation={raw}: 0 must map to reduced motion, nonzero to not-reduced",
            );
        } else {
            assert_eq!(
                windows_reduced_motion(),
                None,
                "an unreadable setting must be None, never a guessed value",
            );
        }
    }
}
