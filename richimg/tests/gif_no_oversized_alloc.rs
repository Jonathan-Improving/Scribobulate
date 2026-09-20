//! TDD 2.23a-b, 6.9 (GIF): an over-cap logical screen must refuse with
//! `Error::TooLarge` from `Animation::new` and `first_frame` WITHOUT ever allocating a
//! canvas-sized buffer, and `probe` must read the bogus dimensions without one either.
//!
//! **One binary per format, one implementation.** Only one `#[global_allocator]` may
//! exist per binary, and this one instruments every allocation the whole process makes
//! — so sharing a binary with other richimg tests would make "no huge allocation
//! happened" depend on whatever else ran there. That argues for three BINARIES. It was
//! taken as an argument for three copies of the body, and the copies drifted; the
//! allocator, the threshold and the assertions now live in `support`.

use richimg::{first_frame, Animation, Error, Limits};
use std::sync::Arc;

#[path = "support/mod.rs"]
mod support;

#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;

fn oversized_canvas_fixture() -> Arc<[u8]> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/gif/oversized_canvas.gif");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|err| panic!("reading {}: {err}", path.display()));
    Arc::from(bytes)
}

/// The instrument before the measurement. See
/// `support::assert_the_counter_can_see_a_large_allocation` for why an absence
/// assertion is worthless without it.
#[test]
fn the_counting_allocator_observes_a_deliberate_large_allocation() {
    support::assert_the_counter_can_see_a_large_allocation();
}

#[test]
fn oversized_canvas_refuses_without_allocating_a_canvas_sized_buffer() {
    let limits = Limits::default();
    let bytes = oversized_canvas_fixture();

    // `probe` does not enforce the cap (see `richimg::probe`'s doc comment) — it must
    // still read the bogus dimensions without allocating anything canvas-sized.
    let info =
        support::measured(|| richimg::probe(&bytes, &limits).expect("probe reads headers only"));
    assert_eq!((info.width, info.height), (16384, 16384));
    // GIF alone reports a frame count here: the fixture declares an oversized screen
    // and then no image descriptors, so the count is a real zero rather than an absence.
    assert_eq!(info.frame_count, Some(0));
    support::assert_no_canvas_sized_allocation("probe()");

    let new_result = support::measured(|| Animation::new(Arc::clone(&bytes), &limits));
    assert_eq!(new_result.err(), Some(Error::TooLarge));
    support::assert_no_canvas_sized_allocation("Animation::new");

    let first_frame_result = support::measured(|| first_frame(&bytes, &limits));
    assert_eq!(first_frame_result.err(), Some(Error::TooLarge));
    support::assert_no_canvas_sized_allocation("first_frame");
}

/// A control: the counting allocator itself must not be why nothing decodes.
#[test]
fn an_ordinary_small_fixture_still_decodes_under_the_counting_allocator() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gif/still.gif");
    let bytes = std::fs::read(&path).expect("reading the control fixture");
    let limits = Limits::default();
    let frame = first_frame(&bytes, &limits).expect("decode the control fixture");
    assert_eq!((frame.width, frame.height), (8, 8));
}
