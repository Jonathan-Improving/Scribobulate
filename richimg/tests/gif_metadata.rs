//! WP4 / TDD 2.23a-b (GIF): `probe`'s `Info` (dimensions, `animated`,
//! `frame_count`, `loop_count`) without decoding any pixels, the three-way
//! loop-count mapping, and the frame-delay floor.
#[path = "support/mod.rs"]
mod support;

use std::time::Duration;

use richimg::{probe, Animation, Limits, LoopCount};
use support::read_gif_fixture;

const CANVAS: u32 = 8;

fn probe_info(fixture: &str) -> richimg::Info {
    let limits = Limits::default();
    let bytes = read_gif_fixture(fixture);
    probe(&bytes, &limits).unwrap_or_else(|err| panic!("probe {fixture}: {err}"))
}

#[test]
fn probe_reports_dimensions_without_decoding_pixels() {
    let info = probe_info("dispose_any_and_keep.gif");
    assert_eq!((info.width, info.height), (CANVAS, CANVAS));
}

#[test]
fn probe_reports_frame_count_and_animated_for_a_multi_frame_gif() {
    let info = probe_info("dispose_any_and_keep.gif");
    assert_eq!(info.frame_count, Some(3));
    assert!(info.animated);
}

#[test]
fn probe_reports_a_still_gif_as_not_animated_with_one_frame() {
    let info = probe_info("still.gif");
    assert_eq!(info.frame_count, Some(1));
    assert!(!info.animated);
}

/// Loop-count mapping, case 1 of 3: no NETSCAPE2.0 block at all → play once.
#[test]
fn loop_count_no_netscape_block_is_finite_one() {
    assert_eq!(probe_info("loop_none.gif").loop_count, LoopCount::Finite(1));
}

/// Loop-count mapping, case 2 of 3: NETSCAPE count 0 → infinite.
#[test]
fn loop_count_netscape_zero_is_infinite() {
    assert_eq!(
        probe_info("loop_infinite.gif").loop_count,
        LoopCount::Infinite
    );
}

/// Loop-count mapping, case 3 of 3: NETSCAPE count N (>0, here 4) →
/// N+1 total plays (`loop_finite.gif` is built with a raw NETSCAPE count
/// of 4, so 5 total plays).
#[test]
fn loop_count_netscape_n_is_n_plus_one_total_plays() {
    assert_eq!(
        probe_info("loop_finite.gif").loop_count,
        LoopCount::Finite(5)
    );
}

/// The delay floor: 0ms and 10ms (both under the fixed 20ms threshold)
/// substitute to `Limits::short_delay_substitute` (50ms by default); a
/// declared 20ms passes through unchanged.
#[test]
fn delay_floor_substitutes_short_delays_and_passes_through_the_rest() {
    let limits = Limits::default();
    let bytes = read_gif_fixture("delay_floor.gif");
    let mut anim = Animation::new(bytes, &limits).expect("open delay_floor.gif");

    let frame0 = anim.next_frame().expect("frame 0 (declared 0ms)");
    assert_eq!(frame0.delay, Duration::from_millis(50));

    let frame1 = anim.next_frame().expect("frame 1 (declared 10ms)");
    assert_eq!(frame1.delay, Duration::from_millis(50));

    let frame2 = anim.next_frame().expect("frame 2 (declared 20ms)");
    assert_eq!(frame2.delay, Duration::from_millis(20));
}
