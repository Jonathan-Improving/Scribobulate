//! TDD 2.23a-b, 27.1 (APNG): everything besides raw compositing —
//! frame-count/loop-count reporting, fractional and floored delays,
//! color-type normalisation (palette+tRNS, 16-bit), the "default image is
//! not a frame" rule, the `frame_count == 1` still path, wrap-after-last,
//! and explicit `rewind`.
#[path = "support/mod.rs"]
mod support;

use std::time::Duration;

use richimg::{probe, Animation, Limits, LoopCount};
use support::{assert_matches_reference, read_apng_ref_frame, read_fixture};

fn open(fixture: &str) -> Animation {
    let limits = Limits::default();
    let bytes = read_fixture(fixture);
    Animation::new(bytes, &limits).unwrap_or_else(|err| panic!("open {fixture}: {err}"))
}

// ---------------------------------------------------------------------------
// Loop count: acTL.num_plays is TOTAL plays (0 => Infinite, N>0 => Finite(N)).
// ---------------------------------------------------------------------------

#[test]
fn finite_loop_count_is_reported_as_total_plays() {
    let limits = Limits::default();
    let bytes = read_fixture("apng/loop_finite.png");
    let info = probe(&bytes, &limits).expect("probe loop_finite");
    assert_eq!(info.loop_count, LoopCount::Finite(3));
    assert_eq!(info.frame_count, Some(2));
    assert!(info.animated);
}

#[test]
fn zero_num_plays_is_infinite() {
    let limits = Limits::default();
    let bytes = read_fixture("apng/loop_infinite.png");
    let info = probe(&bytes, &limits).expect("probe loop_infinite");
    assert_eq!(info.loop_count, LoopCount::Infinite);
}

// ---------------------------------------------------------------------------
// Delays: fractions (including delay_den == 0 meaning 1/100s) and the floor.
// ---------------------------------------------------------------------------

#[test]
fn fractional_delays_convert_exactly_without_float_drift() {
    let mut anim = open("apng/fractional_delays.png");
    let frame0 = anim.next_frame().expect("frame 0 (1/3 s)");
    let frame1 = anim.next_frame().expect("frame 1 (7/100 s)");
    let frame2 = anim.next_frame().expect("frame 2 (5/0 -> 5/100 s)");

    assert_eq!(frame0.delay, Duration::from_nanos(333_333_333));
    assert_eq!(frame1.delay, Duration::from_millis(70));
    assert_eq!(frame2.delay, Duration::from_millis(50));
}

#[test]
fn delay_floor_applies_through_the_real_decode_path() {
    let limits = Limits::default();
    let mut anim = open("apng/short_delays.png");

    let declared_0ms = anim.next_frame().expect("frame with declared 0ms delay");
    let declared_10ms = anim.next_frame().expect("frame with declared 10ms delay");
    let declared_20ms = anim.next_frame().expect("frame with declared 20ms delay");

    assert_eq!(declared_0ms.delay, limits.short_delay_substitute);
    assert_eq!(declared_10ms.delay, limits.short_delay_substitute);
    assert_eq!(declared_20ms.delay, Duration::from_millis(20));
}

// ---------------------------------------------------------------------------
// Colour-type normalisation.
// ---------------------------------------------------------------------------

#[test]
fn palette_with_trns_normalises_to_straight_alpha_rgba() {
    let mut anim = open("apng/palette_trns.png");
    for index in 0..2u32 {
        let frame = anim.next_frame().expect("palette_trns frame");
        assert_eq!(frame.index, index);
        let reference = read_apng_ref_frame("palette_trns", index);
        assert_matches_reference(
            &frame.rgba,
            &reference,
            2,
            2,
            &[],
            &format!("palette_trns frame {index}"),
        );
    }
}

#[test]
fn sixteen_bit_channels_reduce_to_eight_bits() {
    let mut anim = open("apng/sixteen_bit.png");
    for index in 0..2u32 {
        let frame = anim.next_frame().expect("sixteen_bit frame");
        let reference = read_apng_ref_frame("sixteen_bit", index);
        assert_matches_reference(
            &frame.rgba,
            &reference,
            2,
            2,
            &[],
            &format!("sixteen_bit frame {index}"),
        );
    }
}

// ---------------------------------------------------------------------------
// The default image is not a frame.
// ---------------------------------------------------------------------------

/// The distinctive magenta default-image colour used by the fixture; it must
/// never appear in any decoded frame.
const DEFAULT_IMAGE_MAGENTA: [u8; 4] = [255, 0, 255, 255];

#[test]
fn default_image_is_excluded_from_frame_count_and_never_shown() {
    let limits = Limits::default();
    let bytes = read_fixture("apng/default_image_not_a_frame.png");
    let info = probe(&bytes, &limits).expect("probe default_image_not_a_frame");
    // acTL.num_frames already excludes the default image: 2 real frames,
    // not 3.
    assert_eq!(info.frame_count, Some(2));

    let mut anim = Animation::new(bytes, &limits).expect("open default_image_not_a_frame");
    for index in 0..2u32 {
        let frame = anim
            .next_frame()
            .unwrap_or_else(|err| panic!("decode default_image_not_a_frame frame {index}: {err}"));
        assert_eq!(frame.index, index);
        let reference = read_apng_ref_frame("default_image_not_a_frame", index);
        assert_matches_reference(
            &frame.rgba,
            &reference,
            4,
            4,
            &[],
            &format!("default_image_not_a_frame frame {index}"),
        );
        for pixel in frame.rgba.chunks_exact(4) {
            assert_ne!(
                pixel, DEFAULT_IMAGE_MAGENTA,
                "the non-animated default image's magenta must never appear in a decoded frame"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// frame_count == 1 is a "still": next_frame keeps returning frame 0.
// ---------------------------------------------------------------------------

#[test]
fn a_single_frame_apng_is_a_still_that_replays_frame_zero() {
    let limits = Limits::default();
    let bytes = read_fixture("apng/still.png");
    let info = probe(&bytes, &limits).expect("probe still");
    assert_eq!(info.frame_count, Some(1));

    let mut anim = Animation::new(bytes, &limits).expect("open still");
    let first = anim.next_frame().expect("frame 0");
    assert_eq!(first.index, 0);
    for _ in 0..3 {
        let repeat = anim.next_frame().expect("still keeps returning frame 0");
        assert_eq!(repeat.index, 0);
        assert_eq!(repeat.rgba, first.rgba);
    }
}

// ---------------------------------------------------------------------------
// Wrap after the last frame, and explicit rewind — both must clear the
// canvas, not merely rewind a frame pointer (dispose_none accumulates
// regions across frames, so a missed clear would leak a prior loop's pixels
// into the re-decoded frame 0).
// ---------------------------------------------------------------------------

#[test]
fn wraps_to_a_matching_cleared_frame_zero_after_the_last_frame() {
    let mut anim = open("apng/dispose_none.png");
    let frame0_original = anim.next_frame().expect("frame 0").rgba;
    let _frame1 = anim.next_frame().expect("frame 1");
    let _frame2 = anim.next_frame().expect("frame 2");

    let wrapped = anim.next_frame().expect("frame after wrap");
    assert_eq!(wrapped.index, 0, "wrap must report index 0 again");
    assert_eq!(
        wrapped.rgba, frame0_original,
        "the wrapped frame 0 must be pixel-identical to the original frame 0 — any \
         difference means a prior loop's partial frame leaked through an incomplete clear"
    );
}

#[test]
fn explicit_rewind_also_returns_to_a_matching_cleared_frame_zero() {
    let mut anim = open("apng/dispose_none.png");
    let frame0_original = anim.next_frame().expect("frame 0").rgba;
    let _frame1 = anim.next_frame().expect("frame 1");
    let _frame2 = anim.next_frame().expect("frame 2");

    anim.rewind();
    let after_rewind = anim.next_frame().expect("frame 0 after rewind");
    assert_eq!(after_rewind.index, 0);
    assert_eq!(after_rewind.rgba, frame0_original);
}
