//! Slope-over-N after discarding K warm-up samples.
//!
//! Display-free: the GTK driver feeds it a slice of footprint readings; this
//! module never opens a process or a file. That is what lets the assertion
//! itself be unit-tested with planted series — a climbing series must fail,
//! a plateau must pass — so a later change to the arithmetic cannot silently
//! invert the gate (GEP-1 / ScrAP-209).

/// Discard the first `warmup` readings. Allocator and loader warm-up dominate
/// those; a leak is what keeps climbing after they have saturated.
pub(crate) fn after_warmup(samples: &[u64], warmup: usize) -> &[u64] {
    if warmup >= samples.len() {
        &[]
    } else {
        &samples[warmup..]
    }
}

/// Mean of `second half − first half` over `samples`, or `None` when there are
/// fewer than two readings. An odd length drops the middle so both halves are
/// the same size.
pub(crate) fn half_delta(samples: &[u64]) -> Option<i64> {
    if samples.len() < 2 {
        return None;
    }
    let half = samples.len() / 2;
    let first = mean(&samples[..half]);
    let second = mean(&samples[samples.len() - half..]);
    Some(second - first)
}

fn mean(samples: &[u64]) -> i64 {
    let sum: u128 = samples.iter().copied().map(u128::from).sum();
    (sum / samples.len() as u128) as i64
}

/// `Ok(())` when the second-half mean sits no more than `tolerance` above the
/// first-half mean, after `warmup` readings are discarded.
pub(crate) fn assert_flat(samples: &[u64], warmup: usize, tolerance: u64) -> Result<(), String> {
    let rest = after_warmup(samples, warmup);
    let Some(delta) = half_delta(rest) else {
        return Err(format!(
            "need at least {} samples after warmup={warmup} (got {})",
            warmup + 2,
            samples.len()
        ));
    };
    if delta <= tolerance as i64 {
        Ok(())
    } else {
        Err(format!(
            "second-half footprint exceeds first-half by {delta} bytes \
             (tolerance {tolerance}); samples after warmup: {rest:?}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{after_warmup, assert_flat, half_delta};

    #[test]
    fn after_warmup_drops_the_prefix() {
        assert_eq!(after_warmup(&[1, 2, 3, 4, 5], 2), &[3, 4, 5]);
    }

    #[test]
    fn after_warmup_empty_when_warmup_covers_all() {
        assert!(after_warmup(&[1, 2], 2).is_empty());
        assert!(after_warmup(&[1], 5).is_empty());
    }

    #[test]
    fn half_delta_of_a_plateau_is_zero() {
        assert_eq!(half_delta(&[10, 10, 10, 10]), Some(0));
    }

    #[test]
    fn half_delta_of_a_linear_climb_is_positive() {
        // 10, 20, 30, 40 → first mean 15, second mean 35, delta 20.
        assert_eq!(half_delta(&[10, 20, 30, 40]), Some(20));
    }

    #[test]
    fn half_delta_drops_the_middle_of_an_odd_slice() {
        // 10, 20, 30, 40, 50 → halves are [10, 20] and [40, 50], middle 30 dropped.
        assert_eq!(half_delta(&[10, 20, 30, 40, 50]), Some(30));
    }

    #[test]
    fn half_delta_none_on_fewer_than_two() {
        assert_eq!(half_delta(&[]), None);
        assert_eq!(half_delta(&[1]), None);
    }

    #[test]
    fn assert_flat_passes_a_plateau_after_warmup() {
        let samples = [100, 180, 200, 200, 201, 200, 200, 201];
        assert_flat(&samples, 2, 8).unwrap();
    }

    #[test]
    fn assert_flat_fails_a_climb_for_the_slope_reason() {
        // Warm-up 2, then +20 each step. The error must name the delta, not an
        // earlier precondition — otherwise a mutation that breaks the assertion
        // can still "fail" on a length check and look live (ScrAP-183).
        let samples = [10, 20, 30, 50, 70, 90, 110, 130];
        let err = assert_flat(&samples, 2, 8).unwrap_err();
        assert!(
            err.contains("second-half footprint exceeds first-half"),
            "failed for the wrong reason: {err}"
        );
        assert!(
            !err.contains("need at least"),
            "precondition fired instead of the slope: {err}"
        );
    }

    #[test]
    fn assert_flat_names_a_short_series_as_a_precondition() {
        let err = assert_flat(&[1, 2], 2, 8).unwrap_err();
        assert!(err.contains("need at least"), "{err}");
    }
}
