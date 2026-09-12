//! WP5 / TDD 2.23a-b, 27.1 (APNG): the three dispose ops and two blend ops,
//! against `ffmpeg`'s independent APNG decode. Fixtures are hand-assembled
//! byte-for-byte by `tests/fixtures/make_apng.sh` (no encoder exposes
//! per-frame dispose/blend/sequence-number control), and every canvas here
//! is small enough (4x4 or 4x2) to reason about by hand alongside the
//! reference.
#[path = "support/mod.rs"]
mod support;

use richimg::{Animation, Limits};
use support::{assert_matches_reference, read_apng_ref_frame, read_fixture};

fn decode_all(fixture: &str, frame_count: u32) -> Vec<Vec<u8>> {
    let limits = Limits::default();
    let bytes = read_fixture(fixture);
    let mut anim =
        Animation::new(bytes, &limits).unwrap_or_else(|err| panic!("open {fixture}: {err}"));
    (0..frame_count)
        .map(|index| {
            let frame = anim
                .next_frame()
                .unwrap_or_else(|err| panic!("decode {fixture} frame {index}: {err}"));
            assert_eq!(frame.index, index);
            frame.rgba
        })
        .collect()
}

/// dispose = NONE: three frames (one full-canvas, two disjoint partial
/// corners) must all remain visible together — nothing is cleared between
/// them.
#[test]
fn dispose_none_accumulates_every_region() {
    let frames = decode_all("apng/dispose_none.png", 3);
    for (index, frame) in frames.iter().enumerate() {
        let reference = read_apng_ref_frame("dispose_none", index as u32);
        assert_matches_reference(
            frame,
            &reference,
            4,
            4,
            &[],
            &format!("dispose_none frame {index}"),
        );
    }
}

/// dispose = BACKGROUND: frame 0's full-canvas region must clear to
/// transparent before frame 1 (which only covers the left half) draws, so
/// the right half of frame 1's output is transparent, never frame 0's red.
#[test]
fn dispose_background_clears_before_the_next_frame() {
    let frames = decode_all("apng/dispose_background.png", 2);
    for (index, frame) in frames.iter().enumerate() {
        let reference = read_apng_ref_frame("dispose_background", index as u32);
        assert_matches_reference(
            frame,
            &reference,
            4,
            2,
            &[],
            &format!("dispose_background frame {index}"),
        );
    }
    // The right half of frame 1 is transparent (BACKGROUND actually fired),
    // asserted directly rather than only through the reference comparison.
    let frame1 = &frames[1];
    assert_eq!(support::pixel(frame1, 4, 2, 0), [0, 0, 0, 0]);
    assert_eq!(support::pixel(frame1, 4, 3, 0), [0, 0, 0, 0]);
}

/// dispose = PREVIOUS (not on frame 0): frame 1's disposal must restore
/// frame 0's pixels, not frame 1's own, and not a BACKGROUND-style clear.
#[test]
fn dispose_previous_restores_pre_frame_state() {
    let frames = decode_all("apng/dispose_previous.png", 3);
    for (index, frame) in frames.iter().enumerate() {
        let reference = read_apng_ref_frame("dispose_previous", index as u32);
        assert_matches_reference(
            frame,
            &reference,
            4,
            4,
            &[],
            &format!("dispose_previous frame {index}"),
        );
    }
    // Frame 2 (transparent, blend=Over — a no-op blend) reveals whatever
    // frame 1's disposal restored: must be frame 0's blue, not green.
    assert_eq!(support::pixel(&frames[2], 4, 0, 0), [0, 0, 255, 255]);
}

/// dispose = PREVIOUS on frame 0 specifically must be treated as BACKGROUND
/// (there is no earlier state to restore) — the canvas goes transparent
/// after frame 0, not "restores an undefined state" (which a naive
/// implementation might read as a no-op, leaving frame 0's red showing).
#[test]
fn dispose_previous_on_first_frame_is_treated_as_background() {
    let frames = decode_all("apng/dispose_previous_first_frame.png", 2);
    for (index, frame) in frames.iter().enumerate() {
        let reference = read_apng_ref_frame("dispose_previous_first_frame", index as u32);
        assert_matches_reference(
            frame,
            &reference,
            4,
            4,
            &[],
            &format!("dispose_previous_first_frame frame {index}"),
        );
    }
    assert_eq!(support::pixel(&frames[1], 4, 0, 0), [0, 0, 0, 0]);
}

/// blend = SOURCE: the subframe's own straight alpha overwrites the region
/// verbatim — never blended with the opaque blue underneath.
#[test]
fn blend_source_overwrites_including_alpha() {
    let frames = decode_all("apng/blend_source.png", 2);
    for (index, frame) in frames.iter().enumerate() {
        let reference = read_apng_ref_frame("blend_source", index as u32);
        assert_matches_reference(
            frame,
            &reference,
            4,
            4,
            &[],
            &format!("blend_source frame {index}"),
        );
    }
    // The subframe's own semi-transparent green, unblended.
    assert_eq!(support::pixel(&frames[1], 4, 1, 1), [0, 255, 0, 128]);
}

/// blend = OVER: the region must show the spec's straight-alpha "over"
/// composite of the semi-transparent source onto the opaque destination,
/// matching ffmpeg's independent decode within the documented ±1 tolerance.
#[test]
fn blend_over_composites_per_the_spec_formula() {
    let frames = decode_all("apng/blend_over.png", 2);
    for (index, frame) in frames.iter().enumerate() {
        let reference = read_apng_ref_frame("blend_over", index as u32);
        assert_matches_reference(
            frame,
            &reference,
            4,
            4,
            &[],
            &format!("blend_over frame {index}"),
        );
    }
    // Not a straight overwrite: green blended over blue is neither pure
    // green nor pure blue.
    let blended = support::pixel(&frames[1], 4, 1, 1);
    assert_ne!(blended, [0, 255, 0, 128]);
    assert_ne!(blended[..3], [0, 0, 255]);
}
