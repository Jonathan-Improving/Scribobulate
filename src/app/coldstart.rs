//! "Is this activation the process's cold start?" — claimed once, synchronously.
//!
//! Its own module, and a measured one, because it is a pure decision that the two
//! GApplication entry points share: `setup::on_activate` (a bare launch) and
//! `openbatch::on_open` (a launch carrying file arguments). Both of those files are
//! GTK wiring and sit outside the coverage gate; the rule below is not wiring, and
//! the defect it replaces was a rule nobody could test.

/// Whether this activation is the process's cold start — claimed **synchronously**,
/// and `true` for exactly one caller per process.
///
/// # Why a latch and not `app.windows().is_empty()`
///
/// That predicate was read as "has this process started up yet", justified by TDD 8.3:
/// a running process can never reach zero windows, so zero windows means brand new.
/// True of a process in *steady state* — and startup is the one interval that premise
/// does not cover. The answer was decided synchronously and then falsified
/// asynchronously, by the very work it authorised.
///
/// Session restore reads each tab's document off the main thread, one at a time. For
/// as long as that takes, no window exists yet. A second bare launch arriving inside
/// that window is handed to the running instance as `activate`, still sees zero
/// windows, and **restores the entire session a second time**: every document in two
/// tabs, each with its own baseline, file monitor and swapfile, so a save in one raises
/// an external-change decision in the other. Crash recovery ran twice for the same
/// reason.
///
/// It is reachable precisely when it hurts. Restore is slowest on slow storage or a
/// large session, and the symptom of a slow start is *nothing happening* — which is
/// what makes a person click the launcher again.
///
/// Every entry point that can start a cold process claims through here, so the claim
/// and the work it authorises cannot be separated by an await.
pub(super) fn claim() -> bool {
    CLAIMED.with(Claim::take)
}

thread_local! {
    /// Main-thread-only: both callers are GApplication signal handlers, which GTK
    /// emits on the main thread. A process starts up once, so production never resets
    /// it — but a TEST BINARY is one process running many launches, which is what
    /// [`ClaimResetGuard`] exists for.
    static CLAIMED: Claim = const { Claim::new() };
}

/// Put the latch into the state this test requires, and restore it on drop.
///
/// **The production latch is deliberately unresettable, and that is what broke the
/// suite.** Every `#[gtktest::test]` runs on the single main thread of one binary, so
/// the first test to drive `activate` or `open` claimed the process's cold start and
/// every later one saw `false`. The predicate this replaced — "no windows exist yet" —
/// was per-`gtk::Application`, so tests were independent by construction; the fix traded
/// a production race for a suite-ordering coupling, and the guard protecting the `open`
/// recovery route passed only because of where it happened to sort.
///
/// **It takes the answer it wants rather than merely resetting**, and that distinction
/// is the whole fix. A bare reset makes every launching test a cold start, which is just
/// as wrong in the other direction: a test that builds a window and then calls `open` is
/// modelling a launch into a RUNNING instance, and forcing a cold start there runs crash
/// recovery it never expected. Both were observed — resetting unconditionally reddened
/// two `documents` tests that had been green. So a test states which launch it is
/// modelling, and no test's verdict depends on which ran first.
///
/// A guard rather than a setter: a test that set state on the way in and panicked would
/// leave it set for everything after it, which is the same order-dependence one step
/// removed.
#[cfg(all(test, feature = "gtk-integration-tests"))]
#[must_use = "the forced state only holds while the guard is alive"]
pub(crate) fn force_for_test(is_cold_start: bool) -> ClaimGuard {
    // `claim` returns `!claimed`, so "the next claim answers `is_cold_start`" means
    // the flag must hold its negation.
    let previous = CLAIMED.with(|c| c.0.replace(!is_cold_start));
    ClaimGuard { previous }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
pub(crate) struct ClaimGuard {
    previous: bool,
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
impl Drop for ClaimGuard {
    fn drop(&mut self) {
        CLAIMED.with(|c| c.0.set(self.previous));
    }
}

/// The claim itself, as a value rather than as the process's one global.
///
/// Split out so the rule — *first caller wins, every later one loses, for good* — is
/// testable without consuming the process's own claim, which is by design unresettable
/// and so could otherwise be asserted on only once per test binary.
struct Claim(std::cell::Cell<bool>);

impl Claim {
    const fn new() -> Self {
        Self(std::cell::Cell::new(false))
    }

    /// `true` to the first caller only. Named `take` rather than `is_cold_start`
    /// because asking IS claiming: there is deliberately no way to read the answer
    /// without consuming it, which is what stops a caller checking early and acting
    /// after an await.
    fn take(&self) -> bool {
        !self.0.replace(true)
    }
}

#[cfg(test)]
mod tests {
    use super::Claim;

    /// The whole contract: one `true`, then `false` for good.
    ///
    /// The second and third calls are what matter. A predicate derived from observable
    /// state — "no windows exist yet" — answers `true` repeatedly for as long as that
    /// state lasts, and that was the defect: the state stayed true across the await
    /// session restore occupies, so a second launch arriving inside it restored the
    /// whole session again.
    #[test]
    fn the_first_caller_claims_the_cold_start_and_no_later_one_can() {
        let claim = Claim::new();
        assert!(claim.take(), "the first caller is the cold start");
        assert!(
            !claim.take(),
            "a second caller is not, even immediately after"
        );
        assert!(!claim.take(), "and it never becomes available again");
    }

    /// Two claims are independent, so the rule above is a property of the claim and
    /// not of some shared global the first test happened to reach first.
    #[test]
    fn two_claims_do_not_share_a_verdict() {
        let a = Claim::new();
        let b = Claim::new();
        assert!(a.take());
        assert!(b.take(), "b's verdict is its own");
        assert!(!a.take());
        assert!(!b.take());
    }
}
