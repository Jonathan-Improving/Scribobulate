//! Canvas compositing and disposal for GIF frames.
//!
//! The `gif` crate (0.14.x) hands back each frame's own, un-composited
//! rectangle — compositing onto a persistent canvas, and the three-way
//! disposal dance between frames, is richimg's job. This is written
//! in-crate rather than depending on `gif-dispose` (kornelski, MIT/Apache,
//! ~160 SLoC): the algorithm is the standard "keep one canvas plus one
//! saved rectangle" coalesce loop (the same one `ImageMagick -coalesce`
//! and browsers implement), it is small enough to unit-test in full without
//! decoding a single real GIF (every function here takes and returns plain
//! byte buffers), and richimg's `crate::limits::Limits::effective_delay` and
//! `check_pixel_cap` obligations mean the canvas is already sized and capped
//! by the caller — pulling in a dependency to composite a rectangle onto a
//! buffer we already own would not remove any of that surrounding work.
//! POLICY (`sdd/POLICY.md`, "Dependencies") asks for this justification
//! whenever a small enough in-crate alternative exists; ~100 lines below is
//! it.
//!
//! All coordinates here are canvas-space `u32` (already widened from the
//! format's native `u16`), and every rectangle is clipped to the canvas
//! bounds before any of these functions sees it — see [`clip_rect`], the
//! only place clipping happens.

pub(super) const RGBA_CHANNELS: u32 = 4;

/// A frame's placement, clipped to the canvas bounds. Half-open in canvas
/// space: `[x0,x1) x [y0,y1)`. `frame_left`/`frame_top`/`frame_width` are
/// the frame's own UNCLIPPED placement and width, needed to index into the
/// frame's own decoded buffer (which is sized to the frame, not the canvas)
/// even when only part of that buffer lands on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ClippedRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
    pub frame_left: u32,
    pub frame_top: u32,
    pub frame_width: u32,
}

impl ClippedRect {
    fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
}

/// Clips a frame's declared `(left, top, width, height)` rectangle to
/// `[0,canvas_width) x [0,canvas_height)`. A rectangle entirely or partly
/// outside the canvas (GIF places no consistency requirement on this, and
/// `gif::DecodeOptions::check_frame_consistency` is left at its default
/// `false` so such files decode instead of erroring) clips to an empty or
/// partial rect here rather than panicking or indexing out of bounds later.
pub(super) fn clip_rect(
    canvas_width: u32,
    canvas_height: u32,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
) -> ClippedRect {
    let x0 = left.min(canvas_width);
    let y0 = top.min(canvas_height);
    let x1 = left.saturating_add(width).min(canvas_width);
    let y1 = top.saturating_add(height).min(canvas_height);
    ClippedRect {
        x0,
        y0,
        x1,
        y1,
        frame_left: left,
        frame_top: top,
        frame_width: width,
    }
}

fn pixel_offset(row_width: u32, x: u32, y: u32) -> usize {
    ((y * row_width + x) * RGBA_CHANNELS) as usize
}

/// Composites `frame_rgba` (the frame's own, unclipped `frame_width *
/// frame_height` straight-alpha RGBA8 buffer, as decoded by `gif` with
/// `ColorOutput::RGBA`) onto `canvas` at `rect`. A pixel whose alpha is 0 —
/// the GIF transparent index, which `gif`'s RGBA conversion already encodes
/// as alpha 0 — is skipped so it does not overwrite whatever is already on
/// the canvas; every other pixel (GIF has no partial alpha) straight-copies,
/// including its now-guaranteed-255 alpha.
pub(super) fn composite(
    canvas: &mut [u8],
    canvas_width: u32,
    rect: ClippedRect,
    frame_rgba: &[u8],
) {
    const ALPHA_OFFSET: usize = 3;
    for y in rect.y0..rect.y1 {
        let frame_y = y - rect.frame_top;
        for x in rect.x0..rect.x1 {
            let frame_x = x - rect.frame_left;
            let frame_index = pixel_offset(rect.frame_width, frame_x, frame_y);
            let Some(pixel) = frame_rgba.get(frame_index..frame_index + RGBA_CHANNELS as usize)
            else {
                continue; // malformed/short frame buffer: skip rather than panic
            };
            if pixel[ALPHA_OFFSET] == 0 {
                continue; // transparent index: leave the canvas alone
            }
            let canvas_index = pixel_offset(canvas_width, x, y);
            canvas[canvas_index..canvas_index + RGBA_CHANNELS as usize].copy_from_slice(pixel);
        }
    }
}

/// Copies the canvas pixels under `rect` out to an owned buffer, for later
/// [`restore`]. Used only for a `Previous`-disposal frame, taken **before**
/// that frame is [`composite`]d.
pub(super) fn snapshot(canvas: &[u8], canvas_width: u32, rect: ClippedRect) -> Vec<u8> {
    if rect.is_empty() {
        return Vec::new();
    }
    let row_bytes = ((rect.x1 - rect.x0) * RGBA_CHANNELS) as usize;
    let mut out = Vec::with_capacity(row_bytes * (rect.y1 - rect.y0) as usize);
    for y in rect.y0..rect.y1 {
        let start = pixel_offset(canvas_width, rect.x0, y);
        out.extend_from_slice(&canvas[start..start + row_bytes]);
    }
    out
}

/// Writes a [`snapshot`] back onto the canvas at the same `rect` it was
/// taken from — the `Previous` disposal: "restore the canvas to its state
/// before this frame was drawn".
pub(super) fn restore(canvas: &mut [u8], canvas_width: u32, rect: ClippedRect, snapshot: &[u8]) {
    if rect.is_empty() {
        return;
    }
    let row_bytes = ((rect.x1 - rect.x0) * RGBA_CHANNELS) as usize;
    for (row, y) in (rect.y0..rect.y1).enumerate() {
        let start = pixel_offset(canvas_width, rect.x0, y);
        let src_start = row * row_bytes;
        let Some(src) = snapshot.get(src_start..src_start + row_bytes) else {
            continue; // a snapshot shorter than its own rect never happens
                      // in practice (see `snapshot`'s size), but never panic
        };
        canvas[start..start + row_bytes].copy_from_slice(src);
    }
}

/// Clears the canvas under `rect` to fully transparent (`[0,0,0,0]`) — the
/// `Background` disposal. The logical-screen background colour index is
/// deliberately never consulted: WebKit/Blink/Gecko all clear to transparent
/// regardless of it (matches `webp.rs`'s `DISPOSE_CLEAR`, and confirmed for
/// GIF specifically: Blink's `DisposeOverwriteBgcolor` clears the frame rect
/// to fully transparent, Gecko's disposal `CLEAR` likewise).
pub(super) fn clear(canvas: &mut [u8], canvas_width: u32, rect: ClippedRect) {
    if rect.is_empty() {
        return;
    }
    let row_bytes = ((rect.x1 - rect.x0) * RGBA_CHANNELS) as usize;
    for y in rect.y0..rect.y1 {
        let start = pixel_offset(canvas_width, rect.x0, y);
        canvas[start..start + row_bytes].fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS_SIDE: u32 = 4;

    fn blank_canvas() -> Vec<u8> {
        vec![0u8; (CANVAS_SIDE * CANVAS_SIDE * RGBA_CHANNELS) as usize]
    }

    fn opaque_pixel(r: u8, g: u8, b: u8) -> [u8; 4] {
        [r, g, b, 0xFF]
    }

    fn fill_frame(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
        let mut buf = Vec::with_capacity((width * height * RGBA_CHANNELS) as usize);
        for _ in 0..(width * height) {
            buf.extend_from_slice(&pixel);
        }
        buf
    }

    #[test]
    fn clip_rect_is_identity_when_fully_on_canvas() {
        let rect = clip_rect(CANVAS_SIDE, CANVAS_SIDE, 1, 1, 2, 2);
        assert_eq!(
            rect,
            ClippedRect {
                x0: 1,
                y0: 1,
                x1: 3,
                y1: 3,
                frame_left: 1,
                frame_top: 1,
                frame_width: 2
            }
        );
    }

    #[test]
    fn clip_rect_clips_a_rect_extending_past_the_right_and_bottom_edges() {
        let rect = clip_rect(CANVAS_SIDE, CANVAS_SIDE, 2, 3, 5, 5);
        assert_eq!(rect.x0, 2);
        assert_eq!(rect.y0, 3);
        assert_eq!(rect.x1, CANVAS_SIDE);
        assert_eq!(rect.y1, CANVAS_SIDE);
        // The frame's own (unclipped) width is preserved for buffer indexing.
        assert_eq!(rect.frame_width, 5);
    }

    #[test]
    fn clip_rect_handles_a_rect_entirely_off_canvas_without_panicking() {
        let rect = clip_rect(CANVAS_SIDE, CANVAS_SIDE, 10, 10, 2, 2);
        assert!(rect.is_empty());
    }

    #[test]
    fn composite_copies_opaque_pixels_onto_the_canvas() {
        let mut canvas = blank_canvas();
        let rect = clip_rect(CANVAS_SIDE, CANVAS_SIDE, 0, 0, 2, 2);
        let frame = fill_frame(2, 2, opaque_pixel(255, 0, 0));
        composite(&mut canvas, CANVAS_SIDE, rect, &frame);
        let idx = pixel_offset(CANVAS_SIDE, 0, 0);
        assert_eq!(&canvas[idx..idx + 4], &opaque_pixel(255, 0, 0));
        // Untouched pixel elsewhere stays transparent.
        let untouched = pixel_offset(CANVAS_SIDE, 3, 3);
        assert_eq!(&canvas[untouched..untouched + 4], &[0, 0, 0, 0]);
    }

    /// Mutation target: a transparent-index (alpha 0) frame pixel must be
    /// SKIPPED, not copied — this is what proves it "does not overwrite the
    /// canvas". Neutering this check (always copying) would make this test
    /// red: the pre-existing green would become transparent.
    #[test]
    fn composite_skips_transparent_index_pixels_leaving_the_canvas_untouched() {
        let mut canvas = blank_canvas();
        let base_rect = clip_rect(CANVAS_SIDE, CANVAS_SIDE, 0, 0, 1, 1);
        composite(
            &mut canvas,
            CANVAS_SIDE,
            base_rect,
            &fill_frame(1, 1, opaque_pixel(0, 255, 0)),
        );

        let transparent_frame = vec![10u8, 20, 30, 0]; // alpha 0: the GIF transparent index
        composite(&mut canvas, CANVAS_SIDE, base_rect, &transparent_frame);

        let idx = pixel_offset(CANVAS_SIDE, 0, 0);
        assert_eq!(
            &canvas[idx..idx + 4],
            &opaque_pixel(0, 255, 0),
            "a transparent-index pixel must leave the prior canvas content in place"
        );
    }

    #[test]
    fn composite_clips_a_partly_off_canvas_frame_without_panicking() {
        let mut canvas = blank_canvas();
        let rect = clip_rect(CANVAS_SIDE, CANVAS_SIDE, 3, 3, 4, 4);
        let frame = fill_frame(4, 4, opaque_pixel(1, 2, 3));
        composite(&mut canvas, CANVAS_SIDE, rect, &frame);
        let idx = pixel_offset(CANVAS_SIDE, 3, 3);
        assert_eq!(&canvas[idx..idx + 4], &opaque_pixel(1, 2, 3));
    }

    /// Mutation target: `Background` disposal must clear the rect to fully
    /// transparent. Neutering `clear` (making it a no-op) would leave the
    /// prior opaque content in place and this test would go red.
    #[test]
    fn clear_zeroes_the_rect_to_fully_transparent() {
        let mut canvas = blank_canvas();
        let rect = clip_rect(CANVAS_SIDE, CANVAS_SIDE, 0, 0, 2, 2);
        composite(
            &mut canvas,
            CANVAS_SIDE,
            rect,
            &fill_frame(2, 2, opaque_pixel(9, 9, 9)),
        );
        clear(&mut canvas, CANVAS_SIDE, rect);
        let idx = pixel_offset(CANVAS_SIDE, 0, 0);
        assert_eq!(&canvas[idx..idx + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn clear_does_not_touch_pixels_outside_the_rect() {
        let mut canvas = blank_canvas();
        let whole = clip_rect(CANVAS_SIDE, CANVAS_SIDE, 0, 0, CANVAS_SIDE, CANVAS_SIDE);
        composite(
            &mut canvas,
            CANVAS_SIDE,
            whole,
            &fill_frame(CANVAS_SIDE, CANVAS_SIDE, opaque_pixel(5, 6, 7)),
        );
        let small_rect = clip_rect(CANVAS_SIDE, CANVAS_SIDE, 0, 0, 1, 1);
        clear(&mut canvas, CANVAS_SIDE, small_rect);
        let outside = pixel_offset(CANVAS_SIDE, 2, 2);
        assert_eq!(&canvas[outside..outside + 4], &opaque_pixel(5, 6, 7));
    }

    /// Mutation target: `Previous` disposal must restore exactly what the
    /// canvas held before the frame drew. Neutering `restore` (making it a
    /// no-op) would leave the frame's own content in place instead of the
    /// pre-draw snapshot, and this test would go red.
    #[test]
    fn snapshot_then_restore_reverts_a_composited_rect() {
        let mut canvas = blank_canvas();
        let rect = clip_rect(CANVAS_SIDE, CANVAS_SIDE, 0, 0, 2, 2);
        composite(
            &mut canvas,
            CANVAS_SIDE,
            rect,
            &fill_frame(2, 2, opaque_pixel(1, 1, 1)),
        );

        let saved = snapshot(&canvas, CANVAS_SIDE, rect);
        composite(
            &mut canvas,
            CANVAS_SIDE,
            rect,
            &fill_frame(2, 2, opaque_pixel(2, 2, 2)),
        );
        let idx = pixel_offset(CANVAS_SIDE, 0, 0);
        assert_eq!(&canvas[idx..idx + 4], &opaque_pixel(2, 2, 2));

        restore(&mut canvas, CANVAS_SIDE, rect, &saved);
        assert_eq!(&canvas[idx..idx + 4], &opaque_pixel(1, 1, 1));
    }

    #[test]
    fn snapshot_of_the_initial_transparent_canvas_restores_to_transparent() {
        let canvas = blank_canvas();
        let rect = clip_rect(CANVAS_SIDE, CANVAS_SIDE, 0, 0, 2, 2);
        let saved = snapshot(&canvas, CANVAS_SIDE, rect);

        let mut drawn = canvas.clone();
        composite(
            &mut drawn,
            CANVAS_SIDE,
            rect,
            &fill_frame(2, 2, opaque_pixel(9, 9, 9)),
        );
        restore(&mut drawn, CANVAS_SIDE, rect, &saved);

        let idx = pixel_offset(CANVAS_SIDE, 0, 0);
        assert_eq!(&drawn[idx..idx + 4], &[0, 0, 0, 0]);
    }
}
