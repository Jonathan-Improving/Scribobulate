//! TDD 2.23a-b, 6.9 (GIF): an over-cap logical screen must refuse
//! with `Error::TooLarge` from `Animation::new` and `first_frame` WITHOUT
//! ever allocating a canvas-sized buffer, and `probe` must read the (bogus)
//! dimensions without allocating anything canvas-sized either. A counting
//! `#[global_allocator]` proves this directly, in its own test binary
//! target for the same reason `no_oversized_alloc.rs` uses one: it
//! instruments every allocation this whole process makes, so sharing it
//! with other richimg tests would make "no huge allocation happened"
//! depend on what else happened to run in the same binary. Only one
//! `#[global_allocator]` may exist per binary, which is why this is not
//! simply added to `no_oversized_alloc.rs`.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use richimg::{first_frame, Animation, Error, Limits};

struct CountingAllocator;

/// A canvas for the oversized_canvas.gif fixture's claimed 16384x16384
/// RGBA canvas would be 16384 * 16384 * 4 = 1,073,741,824 bytes — this
/// threshold is two orders of magnitude below that, generous enough to
/// never trip on legitimate small allocations (Vec growth, the decoder's
/// own bookkeeping) that have nothing to do with a pixel canvas.
const MAX_LEGITIMATE_ALLOCATION_BYTES: usize = 1024 * 1024; // 1 MiB

static LARGEST_ALLOCATION: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LARGEST_ALLOCATION.fetch_max(layout.size(), Ordering::SeqCst);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LARGEST_ALLOCATION.fetch_max(new_size, Ordering::SeqCst);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn read_oversized_canvas_fixture() -> Arc<[u8]> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/gif/oversized_canvas.gif");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|err| panic!("reading {}: {err}", path.display()));
    Arc::from(bytes)
}

#[test]
fn oversized_canvas_refuses_without_allocating_a_canvas_sized_buffer() {
    let limits = Limits::default();
    let bytes = read_oversized_canvas_fixture();

    LARGEST_ALLOCATION.store(0, Ordering::SeqCst);
    let info = richimg::probe(&bytes, &limits).expect("probe reads headers only");
    assert_eq!((info.width, info.height), (16384, 16384));
    assert_eq!(info.frame_count, Some(0));
    assert!(
        LARGEST_ALLOCATION.load(Ordering::SeqCst) < MAX_LEGITIMATE_ALLOCATION_BYTES,
        "probe() allocated {} bytes in a single request",
        LARGEST_ALLOCATION.load(Ordering::SeqCst)
    );

    LARGEST_ALLOCATION.store(0, Ordering::SeqCst);
    let new_result = Animation::new(Arc::clone(&bytes), &limits);
    assert_eq!(new_result.err(), Some(Error::TooLarge));
    assert!(
        LARGEST_ALLOCATION.load(Ordering::SeqCst) < MAX_LEGITIMATE_ALLOCATION_BYTES,
        "Animation::new allocated {} bytes in a single request before refusing",
        LARGEST_ALLOCATION.load(Ordering::SeqCst)
    );

    LARGEST_ALLOCATION.store(0, Ordering::SeqCst);
    let first_frame_result = first_frame(&bytes, &limits);
    assert_eq!(first_frame_result.err(), Some(Error::TooLarge));
    assert!(
        LARGEST_ALLOCATION.load(Ordering::SeqCst) < MAX_LEGITIMATE_ALLOCATION_BYTES,
        "first_frame allocated {} bytes in a single request before refusing",
        LARGEST_ALLOCATION.load(Ordering::SeqCst)
    );
}

#[test]
fn an_ordinary_small_fixture_still_decodes_under_the_counting_allocator() {
    // A control: the counting allocator itself must not be why nothing
    // decodes — an unrelated fixture should decode normally under it.
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gif/still.gif");
    let bytes = std::fs::read(&path).expect("reading still.gif");
    let limits = Limits::default();
    let frame = first_frame(&bytes, &limits).expect("decode the control fixture");
    assert_eq!((frame.width, frame.height), (8, 8));
}
