use std::time::Duration;

/// Default cap: an 8192x8192 canvas. Matches the application's `MAX_IMAGE_PIXELS`
/// intent (sdd/PLAN.memory-gates.md, "Own the decode — richimg"); the caller
/// supplies its own value from `limits.rs`/`config.toml`, this is only the
/// crate's standalone default.
const DEFAULT_MAX_DIMENSION_PIXELS: u64 = 8192;

/// Default substitute for a declared per-frame delay under [`SHORT_DELAY_THRESHOLD`].
const DEFAULT_SHORT_DELAY_SUBSTITUTE_MS: u64 = 50;

/// A declared delay strictly below this is replaced by `Limits::short_delay_substitute`.
/// Fixed by the format decision (sdd/PLAN.memory-gates.md, "Frame timing: a delay
/// under 20 ms means 50 ms") — unlike `short_delay_substitute` this threshold is not
/// configurable.
const SHORT_DELAY_THRESHOLD_MS: u64 = 20;

/// Below this declared per-frame delay, [`Limits::short_delay_substitute`] is used
/// instead of the declared value. Fixed; see the module-level doc comment.
pub const SHORT_DELAY_THRESHOLD: Duration = Duration::from_millis(SHORT_DELAY_THRESHOLD_MS);

/// Caller-supplied ceilings on decoding cost.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum `width * height` this crate will decode. Checked centrally by
    /// [`crate::Animation::new`] and [`crate::first_frame`] before any canvas
    /// is allocated; exceeding it returns [`crate::Error::TooLarge`].
    pub max_pixels: u64,
    /// What a declared per-frame delay under [`SHORT_DELAY_THRESHOLD`] becomes.
    pub short_delay_substitute: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_pixels: DEFAULT_MAX_DIMENSION_PIXELS * DEFAULT_MAX_DIMENSION_PIXELS,
            short_delay_substitute: Duration::from_millis(DEFAULT_SHORT_DELAY_SUBSTITUTE_MS),
        }
    }
}

impl Limits {
    /// The delay a frame is actually shown for: its declared delay, or
    /// [`Self::short_delay_substitute`] when that is under [`SHORT_DELAY_THRESHOLD`].
    /// Every codec converts its format's delay unit to a `Duration` and calls this.
    pub(crate) fn effective_delay(&self, declared: Duration) -> Duration {
        if declared < SHORT_DELAY_THRESHOLD {
            self.short_delay_substitute
        } else {
            declared
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_match_documented_values() {
        let limits = Limits::default();
        assert_eq!(limits.max_pixels, 8192 * 8192);
        assert_eq!(limits.short_delay_substitute, Duration::from_millis(50));
    }

    #[test]
    fn effective_delay_substitutes_only_under_the_threshold() {
        let limits = Limits::default();
        let ms = Duration::from_millis;
        assert_eq!(limits.effective_delay(ms(0)), ms(50));
        assert_eq!(limits.effective_delay(ms(10)), ms(50));
        assert_eq!(limits.effective_delay(ms(19)), ms(50));
        assert_eq!(limits.effective_delay(ms(20)), ms(20));
        assert_eq!(limits.effective_delay(ms(2250)), ms(2250));
    }

    #[test]
    fn short_delay_threshold_is_20ms() {
        assert_eq!(SHORT_DELAY_THRESHOLD, Duration::from_millis(20));
    }
}
