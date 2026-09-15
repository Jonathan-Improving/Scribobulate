//! Test-only wall-clock sampling, shared by every guard that asserts an algorithmic
//! exponent by timing two input sizes — `annotate::scan`'s (QA R3 D-2, the scan-per-opener
//! quadratic) and `renderer::normalize`'s (QA R3 D-3, the per-tab backwards line walk).
//!
//! # Why this is a module and not a helper in one test file
//!
//! The same reason `testsymlink` is one: a noise remedy was once written into ONE of those
//! two guards, and the other failed on a hosted CI runner for want of it. A remedy that
//! lives inside one consumer is a remedy the next consumer will not find.
//!
//! # Why the two samples are DURATION-MATCHED
//!
//! Both guards used to compare `cost(4n) / cost(n)` against a threshold of 8 (linear ~4,
//! quadratic ~16), each side a best-of-N minimum. That estimator is sound against noise
//! that strikes at random, and it is BLIND to noise that scales with how long a sample
//! runs — which the scheduler's is. MEASURED on this project's reference host, pinning a
//! guard to one core shared with a single busy loop: the annotation guard failed **10 runs
//! out of 10, every one at 8.0x** (2.25 ms -> 18.0 ms). The small sample fits inside one
//! scheduler slice, so some draw always runs clean and the minimum finds it; the large one
//! is longer than a slice, so EVERY draw is preempted and doubled, and no number of draws
//! can find a clean one. The ratio then reads the scheduler, not the algorithm: exactly the
//! 8.0 a 2x slowdown puts on a linear 4.0. More samples narrow nothing, and a wider
//! threshold spends the discrimination the guard exists for.
//!
//! So [`matched_growth`] compares `cost(k·n)` against `k` runs of `cost(n)` — the SAME total
//! work, hence the same duration, hence the same exposure to anything proportional to time
//! (preemption, a co-tenant's steal, frequency scaling). That noise now lands on both sides
//! and cancels. Linear reads ~1 and quadratic ~k, and [`Growth::limit`] sits at their
//! geometric mean. The two sides are drawn ALTERNATELY, so load that arrives partway
//! through a run strikes both rather than only whichever side was sampled later.
//!
//! What remains is noise that strikes one draw and not its partner, which is exactly what a
//! minimum removes: noise is strictly additive, so the floor of the draws is the estimate of
//! the noise-free cost, where a mean would fold the outliers back in.
//!
//! # The knob
//!
//! `SCRIBTEST_TIMING_SAMPLES` overrides [`DEFAULT_SAMPLES`]. Test-scoped by name on
//! purpose: `SCRIB_*` in this tree means a real build variable (`SCRIB_GTK_PREFIX`,
//! `SCRIB_GIT_COMMIT`), and a knob that can only ever loosen a test must not read like one
//! of those. It is set on the CI execution jobs and nowhere else, so a developer running
//! `cargo test` gets the tight default.
//!
//! An override ANNOUNCES ITSELF on stderr, following the house rule that an operator
//! override says so in the output (`pipeline.ps1 -SkipIntegration` does the same). A green
//! run under a raised sample count must not be mistakable for a green run under the
//! default — the assertion is identical either way, but how hard the machine worked to
//! satisfy it is not, and that belongs in the log rather than in someone's memory.

use std::time::Duration;

/// Samples taken when nothing overrides it. What a developer's `cargo test` uses.
pub(crate) const DEFAULT_SAMPLES: usize = 5;

/// How many draws [`matched_growth`] should take.
///
/// Reads `SCRIBTEST_TIMING_SAMPLES`; falls back to [`DEFAULT_SAMPLES`] when unset, empty,
/// unparseable or zero. A malformed value is deliberately NOT a failure: this knob exists
/// to make a noisy machine's run more reliable, and a typo in CI config that turned every
/// timing guard into a hard error would be a worse outcome than quietly using the default.
/// It is announced either way, so a typo is visible rather than silent.
pub(crate) fn samples() -> usize {
    const VAR: &str = "SCRIBTEST_TIMING_SAMPLES";
    match std::env::var(VAR) {
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(n) if n > 0 => {
                eprintln!("[{VAR}] taking {n} timing samples (default {DEFAULT_SAMPLES})");
                n
            }
            _ => {
                eprintln!(
                    "[{VAR}] ignoring unusable value {raw:?}; using the default \
                     {DEFAULT_SAMPLES}"
                );
                DEFAULT_SAMPLES
            }
        },
        Err(_) => DEFAULT_SAMPLES,
    }
}

/// The best (minimum) draw of each side of a duration-matched pair.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Growth {
    /// How many times larger the large input is than the small one — and so how many
    /// small runs make up one draw of `small`.
    pub span: u32,
    /// Best draw of `span` consecutive runs over the small input.
    pub small: Duration,
    /// Best draw of one run over the large input.
    pub large: Duration,
}

impl Growth {
    /// `large / small`: ~1 for a linear algorithm, ~`span` for a quadratic one.
    pub(crate) fn ratio(&self) -> f64 {
        self.large.as_secs_f64() / self.small.as_secs_f64().max(1e-9)
    }

    /// The largest ratio a linear implementation is allowed: the geometric mean of linear
    /// (1) and quadratic (`span`), so each side has the same multiplicative margin.
    pub(crate) fn limit(&self) -> f64 {
        f64::from(self.span).sqrt()
    }

    /// Whether the measured growth is within [`Self::limit`].
    pub(crate) fn is_linear(&self) -> bool {
        self.ratio() < self.limit()
    }
}

/// Measure `large` (one run over an input `span` times the size) against `span` runs of
/// `small`, alternating the two for [`samples()`] rounds and keeping each side's minimum.
///
/// `ceiling` is the escape hatch for the FAILURE case: sampling must not make a red run
/// more expensive than the bug it is reporting. `annotate::scan`'s pre-fix cost was ~96 s
/// per call, so sampling a regression to completion turns it into a multi-hour run —
/// measured; the first attempt at that guard's fix had to be killed. A `large` draw past
/// `ceiling` abandons the remaining rounds. That is sound ONLY where the ceiling has wide
/// headroom over the linear cost, so that a draw above it means a real regression rather
/// than a slow draw; do not pass a tight bound, where it would reintroduce the flake this
/// module exists to remove.
pub(crate) fn matched_growth(
    span: u32,
    mut small: impl FnMut(),
    mut large: impl FnMut(),
    ceiling: Duration,
) -> Growth {
    let mut growth = Growth {
        span,
        small: Duration::MAX,
        large: Duration::MAX,
    };
    for _ in 0..samples() {
        let t = std::time::Instant::now();
        for _ in 0..span {
            small();
        }
        growth.small = growth.small.min(t.elapsed());

        let t = std::time::Instant::now();
        large();
        growth.large = growth.large.min(t.elapsed());
        if growth.large > ceiling {
            break;
        }
    }
    growth
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default is what an unset environment produces. Pinned because every guard's
    /// threshold was calibrated against it, so a change here silently re-aims all of them.
    #[test]
    fn an_unset_environment_uses_the_default() {
        // Not asserted via `samples()`: this suite may itself be running under an override
        // (that is the entire point of the knob), and a test that reads the ambient
        // environment would then assert about the CI config rather than about this code.
        assert_eq!(DEFAULT_SAMPLES, 5);
    }

    /// Each side must report its FLOOR, not its last or mean draw — the estimator rests on
    /// it, and a `max`/`min` slip would leave every guard measuring noise while still
    /// passing on a quiet machine.
    #[test]
    fn each_side_reports_its_minimum_draw_not_the_last() {
        let (mut small_calls, mut large_calls) = (0u32, 0u32);
        let slow = Duration::from_millis(20);
        let growth = matched_growth(
            2,
            || {
                small_calls += 1;
                // Only the first ROUND is slow (two calls make one small draw).
                if small_calls <= 2 {
                    std::thread::sleep(slow);
                }
            },
            || {
                large_calls += 1;
                if large_calls == 1 {
                    std::thread::sleep(slow);
                }
            },
            Duration::MAX,
        );
        assert!(large_calls >= 2, "sampling must take more than one round");
        assert!(
            growth.small < slow,
            "small side kept a slow draw: {growth:?}"
        );
        assert!(
            growth.large < slow,
            "large side kept a slow draw: {growth:?}"
        );
    }

    /// The pairing is the whole remedy: one draw of the small side must be `span` runs, so
    /// that it costs what one large run costs.
    #[test]
    fn a_small_draw_is_span_runs_and_a_large_draw_is_one() {
        let (mut small_calls, mut large_calls) = (0usize, 0usize);
        matched_growth(8, || small_calls += 1, || large_calls += 1, Duration::MAX);
        assert_eq!(large_calls, samples());
        assert_eq!(small_calls, 8 * samples());
    }

    /// A large draw past the ceiling must stop sampling, or the failure-case cost this
    /// module documents is not bounded at all.
    #[test]
    fn a_large_draw_past_the_ceiling_abandons_the_remaining_rounds() {
        let mut large_calls = 0u32;
        // The draw must take MEASURABLE time. An empty body can time as exactly zero —
        // MEASURED on Windows, where it then never exceeds a zero ceiling and the test
        // failed 4 runs in 5 with the code correct.
        let large = || {
            large_calls += 1;
            std::thread::sleep(Duration::from_millis(1));
        };
        matched_growth(1, || {}, large, Duration::ZERO);
        assert_eq!(large_calls, 1, "a draw past the ceiling must end sampling");
    }

    /// The limit is the geometric mean of linear and quadratic, and the verdict uses it.
    #[test]
    fn the_limit_splits_linear_from_quadratic_evenly() {
        let at = |large_ms: u64| Growth {
            span: 16,
            small: Duration::from_millis(10),
            large: Duration::from_millis(large_ms),
        };
        assert!((at(10).limit() - 4.0).abs() < 1e-9);
        assert!(at(10).is_linear(), "equal cost is linear");
        assert!(at(39).is_linear(), "just under the limit is linear");
        assert!(!at(40).is_linear(), "the limit itself is not");
        assert!(!at(160).is_linear(), "quadratic is not");
    }
}
