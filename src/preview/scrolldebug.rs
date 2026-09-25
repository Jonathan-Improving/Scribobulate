//! Diagnostic instrumentation for issue #3 (the preview drawn horizontally
//! scrolled after a mode switch or Reload). `log::trace!`-gated (never compiled
//! out, per `sdd/PLAN.profiling.md`'s T0 doctrine of app-owned instrumentation
//! over a `GTK_DEBUG` channel — those are confirmed dark on the reference
//! host), so it costs nothing at the default log level and can be turned on
//! with `RUST_LOG=trace` (or `scribobulate::hscroll=trace`) against a live
//! session without a rebuild.
//!
//! Every log line here carries the same `target: "scribobulate::hscroll"` so a
//! capture can be filtered to just this investigation, and every line names the
//! axis explicitly (`h` for horizontal, `v` for vertical) since the whole
//! question is whether a write ever reaches the axis nobody meant to touch.
//!
//! The regression coverage this wiring supports lives in `src/window/reload.rs`'s
//! `hscroll_issue_u` module; that module's doc comment records the narrowing
//! this investigation reached (macOS/GTK 4.22.4/Quartz: no reproducible write),
//! and ScrAP-358 records two harness-discipline traps hit while establishing it.

use gtk::prelude::*;

/// Wire `notify::upper` and `notify::value` logging on both of `sw`'s adjustments,
/// for the lifetime of `sw`'s current adjustments (a fresh mount gets fresh
/// adjustments, so this must be called at every mount — see
/// `SplitView::set_preview`, the one choke point every preview mount passes
/// through).
///
/// This is Step 1 of the issue-U investigation: instrument before comparing, and
/// prove the instrument emits (against a known-shifted and a known-normal
/// capture) before trusting its silence on any other run.
pub(crate) fn wire_adjustment_trace(sw: &gtk::ScrolledWindow, mount_id: u64) {
    for (axis, adj) in [("h", sw.hadjustment()), ("v", sw.vadjustment())] {
        adj.connect_notify_local(
            Some("upper"),
            glib::clone!(
                #[strong]
                axis,
                move |adj, _| {
                    log::trace!(
                        target: "scribobulate::hscroll",
                        "mount={mount_id} axis={axis} notify::upper upper={:.2} page_size={:.2} value={:.2}",
                        adj.upper(), adj.page_size(), adj.value(),
                    );
                }
            ),
        );
        adj.connect_notify_local(
            Some("value"),
            glib::clone!(
                #[strong]
                axis,
                move |adj, _| {
                    log::trace!(
                        target: "scribobulate::hscroll",
                        "mount={mount_id} axis={axis} notify::value upper={:.2} page_size={:.2} value={:.2}",
                        adj.upper(), adj.page_size(), adj.value(),
                    );
                }
            ),
        );
    }
}

/// Monotonic identifier for "this specific preview instance's first allocation",
/// so a capture can be tied to one mount rather than treated as one continuous
/// timeline across many rebuilds in a session (Step 1's fourth bullet). Not a
/// widget pointer/debug id — those are reused across mounts in a way a log reader
/// has to cross-reference by hand; a simple incrementing counter reads directly.
pub(crate) fn next_mount_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Log a single labelled snapshot of `sw`'s hadjustment, at a named point in a
/// restore call — the "immediately before / immediately after" pairs Step 1's
/// second and third bullets ask for around `scroll_to_mark` and
/// `saferizer::scrollpos::jump`.
pub(crate) fn log_hadj_snapshot(where_: &str, mount_id: u64, sw: &gtk::ScrolledWindow) {
    let adj = sw.hadjustment();
    log::trace!(
        target: "scribobulate::hscroll",
        "mount={mount_id} at={where_} axis=h upper={:.2} page_size={:.2} value={:.2}",
        adj.upper(), adj.page_size(), adj.value(),
    );
}

/// The same snapshot, both axes at once — used at the mount point itself, where
/// there is no single "restore call" to bracket.
pub(crate) fn log_both_snapshot(where_: &str, mount_id: u64, sw: &gtk::ScrolledWindow) {
    for (axis, adj) in [("h", sw.hadjustment()), ("v", sw.vadjustment())] {
        log::trace!(
            target: "scribobulate::hscroll",
            "mount={mount_id} at={where_} axis={axis} upper={:.2} page_size={:.2} value={:.2}",
            adj.upper(), adj.page_size(), adj.value(),
        );
    }
}
