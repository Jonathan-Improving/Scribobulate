//! The pure decision core behind "collapse consecutive identical log records" —
//! POLICY § Logging, TDD 21.13/21.14.
//!
//! # Why this exists
//!
//! A test that hung inside widget dispose emitted the same `Gtk-WARNING` message
//! roughly 99 million times: 4.2 GB of captured output in about two minutes, with
//! most of the hang's wall-clock cost spent inside GLib's own log writer — the
//! flood was most of the hang, not a symptom running alongside it. That happened
//! in a test harness printing GLib's raw output; the same shape of flood, left
//! unchecked, would also fill the application's own persistent log and crowd the
//! crash-forensics breadcrumb ring past the context a crash report needs
//! (`forensics::ring`). The operator's decision was not to CAP output volume — a
//! byte cap can throw away the one record that matters — but to TRIM the specific
//! unhelpful pattern: once a `(level, domain, message)` has been shown, showing it
//! again teaches nothing new.
//!
//! # What "collapse" means
//!
//! - The FIRST occurrence of a key is never touched — every caller sees exactly
//!   what it always saw for a message that is not repeating.
//! - A REPEAT of the run currently in progress is suppressed, **except** at
//!   bounded milestones (10, 100, 1000, …) where a "previous message repeated N
//!   times" summary is produced instead of the raw repeat. A flood that never
//!   ends still shows growth — nothing here waits for the run to end before
//!   saying anything, and the milestone schedule is a genuine bound: eight
//!   milestones (10¹ through 10⁸) cover a hundred-million-repeat flood.
//! - When a DIFFERENT key arrives and the broken run's true count was never
//!   exactly reported by a milestone (e.g. it stopped at 7 repeats, or at 57), one
//!   final summary reports the true count — so a run that stops between
//!   milestones is never silently left at whatever the last milestone said.
//!
//! Key equality is **exact** on `(level, domain, message)` — a message differing
//! by even one byte starts its own run and is never folded into another's.
//!
//! # Two installations, one core
//!
//! [`RepeatCollapse`] holds no notion of *display*: it never formats a line for
//! output, never touches a file or stderr, and knows nothing about
//! `glib::log_writer_default` or the `log` crate. Two call sites drive it, each
//! deciding for itself what "display" and "record" mean:
//!
//! - `logging::forward` — the application's own glib→`log` bridge, so a flood
//!   cannot fill the persistent log or crowd the breadcrumb ring (TDD 21.14).
//! - `gtk_log_harness` — installed once for **both** GTK test harnesses (the
//!   libtest lib target, via `#[gtktest::test]`'s generated wrapper, and
//!   `gtk_suite.rs`'s main-thread runner), so a runaway widget-dispose flood
//!   cannot bury a test run or fill a CI runner's disk (TDD 21.13).
//!
//! # Thread safety
//!
//! GLib may invoke a log writer func from any thread. [`RepeatCollapse::record`]
//! is a single `Mutex`-guarded decision — safe to call concurrently from however
//! many threads are flooding it at once, and the lock is held only for the
//! handful of comparisons/increments in [`RepeatCollapse::record`], never across
//! any I/O.

use std::sync::Mutex;

/// What identifies "the same message" for collapsing purposes. Exact equality on
/// all three fields — POLICY's requirement that non-identical messages are never
/// folded into one another's run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key<L> {
    pub(crate) level: L,
    pub(crate) domain: String,
    pub(crate) message: String,
}

impl<L> Key<L> {
    pub(crate) fn new(level: L, domain: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level,
            domain: domain.into(),
            message: message.into(),
        }
    }
}

/// What a caller should do with the record it just handed to
/// [`RepeatCollapse::record`].
#[derive(Debug)]
pub(crate) enum Action {
    /// The first record of a new run — display/record it exactly as if this
    /// module did not exist.
    First,
    /// A repeat that just crossed a milestone (10, 100, 1000, …) — display/record
    /// a summary (see [`repeat_summary`]) in place of the raw repeat.
    Milestone(u64),
    /// A repeat that has not yet crossed a milestone — display/record nothing.
    Suppressed,
}

/// A run that just ended because a different key arrived, whose true final count
/// was never exactly reported by a milestone and is therefore still owed.
#[derive(Debug)]
pub(crate) struct ClosedRun<L> {
    pub(crate) key: Key<L>,
    pub(crate) count: u64,
}

/// The full verdict for one incoming record.
#[derive(Debug)]
pub(crate) struct Outcome<L> {
    /// The PRIOR run's belated final tally, if this record broke it and one is
    /// owed. Chronologically first — display/record this before `action`, since
    /// it describes events that happened before the record that closed them.
    pub(crate) closed: Option<ClosedRun<L>>,
    pub(crate) action: Action,
}

struct Run<L> {
    key: Key<L>,
    count: u64,
    /// The highest count already reported to a caller: 1 once the first
    /// occurrence has been shown (nothing has "repeated" yet), or a milestone
    /// value once one has fired. `count > reported_through` is exactly "this run
    /// owes a closing summary if it ends right now".
    reported_through: u64,
}

/// The collapsing decision core. Holds exactly one run at a time — the most
/// recent key and how many times it has repeated — so it is `O(1)` in time and
/// space regardless of flood size. Safe to share across threads.
pub(crate) struct RepeatCollapse<L> {
    state: Mutex<Option<Run<L>>>,
}

impl<L: Clone + PartialEq> RepeatCollapse<L> {
    pub(crate) const fn new() -> Self {
        Self {
            state: Mutex::new(None),
        }
    }

    /// Feed one record in, get back what the caller should do about it.
    ///
    /// A poisoned mutex (a prior panic while the lock was held elsewhere in the
    /// process) still yields a decision rather than propagating the panic — a
    /// diagnostic aid must never itself bring the process down, the same
    /// discipline `forensics::sink` applies to its own file mutex.
    pub(crate) fn record(&self, key: Key<L>) -> Outcome<L> {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(run) = guard.as_mut() {
            if run.key == key {
                run.count += 1;
                return if is_milestone(run.count) {
                    run.reported_through = run.count;
                    Outcome {
                        closed: None,
                        action: Action::Milestone(run.count),
                    }
                } else {
                    Outcome {
                        closed: None,
                        action: Action::Suppressed,
                    }
                };
            }
        }
        // Either no run was in progress, or a DIFFERENT key just arrived and
        // broke the one that was — in both cases `key` starts a fresh run, and
        // in the second case the old run may still owe a closing summary.
        let closed = guard.take().and_then(|run| {
            (run.count > run.reported_through).then(|| ClosedRun {
                key: run.key,
                count: run.count,
            })
        });
        *guard = Some(Run {
            key,
            count: 1,
            reported_through: 1,
        });
        Outcome {
            closed,
            action: Action::First,
        }
    }
}

/// A count is a milestone at every power of ten from 10 up — bounded output no
/// matter how large a flood gets (eight milestones, 10¹ through 10⁸, cover a
/// hundred-million-repeat run), and growth is visible without waiting for the
/// run to end.
fn is_milestone(count: u64) -> bool {
    if count < 10 {
        return false;
    }
    let mut n = count;
    while n.is_multiple_of(10) {
        n /= 10;
    }
    n == 1
}

/// The one wording every milestone and closing summary uses, in both
/// installations, so a reader who has seen this string once recognises it
/// everywhere. Embeds the original message text (not just its count) so a
/// summary line is still useful on its own if the log is later truncated or
/// rotated (TDD 21.6), and so the SAME benign-noise substring check that
/// demotes the original message (`logging::is_benign_gtk_startup_noise`) also
/// recognises its own summary — that check matches on message *content*, and the
/// content is still present here.
pub(crate) fn repeat_summary(domain: &str, message: &str, count: u64) -> String {
    format!("{domain}: previous message repeated {count} times: {message}")
}

/// Pull `GLIB_DOMAIN` and `MESSAGE` out of a glib structured-log field set —
/// shared by `logging::forward` and `gtk_log_harness::writer` so the parsing
/// itself cannot drift between the two installations. `"glib"` is glib's own
/// fallback domain for a record that did not name one.
pub(crate) fn extract_domain_message(fields: &[glib::LogField<'_>]) -> (String, String) {
    let mut domain = "glib".to_owned();
    let mut message = String::new();
    for f in fields {
        match f.key() {
            "MESSAGE" => message = f.value_str().unwrap_or_default().to_owned(),
            "GLIB_DOMAIN" => domain = f.value_str().unwrap_or("glib").to_owned(),
            _ => {}
        }
    }
    (domain, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal level type for these tests — the decision core is generic over
    /// `L`, so it needs no glib/log dependency to unit-test at all.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TestLevel(u8);

    fn key(level: u8, domain: &str, message: &str) -> Key<TestLevel> {
        Key::new(TestLevel(level), domain, message)
    }

    #[test]
    fn the_first_occurrence_is_never_touched() {
        let c = RepeatCollapse::new();
        let outcome = c.record(key(1, "Gtk", "boom"));
        assert!(outcome.closed.is_none());
        assert!(matches!(outcome.action, Action::First));
    }

    #[test]
    fn identical_repeats_are_suppressed_until_the_first_milestone() {
        let c = RepeatCollapse::new();
        c.record(key(1, "Gtk", "boom")); // count 1, First
        for n in 2..=9 {
            let outcome = c.record(key(1, "Gtk", "boom"));
            assert!(matches!(outcome.action, Action::Suppressed), "count {n}");
        }
        let outcome = c.record(key(1, "Gtk", "boom"));
        assert!(matches!(outcome.action, Action::Milestone(10)));
    }

    #[test]
    fn milestones_land_on_every_power_of_ten_and_nowhere_else() {
        let c = RepeatCollapse::new();
        c.record(key(1, "Gtk", "boom")); // count 1
        let mut milestones = Vec::new();
        for _ in 2..=1000 {
            if let Action::Milestone(n) = c.record(key(1, "Gtk", "boom")).action {
                milestones.push(n);
            }
        }
        assert_eq!(milestones, vec![10, 100, 1000]);
    }

    #[test]
    fn is_milestone_fires_at_every_power_of_ten_only() {
        for &n in &[
            10,
            100,
            1_000,
            10_000,
            100_000,
            1_000_000,
            10_000_000,
            100_000_000,
        ] {
            assert!(is_milestone(n), "{n} must be a milestone");
        }
        for &n in &[0, 1, 9, 11, 20, 99, 101, 999, 1_001, 999_999_999] {
            assert!(!is_milestone(n), "{n} must not be a milestone");
        }
    }

    #[test]
    fn a_run_that_never_hit_a_milestone_reports_its_true_count_when_it_breaks() {
        let c = RepeatCollapse::new();
        c.record(key(1, "Gtk", "boom")); // count 1
        for _ in 0..6 {
            c.record(key(1, "Gtk", "boom")); // count reaches 7 — never a milestone (< 10)
        }
        let outcome = c.record(key(1, "Gtk", "a different message"));
        let closed = outcome.closed.expect("the 7-repeat run owed a final tally");
        assert_eq!(closed.count, 7);
        assert_eq!(closed.key.message, "boom");
        assert!(matches!(outcome.action, Action::First));
    }

    #[test]
    fn a_run_that_ended_exactly_on_a_milestone_owes_nothing_more() {
        let c = RepeatCollapse::new();
        c.record(key(1, "Gtk", "boom")); // count 1
        for _ in 0..9 {
            c.record(key(1, "Gtk", "boom")); // count reaches 10 — Milestone(10) already shown
        }
        let outcome = c.record(key(1, "Gtk", "a different message"));
        assert!(
            outcome.closed.is_none(),
            "the exact count was already shown by the milestone — a second, identical \
             closing line would be a duplicate, not new information"
        );
    }

    #[test]
    fn a_message_that_never_repeated_owes_nothing_when_it_breaks() {
        let c = RepeatCollapse::new();
        c.record(key(1, "Gtk", "boom"));
        let outcome = c.record(key(1, "Gtk", "a different message"));
        assert!(outcome.closed.is_none());
    }

    #[test]
    fn non_identical_messages_are_never_collapsed() {
        let c = RepeatCollapse::new();
        assert!(matches!(c.record(key(1, "Gtk", "a")).action, Action::First));
        assert!(matches!(c.record(key(1, "Gtk", "b")).action, Action::First));
        assert!(
            matches!(c.record(key(1, "Gtk", "a")).action, Action::First),
            "\"a\" differs from the run currently in progress (\"b\") — key equality is exact, \
             not \"seen at some point\""
        );
    }

    #[test]
    fn key_equality_is_exact_on_level_and_domain_too() {
        let c = RepeatCollapse::new();
        c.record(key(1, "Gtk", "boom"));
        assert!(
            matches!(c.record(key(2, "Gtk", "boom")).action, Action::First),
            "a different level starts its own run"
        );
        c.record(key(1, "Gtk", "boom"));
        assert!(
            matches!(c.record(key(1, "Gdk", "boom")).action, Action::First),
            "a different domain starts its own run"
        );
    }

    #[test]
    fn repeat_summary_names_the_domain_count_and_original_message() {
        let s = repeat_summary("Gtk", "boom", 42);
        assert_eq!(s, "Gtk: previous message repeated 42 times: boom");
    }

    /// The concurrency-safe wrapper, proven rather than assumed: many threads
    /// hammering the SAME key must serialise through one mutex so that no
    /// increment is lost and no milestone value is ever reported twice or
    /// skipped — the property every other test above assumes single-threaded
    /// access to establish.
    #[test]
    fn concurrent_repeats_of_the_same_key_are_serialized_without_lost_or_duplicate_milestones() {
        use std::sync::Arc;

        let c = Arc::new(RepeatCollapse::new());
        // Establish the run single-threaded first, so every subsequent call from
        // every worker is unambiguously a "repeat" and the expected milestone set
        // below is exact.
        assert!(matches!(
            c.record(key(1, "Gtk", "flood")).action,
            Action::First
        ));

        const THREADS: usize = 8;
        const PER_THREAD: usize = 2_500; // 20,000 repeats total, plus the 1 above
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let c = Arc::clone(&c);
                std::thread::spawn(move || {
                    let mut milestones = Vec::new();
                    for _ in 0..PER_THREAD {
                        if let Action::Milestone(n) = c.record(key(1, "Gtk", "flood")).action {
                            milestones.push(n);
                        }
                    }
                    milestones
                })
            })
            .collect();

        let mut all_milestones: Vec<u64> = handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker thread completes without panicking"))
            .collect();
        all_milestones.sort_unstable();
        // Exactly one thread ever observes each milestone value — a lost update
        // would skip one (fewer than 4 entries); a torn read/write would let two
        // threads both observe e.g. count == 10 (a duplicate entry).
        assert_eq!(
            all_milestones,
            vec![10, 100, 1_000, 10_000],
            "each milestone must be reported exactly once across every thread"
        );
    }

    #[test]
    fn extract_domain_message_reads_both_fields_and_falls_back_to_glib() {
        use glib::gstr;

        let with_domain = [
            glib::LogField::new(gstr!("GLIB_DOMAIN"), b"Gtk"),
            glib::LogField::new(gstr!("MESSAGE"), b"boom"),
        ];
        assert_eq!(
            extract_domain_message(&with_domain),
            ("Gtk".to_owned(), "boom".to_owned())
        );

        let without_domain = [glib::LogField::new(gstr!("MESSAGE"), b"boom")];
        assert_eq!(
            extract_domain_message(&without_domain),
            ("glib".to_owned(), "boom".to_owned())
        );
    }
}
