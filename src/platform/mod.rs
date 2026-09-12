//! Per-platform seams, one module per target OS — a directory where the platform
//! needs more than one file, a single file where it does not.
//!
//! Each child is `#[cfg]`-gated **at its declaration** here and never internally,
//! so a build for another platform compiles none of it and the `-D warnings`
//! clippy gate stays satisfiable with no `#[allow]` (the same rule `workaround`
//! follows).
//!
//! Declaring them here rather than at the crate root is what makes that gating a
//! single fact: `lib.rs` and `gtk_suite.rs` are two crate roots that must otherwise
//! re-declare every module, and a platform module missing from the second silently
//! drops its whole test surface from the main-thread suite.
//!
//! The bar for putting something here is narrow: it must be work the toolkit does
//! for us elsewhere and does not do on this platform. Anything that changes what
//! the app *does* belongs in the shared code, behind a capability, so the
//! platforms cannot drift in behaviour — only in the plumbing underneath.

#[cfg(target_os = "macos")]
pub(crate) mod mac;

#[cfg(windows)]
pub(crate) mod win32;

// ── the system "reduce animations" preference ───────────────────────────────────
//
// A second façade beside the per-platform modules above, living here rather than in
// either child, because it has THREE arms rather than two: Windows has a source
// (`win32::reduced_motion`), macOS has one too but wiring it is deliberately
// deferred (it shares the no-source arm below), and every other platform — Linux included,
// where `GtkSettings:gtk-enable-animations` already carries the desktop's answer —
// has none at all. `animation::policy` is the one caller and the one place that
// turns this into a decision (POLICY § Platform seams: a seam supplies a source and
// owns no behaviour).

/// A live subscription to [`system_reduced_motion`] changing. Dropping it
/// unsubscribes. Each platform supplies its own teardown — a GLib timeout source
/// removed on Windows, nothing to do where there is no source at all — behind one
/// droppable type so `animation::policy::watch` holds a single guard regardless of
/// platform, exactly as it already does for the reader's own choice and for
/// `gtk-enable-animations`.
pub(crate) struct ReducedMotionWatch(Option<Box<dyn FnOnce()>>);

impl ReducedMotionWatch {
    /// Build a guard whose `teardown` runs once, on drop.
    fn new(teardown: impl FnOnce() + 'static) -> Self {
        Self(Some(Box::new(teardown)))
    }

    /// A subscription over nothing: never fires, nothing to unregister. The source
    /// half for every platform with no source of its own (Linux, and macOS while its
    /// module is deferred). `#[cfg]`-gated to exactly those platforms so the Windows
    /// build, which never calls it, has no dead-code warning under `-D warnings`.
    #[cfg(not(windows))]
    fn noop() -> Self {
        Self(None)
    }
}

impl Drop for ReducedMotionWatch {
    fn drop(&mut self) {
        if let Some(teardown) = self.0.take() {
            teardown();
        }
    }
}

/// Whether the OS is currently asking for reduced motion, or `None` if this
/// platform has no source of its own to ask. `Some(true)` means the system wants
/// LESS animation; `Some(false)` that it explicitly does not; `None` must never be
/// treated as either — a caller that cannot tell must leave its other inputs
/// standing rather than guess (mirrors how `animation::policy::enabled_of` treats a
/// missing `GtkSettings` as GTK's own default rather than as a guessed answer).
///
/// - **Windows**: `SPI_GETCLIENTAREAANIMATION` — see
///   `platform::win32::reduced_motion` for the exact API and why not its neighbour
///   `SPI_GETANIMATION`.
/// - **macOS**: a real source exists —
///   `[[NSWorkspace sharedWorkspace] accessibilityDisplayShouldReduceMotion]`, with
///   change notification `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification`
///   posted on `[NSWorkspace sharedWorkspace].notificationCenter` — but wiring it is
///   deliberately DEFERRED: no seat can compile or verify Objective-C runtime FFI
///   for it right now. Until it lands, macOS answers `None` here exactly like a
///   platform with no source at all, so `gtk-enable-animations` stays hardcoded
///   `TRUE` there and nothing about this platform's behaviour changes yet.
/// - **Linux and everything else**: `None`, always — `GtkSettings` already carries
///   the desktop's answer, so there is nothing for this façade to add.
pub(crate) fn system_reduced_motion() -> Option<bool> {
    platform_reduced_motion()
}

/// Subscribe to [`system_reduced_motion`] changing. `f` is called with the NEW
/// value whenever the platform's source changes; dropping the returned guard
/// unsubscribes. On a platform with no source (Linux, and macOS until its module
/// lands — see [`system_reduced_motion`]), the source half never fires: `f` is
/// still accepted rather than refused, so a caller never needs a platform match of
/// its own.
///
/// On EVERY platform a subscription has two halves: the platform's own source, and
/// an entry in [`listeners`]. The entry lets a test signal a change without a real
/// OS setting to flip, through the same façade on Windows, macOS and Linux. That way
/// a test of a caller's subscription compiles and runs on every platform instead of
/// only one (POLICY: never `#[cfg(platform)]` a test). Dropping the guard tears down
/// both halves.
pub(crate) fn watch_reduced_motion(f: impl Fn(Option<bool>) + 'static) -> ReducedMotionWatch {
    let f: listeners::Listener = std::rc::Rc::new(f);
    listeners::add(&f);
    let source = platform_watch_reduced_motion({
        let f = f.clone();
        move |value| f(value)
    });
    ReducedMotionWatch::new(move || {
        drop(source);
        listeners::remove(&f);
    })
}

#[cfg(windows)]
fn platform_reduced_motion() -> Option<bool> {
    win32::system_reduced_motion()
}

#[cfg(windows)]
fn platform_watch_reduced_motion(f: impl Fn(Option<bool>) + 'static) -> ReducedMotionWatch {
    win32::watch_reduced_motion(f)
}

// Every platform with no source of its own: Linux, anything else that is not
// Windows, and macOS. macOS does have a real source, but wiring it is deliberately
// deferred (see `system_reduced_motion`'s doc comment above for the exact API and
// notification name) because no seat can compile or verify Objective-C runtime FFI
// for it right now. When the macOS module lands, macOS gets its own arm, as Windows
// already has.
#[cfg(not(windows))]
fn platform_reduced_motion() -> Option<bool> {
    None
}

#[cfg(not(windows))]
fn platform_watch_reduced_motion(f: impl Fn(Option<bool>) + 'static) -> ReducedMotionWatch {
    let _ = f;
    ReducedMotionWatch::noop()
}

/// Every live [`watch_reduced_motion`] subscriber, on every platform. Production code
/// only adds and removes entries; the one thing that fires them is
/// [`testing::inject_reduced_motion`]. No host can produce a real Windows poll tick or
/// macOS notification from inside a test, so a test drives this list instead. That
/// proves a caller really subscribed through the façade and re-fires when a change
/// arrives. It does not test any platform's OS mechanism; each platform seat verifies
/// that separately (POLICY: verification is per-platform).
mod listeners {
    use std::cell::RefCell;
    use std::rc::Rc;

    /// One registered subscriber. Named so `LISTENERS` below reads as a plain
    /// collection rather than tripping `clippy::type_complexity`.
    pub(super) type Listener = Rc<dyn Fn(Option<bool>)>;

    thread_local! {
        static LISTENERS: RefCell<Vec<Listener>> = RefCell::new(Vec::new());
    }

    pub(super) fn add(f: &Listener) {
        LISTENERS.with(|l| l.borrow_mut().push(f.clone()));
    }

    pub(super) fn remove(f: &Listener) {
        LISTENERS.with(|l| l.borrow_mut().retain(|other| !Rc::ptr_eq(other, f)));
    }

    /// TEST-ONLY: call every live subscriber with `value`, as a real Windows poll
    /// tick (or a future macOS notification callback) would when the OS setting
    /// changes. It deliberately does NOT change what [`super::system_reduced_motion`]
    /// returns. A test that uses this is testing the subscription plumbing, not a
    /// made-up value that the production build could never return.
    ///
    /// `pub(crate)`, not `pub(super)` like its neighbours: it is re-exported through
    /// [`super::testing`] for `animation::policy`'s tests, which live outside
    /// `platform` entirely. A re-export can never widen an item's own visibility.
    /// It uses the same cfg as its callers rather than a bare `#[cfg(test)]` (POLICY §
    /// GTK-object integration tests).
    #[cfg(all(test, feature = "gtk-integration-tests"))]
    pub(crate) fn inject(value: Option<bool>) {
        let snapshot: Vec<_> = LISTENERS.with(|l| l.borrow().clone());
        for f in snapshot {
            f(value);
        }
    }
}

/// Test-only seam onto [`listeners::inject`], so `animation::policy`'s own tests can
/// signal a reduced-motion change without reaching into a private sibling module.
/// See [`listeners`]'s doc comment for why simulating the SUBSCRIPTION firing is the
/// honest test. Present on every platform, like its callers.
#[cfg(all(test, feature = "gtk-integration-tests"))]
pub(crate) mod testing {
    pub(crate) use super::listeners::inject as inject_reduced_motion;
}
