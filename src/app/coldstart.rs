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
    thread_local! {
        /// Main-thread-only: both callers are GApplication signal handlers, which GTK
        /// emits on the main thread. Never reset — a process starts up once.
        static CLAIMED: Claim = const { Claim::new() };
    }
    CLAIMED.with(Claim::take)
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
