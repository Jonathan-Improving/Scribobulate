//! TDD 2.23a-b, 6.9: animated WebP — dimensions/metadata, per-frame
//! fidelity against `magick -coalesce`, the delay floor, and the wrap/rewind
//! contract (`next_frame` after the last frame behaves as `rewind()` then
//! frame 0; `rewind` clears the canvas, not merely the frame pointer).
#[path = "support/mod.rs"]
mod support;

use std::time::Duration;

use richimg::{probe, Animation, Limits, LoopCount};
use support::{assert_matches_reference, read_fixture, read_ref_frame};

const CANVAS: (u32, u32) = (16, 16);

#[test]
fn animated_full_canvas_info_is_correct() {
    let limits = Limits::default();
    let bytes = read_fixture("animated_full_canvas.webp");
    let info = probe(&bytes, &limits).expect("probe animated_full_canvas");
    assert_eq!((info.width, info.height), CANVAS);
    assert!(info.animated);
    assert_eq!(info.frame_count, Some(3));
    assert_eq!(info.loop_count, LoopCount::Finite(3));
}

#[test]
fn animated_full_canvas_frames_match_reference() {
    let limits = Limits::default();
    let bytes = read_fixture("animated_full_canvas.webp");
    let mut anim = Animation::new(bytes, &limits).expect("open animated_full_canvas");

    for index in 0..3u32 {
        let frame = anim.next_frame().expect("decode a full-canvas frame");
        assert_eq!(frame.index, index);
        let reference = read_ref_frame("animated_full_canvas", index);
        assert_matches_reference(
            &frame.rgba,
            &reference,
            CANVAS.0,
            CANVAS.1,
            &[],
            &format!("animated_full_canvas frame {index}"),
        );
    }
}

#[test]
fn animated_full_canvas_wraps_to_frame_zero_after_the_last_frame() {
    let limits = Limits::default();
    let bytes = read_fixture("animated_full_canvas.webp");
    let mut anim = Animation::new(bytes, &limits).expect("open animated_full_canvas");

    let first_pass: Vec<_> = (0..3)
        .map(|_| anim.next_frame().expect("frame").rgba)
        .collect();
    let wrapped = anim.next_frame().expect("frame after wrap");
    assert_eq!(wrapped.index, 0, "wrap must report index 0 again");
    assert_eq!(
        wrapped.rgba, first_pass[0],
        "the wrapped frame 0 must be pixel-identical to the original frame 0"
    );
}

#[test]
fn explicit_rewind_also_returns_to_a_matching_frame_zero() {
    let limits = Limits::default();
    let bytes = read_fixture("animated_full_canvas.webp");
    let mut anim = Animation::new(bytes, &limits).expect("open animated_full_canvas");

    let frame0_first = anim.next_frame().expect("frame 0").rgba;
    let _ = anim.next_frame().expect("frame 1");
    anim.rewind();
    let frame0_after_rewind = anim.next_frame().expect("frame 0 again");
    assert_eq!(frame0_after_rewind.index, 0);
    assert_eq!(frame0_after_rewind.rgba, frame0_first);
}

#[test]
fn delay_floor_applies_through_the_real_decode_path() {
    let limits = Limits::default();
    let bytes = read_fixture("short_delays.webp");
    let mut anim = Animation::new(bytes, &limits).expect("open short_delays");

    let declared_0ms = anim.next_frame().expect("frame with declared 0ms delay");
    let declared_10ms = anim.next_frame().expect("frame with declared 10ms delay");
    let declared_20ms = anim.next_frame().expect("frame with declared 20ms delay");

    assert_eq!(declared_0ms.delay, limits.short_delay_substitute);
    assert_eq!(declared_10ms.delay, limits.short_delay_substitute);
    assert_eq!(declared_20ms.delay, Duration::from_millis(20));
}

#[test]
fn partial_frame0_loop_frames_match_reference() {
    let limits = Limits::default();
    let bytes = read_fixture("partial_frame0_loop.webp");
    let mut anim = Animation::new(bytes, &limits).expect("open partial_frame0_loop");

    for index in 0..3u32 {
        let frame = anim.next_frame().expect("decode a partial-loop frame");
        let reference = read_ref_frame("partial_frame0_loop", index);
        assert_matches_reference(
            &frame.rgba,
            &reference,
            CANVAS.0,
            CANVAS.1,
            &[],
            &format!("partial_frame0_loop frame {index}"),
        );
    }
}

/// The fixture that exists specifically to catch a missing canvas clear on
/// rewind: three DISJOINT, non-overlapping opaque regions across frames
/// 0/1/2, dispose=none, over a distinctive (black) background that none of
/// the frames' own colors use. If `rewind` only rewound image-webp's ANMF
/// pointer (the 0.2.4 default `reset_animation` behaviour) rather than
/// clearing the canvas, frame 1's top-right square would still be showing
/// when frame 0 is re-decoded after the wrap — because the pointer-only
/// rewind's own background clear only covers the LAST frame's rectangle
/// (frame 2's bottom half), not every region drawn earlier in the loop.
#[test]
fn partial_frame0_loop_wrap_clears_every_earlier_region_not_just_the_last() {
    let limits = Limits::default();
    let bytes = read_fixture("partial_frame0_loop.webp");
    let mut anim = Animation::new(bytes, &limits).expect("open partial_frame0_loop");

    let frame0_original = anim.next_frame().expect("frame 0").rgba;
    let _frame1 = anim.next_frame().expect("frame 1"); // draws the top-right square
    let _frame2 = anim.next_frame().expect("frame 2"); // draws the bottom half

    let frame0_after_wrap = anim.next_frame().expect("frame 0 after wrap");
    assert_eq!(frame0_after_wrap.index, 0);
    assert_eq!(
        frame0_after_wrap.rgba, frame0_original,
        "frame 0 after the wrap must match the very first decode of frame 0 \
         byte-for-byte — any difference means a prior loop's partial frame \
         leaked through an incomplete canvas clear"
    );
}
