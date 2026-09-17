//! TDD 2.23a-b, 27.1 (GIF): the transparent-index checkerboard over a
//! prior opaque frame, checked pixel-for-pixel against `magick -coalesce`.
#[path = "support/mod.rs"]
mod support;

use richimg::{Animation, Limits};
use support::{assert_matches_reference, pixel, read_gif_fixture, read_gif_ref_frame};

const CANVAS: u32 = 8;

#[test]
fn transparent_index_reveals_the_prior_frame_underneath() {
    let limits = Limits::default();
    let bytes = read_gif_fixture("transparency.gif");
    let mut anim = Animation::new(bytes, &limits).expect("open transparency.gif");

    let frame0 = anim.next_frame().expect("frame 0");
    assert_matches_reference(
        &frame0.rgba,
        &read_gif_ref_frame("transparency", 0),
        CANVAS,
        CANVAS,
        &[],
        "transparency.gif frame 0",
    );

    let frame1 = anim.next_frame().expect("frame 1");
    assert_matches_reference(
        &frame1.rgba,
        &read_gif_ref_frame("transparency", 1),
        CANVAS,
        CANVAS,
        &[],
        "transparency.gif frame 1",
    );

    // Direct, explicit checks in addition to the reference comparison:
    // a transparent-index checker cell shows the opaque blue base frame
    // underneath (not black, not the base's own color coincidentally).
    assert_eq!(pixel(&frame1.rgba, CANVAS, 2, 2), [0, 0, 255, 255]);
    // A green checker cell is fully opaque green.
    assert_eq!(pixel(&frame1.rgba, CANVAS, 3, 2), [0, 255, 0, 255]);
}

/// Mutation target: a transparent-index pixel must be SKIPPED during
/// compositing, not copied. If that check were neutered, the checker cells
/// that should reveal blue would instead show whatever incidental RGB the
/// decoder attaches to the transparent index — going red against the
/// reference above (see the report for the actual mutation run).
#[test]
fn transparent_pixels_never_carry_opaque_alpha() {
    let limits = Limits::default();
    let bytes = read_gif_fixture("transparency.gif");
    let mut anim = Animation::new(bytes, &limits).expect("open transparency.gif");
    let _ = anim.next_frame().expect("frame 0");
    let frame1 = anim.next_frame().expect("frame 1");

    for &(x, y) in &[(2u32, 2u32), (4, 2), (2, 4), (4, 4)] {
        assert_eq!(
            pixel(&frame1.rgba, CANVAS, x, y),
            [0, 0, 255, 255],
            "transparent checker cell at ({x},{y}) should reveal the opaque blue base"
        );
    }
}
