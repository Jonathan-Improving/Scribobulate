//! Per-render memory-growth gating (TDD 6.6–6.8).
//!
//! Two halves of one class, catching disjoint failures:
//!
//! * [`slope`] — after discarding warm-up, the second half of a sample series
//!   must not sit more than a per-platform tolerance above the first. That is
//!   the only honest shape: a single-shot "render, free, assert the number
//!   came back" cannot pass on a correct implementation, because freed pages
//!   stay with the allocator on every platform this project ships.
//! * [`footprint`] — one sampler, three `cfg` bodies. The field is named
//!   **footprint**, never RSS: `/proc` VmRSS, macOS `ri_phys_footprint` and
//!   Windows `WorkingSetSize` are not the same quantity.
//!
//! The GTK driver that *produces* the samples lives in [`gtk`] and is compiled
//! only under the `memory-gates` feature, so pipeline step 5 never runs it.
//! The decision cores here run as ordinary unit tests (step 4) and stay inside
//! the coverage ratchet.

pub(crate) mod footprint;
pub(crate) mod slope;

#[cfg(all(test, feature = "memory-gates"))]
mod gtk;
