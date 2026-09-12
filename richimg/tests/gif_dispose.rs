//! WP4 / TDD 2.23a-b, 27.1 (GIF): each of the four disposal modes
//! (`Any`, `Keep`, `Background`, `Previous`), including `Previous` on frame
//! 0 specifically, and a frame rect partly outside the logical screen.
//! Every fixture's every frame is checked against `magick -coalesce`
//! (`tests/fixtures/gif/refs/`), built by `make_gif.sh`.
#[path = "support/mod.rs"]
mod support;

use richimg::{Animation, Limits};
use support::{assert_matches_reference, read_gif_fixture, read_gif_ref_frame};

const CANVAS: u32 = 8;

fn assert_all_frames_match(fixture: &str, frame_count: u32) {
    let limits = Limits::default();
    let bytes = read_gif_fixture(fixture);
    let mut anim =
        Animation::new(bytes, &limits).unwrap_or_else(|err| panic!("open {fixture}: {err}"));
    let stem = fixture.trim_end_matches(".gif");
    for index in 0..frame_count {
        let frame = anim
            .next_frame()
            .unwrap_or_else(|err| panic!("{fixture} frame {index}: {err}"));
        assert_eq!(frame.index, index, "{fixture} frame {index}: wrong index");
        let reference = read_gif_ref_frame(stem, index);
        assert_matches_reference(
            &frame.rgba,
            &reference,
            CANVAS,
            CANVAS,
            &[],
            &format!("{fixture} frame {index}"),
        );
    }
}

/// `Any` (disposal 0, frame 0) and `Keep` (disposal 1, frame 1) are both the
/// "leave the frame" no-op in richimg's compositing — this fixture exercises
/// both raw disposal codes and shows the canvas accumulating both squares
/// rather than either one being cleared.
#[test]
fn dispose_any_and_keep_leave_prior_content_in_place() {
    assert_all_frames_match("dispose_any_and_keep.gif", 3);
}

/// Mutation target: `Background` disposal must clear to transparent before
/// the next frame draws. Neutering it (see the report) turns this red.
#[test]
fn dispose_background_clears_to_transparent_not_the_bg_color_index() {
    assert_all_frames_match("dispose_background.gif", 2);
}

/// Mutation target: `Previous` disposal must restore the canvas to its
/// state before the disposed frame was drawn.
#[test]
fn dispose_previous_restores_the_pre_draw_canvas() {
    assert_all_frames_match("dispose_previous.gif", 3);
}

/// The pinned edge case: `Previous` disposal on frame 0 itself is invalid
/// (there is no state "before" the first frame) and must behave as
/// clear-to-transparent, per the researcher-confirmed citations in
/// `gif.rs`'s `map_loop_count` neighbourhood (Gecko's `imgFrame`/`Decoder`
/// treat it exactly this way).
#[test]
fn dispose_previous_on_frame_zero_clears_to_transparent() {
    assert_all_frames_match("dispose_previous_frame0.gif", 2);
}

/// A frame rect extending past the logical screen's right and bottom edges
/// must be clipped, never panic — the test passing at all (not aborting)
/// is part of the proof, and the pixel-for-pixel reference comparison
/// covers the clipped region's actual values.
#[test]
fn partial_offscreen_frame_rect_is_clipped_not_panicking() {
    assert_all_frames_match("partial_offscreen.gif", 2);
}
