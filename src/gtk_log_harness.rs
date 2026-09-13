//! The GTK test harnesses' own glib log writer — collapses a flood of identical
//! records so it cannot bury a run or fill a CI runner's disk, without touching
//! the format either harness relies on (TDD 21.13).
//!
//! # Why this is not `logging::forward`
//!
//! `gtk_suite.rs`'s `main()` deliberately does **not** call `logging::init()`:
//! that installs the glib→`log` bridge, which *reformats* GLib's own output
//! (`Gtk-WARNING **:` becomes `(Gtk) Warning:`), and both `#[gtktest::test]`
//! bodies and `tests/MANUAL-TEST.md` grep the literal GLib token. So the harness
//! writer here must print the FIRST occurrence of a record exactly as GLib's own
//! default writer would — it delegates to [`glib::log_writer_default`] rather
//! than formatting anything itself — and collapse only the repeats. This module
//! shares [`crate::logrepeat`]'s decision core with `logging::forward`, but is
//! its own, much thinner installation: no forensic ring, no persistent log, no
//! demotion — just "print the first, collapse the rest".
//!
//! # Installed once, for both GTK harnesses
//!
//! `glib::log_set_writer_func` panics if called a second time in one process, so
//! [`install_once`] is idempotent via a [`std::sync::OnceLock`] and is safe to
//! call from every entry point that might be first:
//!
//! - `gtk_suite.rs`'s `main()` calls it once, directly, right after `gtk::init()`
//!   — the main-thread suite's own one-time init point.
//! - The libtest lib harness has no equivalent single init point of its own:
//!   `#[gtk::test]` (what `#[gtktest::test]` expands its libtest half into)
//!   calls `gtk::init()` inside gtk4-rs's own generated wrapper, on a
//!   `glib::ThreadPool` worker, with nothing in this crate able to hook it.
//!   `#[gtktest::test]`'s generated wrapper is the one thing every libtest GTK
//!   body passes through, so it calls [`install_once`] as its first statement —
//!   cheap (one atomic load) on every call after the first, and it guarantees
//!   the writer is installed before the *first* GTK body in the binary runs,
//!   regardless of which body that turns out to be.
//!
//! Both installations end up sharing the process's one glib writer func, so a
//! flood in ANY test, under EITHER harness, is bounded the same way.
//!
//! # Which levels are collapsed
//!
//! Only `Error`, `Critical`, `Warning` and `Message` — the levels
//! `g_log_writer_default` always prints regardless of `G_MESSAGES_DEBUG`, and
//! (not incidentally) the ones it always sends to stderr, so a milestone/closing
//! summary printed here via `eprintln!` lands in the same stream as the
//! delegated first occurrence. `Info`/`Debug` visibility is gated by
//! `G_MESSAGES_DEBUG` *inside* GLib itself — collapsing them would either
//! manufacture new visible output for a level nothing here decided to show, or
//! silently start counting invisible spam into a milestone nobody would ever
//! see reach it (POLICY § Logging's "must not be counted in a misleading way").
//! Those two levels are passed straight through, uncollapsed, exactly as if this
//! writer did not exist.

use glib::LogLevel::{Critical, Error, Message, Warning};

use crate::logrepeat;

/// The process-wide collapse state for this harness writer — a fresh instance
/// from `logging::forward`'s, since they run in different processes/binaries.
static COLLAPSE: logrepeat::RepeatCollapse<glib::LogLevel> = logrepeat::RepeatCollapse::new();

/// Install [`writer`] as the process's glib log writer func, exactly once.
///
/// Safe to call from every GTK test entry point (see the module docs) — the
/// second and later calls are a single atomic load and do nothing.
pub(crate) fn install_once() {
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| {
        glib::log_set_writer_func(writer);
    });
}

/// The installed writer. See the module docs for the design; this function is
/// deliberately thin — extract, collapse, dispatch — with no logic of its own
/// beyond the level gate.
fn writer(level: glib::LogLevel, fields: &[glib::LogField<'_>]) -> glib::LogWriterOutput {
    if !matches!(level, Error | Critical | Warning | Message) {
        return glib::log_writer_default(level, fields);
    }

    let (domain, message) = logrepeat::extract_domain_message(fields);
    let outcome = COLLAPSE.record(logrepeat::Key::new(level, domain.clone(), message.clone()));
    if let Some(closed) = outcome.closed {
        eprintln!(
            "{}",
            logrepeat::repeat_summary(&closed.key.domain, &closed.key.message, closed.count)
        );
    }
    match outcome.action {
        // The one call that must reproduce GLib's own formatting exactly — every
        // caller of this module relies on it being byte-identical to what ran
        // before this writer existed.
        logrepeat::Action::First => glib::log_writer_default(level, fields),
        logrepeat::Action::Milestone(count) => {
            eprintln!("{}", logrepeat::repeat_summary(&domain, &message, count));
            glib::LogWriterOutput::Handled
        }
        logrepeat::Action::Suppressed => glib::LogWriterOutput::Handled,
    }
}
