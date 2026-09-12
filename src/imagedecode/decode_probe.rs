//! Test-only decode counter (Finding 2 / TDD 6.8).
//!
//! `local_cache_reuses_decode_ttd_6_8` used to assert a cache HIT by measuring
//! footprint growth against `TOLERANCE_BYTES` — but `anim.webp` decodes to ~0.49 MiB,
//! comfortably under the 2 MiB Linux tolerance, so a fresh decode on the "cached" load
//! satisfied the assertion just as well as an actual hit: deleting the cache entirely
//! still passed. On a commit that exists because a leak went ungated, a gate that
//! cannot fail is the defect it was written against.
//!
//! The fix is to assert the cache HIT directly rather than inferring it from bytes:
//! count real calls into [`super::decode`], the crate's one decode choke point, and
//! assert the count does not move across the "cached" load. Compiled only under
//! `cfg(all(test, feature = "memory-gates"))` — the exact gate `memgate::gtk`'s tests
//! already carry (`memgate/mod.rs`) — so it adds nothing to a shipped build, to a plain
//! `cargo test`, or to the `gtk-integration-tests` suite; the call site in
//! [`super::decode`] is itself `#[cfg]`-gated identically, so the counter's very
//! existence and its one increment appear or vanish together.

use std::sync::atomic::{AtomicU64, Ordering};

/// A monotonic event counter shared across every test thread in one process.
///
/// A named type rather than two bare statics so the counter's arithmetic can be tested
/// against a PRIVATE instance. The self-test used to reset the real `DECODES` and assert
/// it read exactly 0 and then exactly 2 — sound only while nothing else decodes, which
/// is never guaranteed: sibling tests in the same binary call `decode` on parallel
/// threads, so the assertion passed or failed on timing. (It surfaced when an unrelated
/// test grew long enough to overlap it.) The gates that CONSUME these counters do not
/// have that problem — they run under the serialized `#[gtktest::test]` harness — so the
/// race was confined to the instrument's own test, which is exactly the place a false
/// green is least affordable.
struct Counter(AtomicU64);

impl Counter {
    const fn new() -> Self {
        Counter(AtomicU64::new(0))
    }
    fn note(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
    fn reset(&self) {
        self.0.store(0, Ordering::Relaxed);
    }
}

static DECODES: Counter = Counter::new();

/// Record one real call into [`super::decode`]. Called from inside that function,
/// before it branches on content — so this counts every decode ATTEMPT (including one
/// that later fails to produce a texture), which is exactly what "was this a cache
/// hit" needs: a hit never calls `decode` at all, a miss always does, whatever the
/// outcome.
pub(crate) fn note_decode() {
    DECODES.note();
}

/// Record one VECTOR re-rasterisation (`imagecache::loader::rasterize_vector`).
///
/// Counted separately because a vector re-render never enters [`super::decode`] at all
/// — it goes to gdk-pixbuf with a target size, which is the one decode this module's
/// choke point deliberately does not own (ScrAP-343). Without its own counter,
/// `local_cache_makes_svg_rerender_free_ttd_6_8` keeps the shape the raster gate was
/// just rescued from: a footprint tolerance that a fresh decode fits inside, so
/// deleting the cache would still pass.
pub(crate) fn note_vector_rasterize() {
    VECTOR_RASTERIZES.note();
}

static VECTOR_RASTERIZES: Counter = Counter::new();

/// How many vector re-rasterisations have happened on this process so far.
pub(crate) fn vector_rasterize_count() -> u64 {
    VECTOR_RASTERIZES.get()
}

/// The number of [`super::decode`] calls since the last [`reset_for_test`] (or since
/// process start).
pub(crate) fn count() -> u64 {
    DECODES.get()
}

/// Zero the counter. Tests call this alongside `imagecache::reset_for_test()` so a
/// prior test's decodes cannot be mistaken for this one's.
pub(crate) fn reset_for_test() {
    DECODES.reset();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises a PRIVATE counter, never the shared `DECODES`. Asserting absolute
    /// values against the shared one races every sibling test that decodes — see
    /// [`Counter`]'s own doc comment.
    #[test]
    fn counts_accumulate_and_reset() {
        let counter = Counter::new();
        assert_eq!(counter.get(), 0);
        counter.note();
        counter.note();
        assert_eq!(counter.get(), 2);
        counter.reset();
        assert_eq!(counter.get(), 0);
    }
}
