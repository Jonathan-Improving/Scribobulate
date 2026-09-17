//! TDD 2.23a-b, 27.1 (GIF): a still GIF always yields frame 0; an
//! animated GIF wraps to a freshly-cleared frame 0 after its last frame,
//! matching the first decode of frame 0 exactly; and `Animation::rewind`
//! does the same on demand.
#[path = "support/mod.rs"]
mod support;

use richimg::{Animation, Limits};
use support::{read_gif_fixture, read_gif_ref_frame};

const CANVAS: u32 = 8;

#[test]
fn a_still_gif_keeps_returning_frame_zero() {
    let limits = Limits::default();
    let bytes = read_gif_fixture("still.gif");
    let mut anim = Animation::new(bytes, &limits).expect("open still.gif");

    for attempt in 0..3 {
        let frame = anim
            .next_frame()
            .unwrap_or_else(|err| panic!("attempt {attempt}: {err}"));
        assert_eq!(
            frame.index, 0,
            "attempt {attempt}: still image must always report index 0"
        );
        assert_eq!(
            frame.rgba,
            read_gif_ref_frame("still", 0),
            "attempt {attempt}: still image content must not change"
        );
    }
}

#[test]
fn wrapping_after_the_last_frame_gives_index_zero_with_a_cleared_canvas() {
    let limits = Limits::default();
    let bytes = read_gif_fixture("dispose_previous.gif");
    let mut anim = Animation::new(bytes, &limits).expect("open dispose_previous.gif");

    let first_pass_frame0 = anim.next_frame().expect("frame 0, first pass").rgba;
    anim.next_frame().expect("frame 1"); // has dispose = Previous
    anim.next_frame().expect("frame 2"); // has dispose = Keep

    // frame_count is 3 (indices 0,1,2); the next call must wrap to index 0
    // and reproduce EXACTLY the first decode of frame 0 — proving the wrap
    // clears the whole canvas rather than leaving frame 2's disposal (or
    // frame 1's still-pending Previous restore) bleeding through.
    let wrapped = anim.next_frame().expect("wrapped frame");
    assert_eq!(
        wrapped.index, 0,
        "wrapping past the last frame must report index 0"
    );
    assert_eq!(
        wrapped.rgba, first_pass_frame0,
        "a wrapped frame 0 must exactly match the first decode of frame 0"
    );
    assert_eq!(wrapped.rgba, read_gif_ref_frame("dispose_previous", 0));
}

#[test]
fn rewind_restores_frame_zero_with_a_cleared_canvas() {
    let limits = Limits::default();
    let bytes = read_gif_fixture("dispose_previous.gif");
    let mut anim = Animation::new(bytes, &limits).expect("open dispose_previous.gif");

    let first_pass_frame0 = anim.next_frame().expect("frame 0").rgba;
    anim.next_frame().expect("frame 1");

    anim.rewind();
    let after_rewind = anim.next_frame().expect("frame 0 after rewind");
    assert_eq!(after_rewind.index, 0);
    assert_eq!(after_rewind.rgba, first_pass_frame0);
}

#[test]
fn a_single_frame_gif_wraps_to_itself_with_the_same_bytes() {
    // frame_count == 1: the "still" path and the "wrap" path are the same
    // code path for GIF (see `gif.rs`'s `composited_frame` doc comment) —
    // this exercises the wrap specifically for the one-frame case, since
    // `a_still_gif_keeps_returning_frame_zero` above only proves the
    // observable behaviour, not that it goes through the wrap branch.
    let limits = Limits::default();
    let bytes = read_gif_fixture("still.gif");
    let mut anim = Animation::new(bytes, &limits).expect("open still.gif");
    assert_eq!(anim.info().frame_count, Some(1));

    let first = anim.next_frame().expect("frame 0").rgba;
    let second = anim.next_frame().expect("frame 0 again, via the wrap");
    assert_eq!(second.index, 0);
    assert_eq!(second.rgba, first);
    assert_eq!(CANVAS, anim.info().width);
}
