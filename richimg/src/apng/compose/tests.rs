//! Unit tests for `apng::compose` — split into its own file (rather than an
//! inline `#[cfg(test)] mod tests { ... }`) purely to keep `compose.rs`
//! itself under the crate's file-size-per-file limit.
use super::*;

fn geometry(
    x_offset: u32,
    y_offset: u32,
    width: u32,
    height: u32,
    dispose: Dispose,
    blend: Blend,
) -> FrameGeometry {
    FrameGeometry {
        x_offset,
        y_offset,
        width,
        height,
        dispose,
        blend,
    }
}

fn solid(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
    pixel.repeat((width as usize) * (height as usize))
}

#[test]
fn new_canvas_is_fully_transparent() {
    let compositor = Compositor::new(2, 2);
    assert_eq!(compositor.canvas_pixels(), &[0u8; 2 * 2 * 4]);
}

#[test]
fn source_blend_overwrites_region_including_alpha() {
    let mut compositor = Compositor::new(2, 2);
    let red = solid(2, 2, [255, 0, 0, 128]);
    compositor
        .composite(geometry(0, 0, 2, 2, Dispose::None, Blend::Source), &red)
        .unwrap();
    assert_eq!(compositor.canvas_pixels(), red.as_slice());
}

#[test]
fn over_blend_of_fully_opaque_source_is_a_straight_overwrite() {
    let mut compositor = Compositor::new(1, 1);
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::None, Blend::Source),
            &[10, 20, 30, 255],
        )
        .unwrap();
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::None, Blend::Over),
            &[200, 100, 50, 255],
        )
        .unwrap();
    assert_eq!(compositor.canvas_pixels(), &[200, 100, 50, 255]);
}

#[test]
fn over_blend_of_fully_transparent_source_leaves_destination_unchanged() {
    let mut compositor = Compositor::new(1, 1);
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::None, Blend::Source),
            &[10, 20, 30, 255],
        )
        .unwrap();
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::None, Blend::Over),
            &[9, 9, 9, 0],
        )
        .unwrap();
    assert_eq!(compositor.canvas_pixels(), &[10, 20, 30, 255]);
}

#[test]
fn over_blend_of_half_alpha_source_onto_opaque_destination_matches_hand_computed_formula() {
    let mut compositor = Compositor::new(1, 1);
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::None, Blend::Source),
            &[0, 0, 0, 255],
        )
        .unwrap();
    // src=(255,255,255,128) over dst=(0,0,0,255): out_alpha=1, out_color = src*128/255 + dst*127/255.
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::None, Blend::Over),
            &[255, 255, 255, 128],
        )
        .unwrap();
    let expected = ((128.0f64 / 255.0) * 255.0).round() as u8; // == 128
    let pixel = compositor.canvas_pixels();
    assert_eq!(pixel[3], 255);
    for &channel_value in &pixel[..3] {
        assert!((i16::from(channel_value) - i16::from(expected)).abs() <= 1);
    }
}

#[test]
fn dispose_background_clears_region_before_the_next_frame_only() {
    let mut compositor = Compositor::new(2, 1);
    compositor
        .composite(
            geometry(0, 0, 2, 1, Dispose::Background, Blend::Source),
            &solid(2, 1, [1, 2, 3, 255]),
        )
        .unwrap();
    // Disposal has not happened yet: frame 0 is still fully visible right after compositing it.
    assert_eq!(
        compositor.canvas_pixels(),
        solid(2, 1, [1, 2, 3, 255]).as_slice()
    );

    // Drawing frame 1 over only the left pixel triggers frame 0's deferred BACKGROUND
    // disposal first, so the right pixel (outside frame 1's region) goes transparent.
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::None, Blend::Source),
            &[9, 9, 9, 255],
        )
        .unwrap();
    let pixels = compositor.canvas_pixels();
    assert_eq!(&pixels[0..4], &[9, 9, 9, 255]);
    assert_eq!(&pixels[4..8], &[0, 0, 0, 0]);
}

#[test]
fn dispose_previous_restores_pre_frame_state() {
    let mut compositor = Compositor::new(1, 1);
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::None, Blend::Source),
            &[7, 7, 7, 255],
        )
        .unwrap();
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::Previous, Blend::Source),
            &[200, 0, 0, 255],
        )
        .unwrap();
    // Frame 1 (dispose=Previous) is visible until the next frame draws.
    assert_eq!(compositor.canvas_pixels(), &[200, 0, 0, 255]);
    // Blend::Over with a fully transparent source leaves the destination unchanged, so
    // this demonstrates frame 1's disposal restored frame 0's pixel underneath it.
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::None, Blend::Over),
            &[0, 0, 0, 0],
        )
        .unwrap();
    assert_eq!(compositor.canvas_pixels(), &[7, 7, 7, 255]);
}

#[test]
fn first_frame_previous_is_treated_as_background() {
    let mut compositor = Compositor::new(1, 1);
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::Previous, Blend::Source),
            &[9, 9, 9, 255],
        )
        .unwrap();
    assert_eq!(compositor.canvas_pixels(), &[9, 9, 9, 255]);
    // Per the spec, frame-0-PREVIOUS behaves as BACKGROUND: the region clears to
    // transparent before frame 1, not "restores to before frame 0" (there is no such state).
    compositor
        .composite(
            geometry(0, 0, 1, 1, Dispose::None, Blend::Source),
            &[0, 0, 0, 0],
        )
        .unwrap();
    assert_eq!(compositor.canvas_pixels(), &[0, 0, 0, 0]);
}

#[test]
fn zero_sized_frame_is_malformed() {
    let mut compositor = Compositor::new(2, 2);
    let err = compositor
        .composite(geometry(0, 0, 0, 1, Dispose::None, Blend::Source), &[])
        .unwrap_err();
    assert_eq!(err, Error::Malformed);
}

#[test]
fn out_of_canvas_frame_is_malformed() {
    let mut compositor = Compositor::new(2, 2);
    let err = compositor
        .composite(
            geometry(1, 1, 2, 2, Dispose::None, Blend::Source),
            &solid(2, 2, [1, 1, 1, 1]),
        )
        .unwrap_err();
    assert_eq!(err, Error::Malformed);
}

#[test]
fn mismatched_subframe_length_is_malformed() {
    let mut compositor = Compositor::new(2, 2);
    let err = compositor
        .composite(
            geometry(0, 0, 2, 2, Dispose::None, Blend::Source),
            &[0u8; 4],
        )
        .unwrap_err();
    assert_eq!(err, Error::Malformed);
}
