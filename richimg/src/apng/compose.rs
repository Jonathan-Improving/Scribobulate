//! Pure APNG canvas compositing: no decoding, no I/O, no `png`-crate types —
//! just the three dispose ops and two blend ops from the APNG spec
//! (<https://wiki.mozilla.org/APNG_Specification>, "`fcTL`: The Frame Control
//! Chunk" and "Chunk Sequence Numbers"), applied to a persistent full-canvas
//! RGBA8 **straight-alpha** buffer. `crate::apng` owns the `png`-crate
//! decoding and translates its `FrameControl` into [`FrameGeometry`] before
//! calling in here, so this module is decoder-agnostic and independently
//! testable.

use crate::error::Error;

pub(crate) const RGBA_CHANNELS: usize = 4;

/// Fully transparent black, straight alpha — the canvas's initial state and
/// what `Dispose::Background` clears a region back to.
const TRANSPARENT_PIXEL: [u8; RGBA_CHANNELS] = [0, 0, 0, 0];

/// One frame's disposal instruction (APNG spec `fcTL.dispose_op`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dispose {
    /// Leave the canvas exactly as this frame left it.
    None,
    /// Clear this frame's region to transparent black before the next frame
    /// is drawn.
    Background,
    /// Restore this frame's region to its state from just before this frame
    /// was drawn, before the next frame is drawn.
    Previous,
}

/// One frame's blend instruction (APNG spec `fcTL.blend_op`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Blend {
    /// Overwrite the region, alpha included.
    Source,
    /// Composite onto the region with the spec's straight-alpha "over"
    /// formula.
    Over,
}

/// One frame's placement and compositing instructions, decoupled from
/// `png::FrameControl` so this module carries no decoder dependency.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FrameGeometry {
    pub x_offset: u32,
    pub y_offset: u32,
    pub width: u32,
    pub height: u32,
    pub dispose: Dispose,
    pub blend: Blend,
}

/// A rectangular canvas region in pixel coordinates.
#[derive(Debug, Clone, Copy)]
struct RegionRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Rejects a frame whose region does not fit the canvas, or that has no
/// area — the APNG spec requires every subframe to lie within the canvas
/// declared by `IHDR`/`acTL`, and a zero-sized frame cannot be composited.
fn validate_geometry(
    canvas_width: u32,
    canvas_height: u32,
    geometry: &FrameGeometry,
) -> Result<(), Error> {
    if geometry.width == 0 || geometry.height == 0 {
        return Err(Error::Malformed);
    }
    let right = geometry
        .x_offset
        .checked_add(geometry.width)
        .ok_or(Error::Malformed)?;
    let bottom = geometry
        .y_offset
        .checked_add(geometry.height)
        .ok_or(Error::Malformed)?;
    if right > canvas_width || bottom > canvas_height {
        return Err(Error::Malformed);
    }
    Ok(())
}

/// What to do to the canvas just before the **next** frame is drawn,
/// deferred from the frame that requested it. Every reference decoder
/// applies disposal lazily this way: it is never visible on the frame that
/// requests it, only once the following frame's own region fails to fully
/// cover it.
enum PendingDispose {
    None,
    Background {
        region: RegionRect,
    },
    Previous {
        region: RegionRect,
        /// The region's pixels as they stood immediately before the frame
        /// that requested `Previous` was drawn.
        snapshot: Vec<u8>,
    },
}

/// The persistent, full-canvas composited buffer plus its pending disposal.
pub(crate) struct Compositor {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    pending: PendingDispose,
    /// How many frames have been composited so far — needed only to apply
    /// the spec's frame-0 exception (see [`Self::composite`]).
    frames_composited: u32,
}

impl Compositor {
    /// A fresh, fully transparent canvas — also what `rewind` conceptually
    /// produces (`crate::apng` rebuilds a whole new `Compositor` rather than
    /// clearing this one in place, since a rebuild is needed for the decoder
    /// anyway).
    pub fn new(width: u32, height: u32) -> Compositor {
        let len = (width as usize) * (height as usize) * RGBA_CHANNELS;
        Compositor {
            width,
            height,
            pixels: vec![0u8; len],
            pending: PendingDispose::None,
            frames_composited: 0,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn canvas_pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Composites one already-decoded, already-RGBA8-straight-alpha subframe
    /// onto the canvas at `geometry`. `subframe_rgba` must be exactly
    /// `geometry.width * geometry.height * 4` bytes, row-major.
    pub fn composite(
        &mut self,
        geometry: FrameGeometry,
        subframe_rgba: &[u8],
    ) -> Result<(), Error> {
        validate_geometry(self.width, self.height, &geometry)?;
        let expected_len = (geometry.width as usize) * (geometry.height as usize) * RGBA_CHANNELS;
        if subframe_rgba.len() != expected_len {
            return Err(Error::Malformed);
        }

        self.apply_pending_dispose();

        let region = RegionRect {
            x: geometry.x_offset,
            y: geometry.y_offset,
            width: geometry.width,
            height: geometry.height,
        };

        // APNG spec: "if the first frame uses a dispose_op of
        // APNG_DISPOSE_OP_PREVIOUS it should be treated as
        // APNG_DISPOSE_OP_BACKGROUND" — there is no prior state to restore.
        let effective_dispose =
            if self.frames_composited == 0 && geometry.dispose == Dispose::Previous {
                Dispose::Background
            } else {
                geometry.dispose
            };

        let snapshot = match effective_dispose {
            Dispose::Previous => Some(self.copy_region(&region)),
            Dispose::None | Dispose::Background => None,
        };

        match geometry.blend {
            Blend::Source => self.blend_source(&region, subframe_rgba),
            Blend::Over => self.blend_over(&region, subframe_rgba),
        }

        self.pending = match (effective_dispose, snapshot) {
            (Dispose::None, _) => PendingDispose::None,
            (Dispose::Background, _) => PendingDispose::Background { region },
            (Dispose::Previous, Some(snapshot)) => PendingDispose::Previous { region, snapshot },
            // Unreachable: `snapshot` is always `Some` exactly when
            // `effective_dispose` is `Previous` (see the match just above).
            (Dispose::Previous, None) => PendingDispose::None,
        };
        self.frames_composited += 1;
        Ok(())
    }

    fn apply_pending_dispose(&mut self) {
        match std::mem::replace(&mut self.pending, PendingDispose::None) {
            PendingDispose::None => {}
            PendingDispose::Background { region } => self.clear_region(&region),
            PendingDispose::Previous { region, snapshot } => self.paste_region(&region, &snapshot),
        }
    }

    fn row_offset(&self, x: u32, y: u32) -> usize {
        ((y as usize) * (self.width as usize) + (x as usize)) * RGBA_CHANNELS
    }

    fn copy_region(&self, region: &RegionRect) -> Vec<u8> {
        let mut out =
            Vec::with_capacity((region.width as usize) * (region.height as usize) * RGBA_CHANNELS);
        for row in 0..region.height {
            let start = self.row_offset(region.x, region.y + row);
            let len = (region.width as usize) * RGBA_CHANNELS;
            out.extend_from_slice(&self.pixels[start..start + len]);
        }
        out
    }

    fn paste_region(&mut self, region: &RegionRect, snapshot: &[u8]) {
        let row_len = (region.width as usize) * RGBA_CHANNELS;
        for row in 0..region.height {
            let start = self.row_offset(region.x, region.y + row);
            let src = &snapshot[(row as usize) * row_len..(row as usize) * row_len + row_len];
            self.pixels[start..start + row_len].copy_from_slice(src);
        }
    }

    fn clear_region(&mut self, region: &RegionRect) {
        for row in 0..region.height {
            let start = self.row_offset(region.x, region.y + row);
            for pixel in self.pixels[start..start + (region.width as usize) * RGBA_CHANNELS]
                .chunks_exact_mut(RGBA_CHANNELS)
            {
                pixel.copy_from_slice(&TRANSPARENT_PIXEL);
            }
        }
    }

    fn blend_source(&mut self, region: &RegionRect, subframe_rgba: &[u8]) {
        let row_len = (region.width as usize) * RGBA_CHANNELS;
        for row in 0..region.height {
            let dst_start = self.row_offset(region.x, region.y + row);
            let src = &subframe_rgba[(row as usize) * row_len..(row as usize) * row_len + row_len];
            self.pixels[dst_start..dst_start + row_len].copy_from_slice(src);
        }
    }

    fn blend_over(&mut self, region: &RegionRect, subframe_rgba: &[u8]) {
        let row_len = (region.width as usize) * RGBA_CHANNELS;
        for row in 0..region.height {
            let dst_start = self.row_offset(region.x, region.y + row);
            let src_row =
                &subframe_rgba[(row as usize) * row_len..(row as usize) * row_len + row_len];
            for (dst_pixel, src_pixel) in self.pixels[dst_start..dst_start + row_len]
                .chunks_exact_mut(RGBA_CHANNELS)
                .zip(src_row.chunks_exact(RGBA_CHANNELS))
            {
                let blended = over(
                    [dst_pixel[0], dst_pixel[1], dst_pixel[2], dst_pixel[3]],
                    [src_pixel[0], src_pixel[1], src_pixel[2], src_pixel[3]],
                );
                dst_pixel.copy_from_slice(&blended);
            }
        }
    }
}

/// The APNG spec's straight-alpha (non-premultiplied) "over" operator:
///
/// ```text
/// blend.alpha = src.alpha + dst.alpha * (1 - src.alpha)
/// if blend.alpha == 0 { blend.color = 0 } else {
///     blend.color = (src.color*src.alpha + dst.color*dst.alpha*(1-src.alpha)) / blend.alpha
/// }
/// ```
///
/// (<https://wiki.mozilla.org/APNG_Specification>, "`fcTL`: The Frame
/// Control Chunk" — `APNG_BLEND_OP_OVER`). Computed in `f64` and rounded to
/// the nearest `u8`; richimg's own test tolerance (`CHANNEL_TOLERANCE = 1` in
/// `tests/support/mod.rs`) absorbs the last-bit rounding difference against
/// a fixed-point reference decoder such as `ffmpeg`.
fn over(dst: [u8; RGBA_CHANNELS], src: [u8; RGBA_CHANNELS]) -> [u8; RGBA_CHANNELS] {
    const MAX_CHANNEL: f64 = 255.0;
    let src_alpha = f64::from(src[3]) / MAX_CHANNEL;
    let dst_alpha = f64::from(dst[3]) / MAX_CHANNEL;
    let out_alpha = src_alpha + dst_alpha * (1.0 - src_alpha);

    if out_alpha <= 0.0 {
        return TRANSPARENT_PIXEL;
    }

    let blend_channel = |src_channel: u8, dst_channel: u8| -> u8 {
        let src_channel = f64::from(src_channel) / MAX_CHANNEL;
        let dst_channel = f64::from(dst_channel) / MAX_CHANNEL;
        let out_channel =
            (src_channel * src_alpha + dst_channel * dst_alpha * (1.0 - src_alpha)) / out_alpha;
        (out_channel * MAX_CHANNEL).round().clamp(0.0, MAX_CHANNEL) as u8
    };

    [
        blend_channel(src[0], dst[0]),
        blend_channel(src[1], dst[1]),
        blend_channel(src[2], dst[2]),
        (out_alpha * MAX_CHANNEL).round().clamp(0.0, MAX_CHANNEL) as u8,
    ]
}

#[cfg(test)]
mod tests;
