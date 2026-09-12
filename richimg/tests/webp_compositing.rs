//! WP2 / TDD 2.23a-b: dispose-to-background and partial-transparent
//! compositing against `magick -coalesce`, including one DOCUMENTED divergence
//! (blend-off alpha) that is not a tolerance fudge — see that test's comment.
#[path = "support/mod.rs"]
mod support;

use richimg::{Animation, Limits};
use support::{assert_matches_reference, pixel, read_fixture, read_ref_frame, Exclude};

const CANVAS: (u32, u32) = (16, 16);

/// Frame 0 is a full, opaque red canvas with `dispose=background`. Frame 1
/// covers only the LEFT half (columns 0-7) with opaque green; the RIGHT half
/// (columns 8-15) is the disposed region and must be TRANSPARENT — what libwebp's
/// `WebPAnimDecoder`, every browser engine and `magick -coalesce` show. The
/// fixture's ANIM bgcolor (A,R,G,B = 255,200,100,50) is deliberately opaque and
/// non-grey, so a decoder that filled the hint instead — in either channel order
/// — fails here.
#[test]
fn dispose_to_background_clears_the_disposed_region_to_transparent() {
    let limits = Limits::default();
    let bytes = read_fixture("dispose_to_background.webp");
    let mut anim = Animation::new(bytes, &limits).expect("open dispose_to_background");
    for index in 0..2 {
        let frame = anim.next_frame().expect("frame");
        assert_matches_reference(
            &frame.rgba,
            &read_ref_frame("dispose_to_background", index),
            CANVAS.0,
            CANVAS.1,
            &[],
            &format!("dispose_to_background frame {index}"),
        );
    }
}

/// Frame 0: full opaque blue. Frame 1: an 8x8 semi-transparent green square
/// at (4,4), blend ON — alpha-composited ("over") atop the blue. Frame 2:
/// the same region, semi-transparent red, blend OFF — a straight overwrite
/// that must keep the frame's own straight (non-premultiplied) alpha.
///
/// `magick -coalesce` agrees with richimg on frame 1 (both alpha-composite,
/// within the usual ±1 rounding) but DISAGREES on frame 2's alpha channel:
/// measured, magick reports the blend-off region as fully opaque
/// (`[255, 0, 0, 255]`) while richimg preserves the source frame's actual
/// straight alpha (`[255, 0, 0, 128]`), which is what the WebP spec's
/// "do not blend" flag means (a verbatim copy of the frame's own pixels,
/// alpha included) and what `extended::composite_frame`'s
/// `frame_has_alpha && !frame_use_alpha_blending` branch in `image-webp`
/// 0.2.4 actually does (a plain `copy_from_slice`, confirmed by reading that
/// branch). This reads as a real gap in `magick`'s WebP demux, not a
/// richimg defect, so frame 2's ALPHA channel in the blended square is
/// excluded from the reference comparison and asserted directly instead.
#[test]
fn partial_transparent_blend_on_and_off_composite_correctly() {
    let limits = Limits::default();
    let bytes = read_fixture("partial_transparent.webp");
    let mut anim = Animation::new(bytes, &limits).expect("open partial_transparent");

    let frame0 = anim.next_frame().expect("frame 0");
    assert_matches_reference(
        &frame0.rgba,
        &read_ref_frame("partial_transparent", 0),
        CANVAS.0,
        CANVAS.1,
        &[],
        "partial_transparent frame 0",
    );

    let frame1 = anim.next_frame().expect("frame 1");
    assert_matches_reference(
        &frame1.rgba,
        &read_ref_frame("partial_transparent", 1),
        CANVAS.0,
        CANVAS.1,
        &[],
        "partial_transparent frame 1 (blend on)",
    );

    let frame2 = anim.next_frame().expect("frame 2");
    let alpha_in_the_overwritten_square_is_the_documented_divergence = Exclude {
        x0: 4,
        y0: 4,
        x1: 12,
        y1: 12,
        skip_rgb: false,
        skip_alpha: true,
    };
    assert_matches_reference(
        &frame2.rgba,
        &read_ref_frame("partial_transparent", 2),
        CANVAS.0,
        CANVAS.1,
        &[alpha_in_the_overwritten_square_is_the_documented_divergence],
        "partial_transparent frame 2 (blend off, RGB only in the overwritten square)",
    );
    for x in 4..12u32 {
        for y in 4..12u32 {
            let [r, g, b, a] = pixel(&frame2.rgba, CANVAS.0, x, y);
            assert_eq!((r, g, b), (255, 0, 0), "frame 2 pixel ({x},{y}) RGB");
            assert_eq!(
                a, 128,
                "frame 2 pixel ({x},{y}): straight alpha must survive a blend-off overwrite"
            );
        }
    }
}
