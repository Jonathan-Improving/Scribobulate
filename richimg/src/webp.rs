//! WebP codec (`image-webp` 0.2.4), still and animated. Carried obligations
//! from `sdd/PLAN.memory-gates.md` ("Decoder: image-webp 0.2.4") that this
//! module exists to satisfy:
//!
//! - Dispose-to-background clears to transparent ([`DISPOSE_CLEAR`]); 0.2.4 makes
//!   it a no-op unless a colour is set.
//! - `rewind` recreates the decoder from the shared bytes, because
//!   `reset_animation` does not clear the canvas (only rewinds the ANMF pointer).
//! - `set_memory_limit` as defence in depth, never the pixel cap itself (that
//!   check is central, in `crate::check_pixel_cap`).
//! - Straight alpha; an RGB8 source (no alpha in the file) is expanded to RGBA8
//!   with alpha 255.
use std::io::{BufRead, Cursor, Seek};
use std::sync::Arc;
use std::time::Duration;

use image_webp::{DecodingError, LoopCount as WebpLoopCount, WebPDecoder};

use crate::codec::Codec;
use crate::error::Error;
use crate::limits::Limits;
use crate::types::{Frame, Info, LoopCount};

/// Extra headroom above `max_pixels * 4` bytes for `set_memory_limit`, to cover
/// incidental chunk reads (ICCP/EXIF/XMP/ANIM) that are not part of the pixel
/// canvas. Deliberately generous: this limit is defence in depth, never the cap.
const MEMORY_LIMIT_HEADROOM_BYTES: u64 = 16 * 1024 * 1024;
const RGBA_CHANNELS: u64 = 4;
const RGB_CHANNELS: usize = 3;
const RGBA_CHANNELS_USIZE: usize = 4;
const OPAQUE_ALPHA: u8 = 255;
const STILL_IMAGE_FRAME_COUNT: u32 = 1;
const STILL_IMAGE_LOOP_COUNT: u32 = 1;
const STILL_IMAGE_INDEX: u32 = 0;
/// A still image has no declared per-frame delay; treat it as the shortest
/// possible declaration (0 ms) so the same floor logic applies uniformly.
const STILL_IMAGE_DECLARED_DELAY_MS: u32 = 0;

type Reader = Cursor<Arc<[u8]>>;

pub(crate) struct WebpCodec {
    bytes: Arc<[u8]>,
    decoder: WebPDecoder<Reader>,
    info: Info,
    memory_limit: usize,
    limits: Limits,
    next_index: u32,
}

/// Reads the headers through a borrowed cursor: a probe copies no bytes and
/// allocates no pixels.
pub(crate) fn probe(bytes: &[u8], limits: &Limits) -> Result<Info, Error> {
    let decoder = new_decoder(Cursor::new(bytes), memory_limit_bytes(limits))?;
    Ok(build_info(&decoder))
}

pub(crate) fn open(bytes: Arc<[u8]>, limits: &Limits) -> Result<Box<dyn Codec>, Error> {
    let memory_limit = memory_limit_bytes(limits);
    let mut decoder = new_decoder(Cursor::new(Arc::clone(&bytes)), memory_limit)?;
    apply_dispose_clear(&mut decoder)?;
    let info = build_info(&decoder);
    Ok(Box::new(WebpCodec {
        bytes,
        decoder,
        info,
        memory_limit,
        limits: *limits,
        next_index: 0,
    }))
}

fn new_decoder<R: BufRead + Seek>(reader: R, memory_limit: usize) -> Result<WebPDecoder<R>, Error> {
    let mut decoder = WebPDecoder::new(reader).map_err(map_decoding_error)?;
    decoder.set_memory_limit(memory_limit);
    Ok(decoder)
}

/// Dispose-to-background clears to TRANSPARENT, as libwebp's `WebPAnimDecoder`
/// (`ZeroFillFrameRect`), Blink, Gecko and WebKit all do: the ANIM background
/// colour is a hint every reference renderer ignores. 0.2.4 makes the dispose a
/// no-op unless a colour is set, so one is — and it must not be
/// `background_color_hint()`, which is also in on-disk BGRA order.
const DISPOSE_CLEAR: [u8; 4] = [0, 0, 0, 0];

/// Applies [`DISPOSE_CLEAR`] to an animated decoder; a still image has no
/// disposal, and image-webp refuses the call there.
fn apply_dispose_clear<R: BufRead + Seek>(decoder: &mut WebPDecoder<R>) -> Result<(), Error> {
    if decoder.is_animated() {
        decoder
            .set_background_color(DISPOSE_CLEAR)
            .map_err(map_decoding_error)?;
    }
    Ok(())
}

fn memory_limit_bytes(limits: &Limits) -> usize {
    let pixel_bytes = limits.max_pixels.saturating_mul(RGBA_CHANNELS);
    let with_headroom = pixel_bytes.saturating_add(MEMORY_LIMIT_HEADROOM_BYTES);
    usize::try_from(with_headroom).unwrap_or(usize::MAX)
}

fn build_info<R: BufRead + Seek>(decoder: &WebPDecoder<R>) -> Info {
    let (width, height) = decoder.dimensions();
    let animated = decoder.is_animated();
    let frame_count = Some(if animated {
        decoder.num_frames()
    } else {
        STILL_IMAGE_FRAME_COUNT
    });
    let loop_count = if animated {
        match decoder.loop_count() {
            WebpLoopCount::Forever => LoopCount::Infinite,
            WebpLoopCount::Times(times) => LoopCount::Finite(u32::from(times.get())),
        }
    } else {
        LoopCount::Finite(STILL_IMAGE_LOOP_COUNT)
    };
    Info {
        width,
        height,
        animated,
        frame_count,
        loop_count,
    }
}

fn map_decoding_error(_err: DecodingError) -> Error {
    // image-webp's DecodingError is #[non_exhaustive] and every variant here
    // (truncated input, a bad chunk header, a memory-limit trip, ...) is a
    // decode failure from richimg's point of view. A panic on crafted input
    // never reaches this function — it is caught centrally by `crate::guarded`.
    Error::Malformed
}

fn expand_to_rgba(buf: Vec<u8>, has_alpha: bool) -> Vec<u8> {
    if has_alpha {
        return buf;
    }
    let mut rgba = Vec::with_capacity(buf.len() / RGB_CHANNELS * RGBA_CHANNELS_USIZE);
    for pixel in buf.chunks_exact(RGB_CHANNELS) {
        rgba.extend_from_slice(pixel);
        rgba.push(OPAQUE_ALPHA);
    }
    rgba
}

impl WebpCodec {
    fn still_frame(&mut self) -> Result<Frame, Error> {
        let output_len = self.decoder.output_buffer_size().ok_or(Error::TooLarge)?;
        let mut buf = vec![0u8; output_len];
        self.decoder
            .read_image(&mut buf)
            .map_err(map_decoding_error)?;
        let rgba = expand_to_rgba(buf, self.decoder.has_alpha());
        Ok(Frame {
            width: self.info.width,
            height: self.info.height,
            rgba,
            delay: self.limits.effective_delay(Duration::from_millis(u64::from(
                STILL_IMAGE_DECLARED_DELAY_MS,
            ))),
            index: STILL_IMAGE_INDEX,
        })
    }

    fn animated_frame(&mut self) -> Result<Frame, Error> {
        let total_frames = self.info.frame_count.unwrap_or(0);
        if self.next_index >= total_frames {
            self.rewind();
        }
        let output_len = self.decoder.output_buffer_size().ok_or(Error::TooLarge)?;
        let mut buf = vec![0u8; output_len];
        let delay_ms = self
            .decoder
            .read_frame(&mut buf)
            .map_err(map_decoding_error)?;
        let rgba = expand_to_rgba(buf, self.decoder.has_alpha());
        let index = self.next_index;
        self.next_index += 1;
        Ok(Frame {
            width: self.info.width,
            height: self.info.height,
            rgba,
            delay: self
                .limits
                .effective_delay(Duration::from_millis(u64::from(delay_ms))),
            index,
        })
    }
}

impl Codec for WebpCodec {
    fn info(&self) -> &Info {
        &self.info
    }

    fn next_frame(&mut self) -> Result<Frame, Error> {
        if self.info.animated {
            self.animated_frame()
        } else {
            self.still_frame()
        }
    }

    fn rewind(&mut self) {
        // `reset_animation` only rewinds image-webp's internal ANMF pointer —
        // it does not clear the composited canvas — so a loop whose frame 0 is
        // partial and transparent would otherwise composite over the previous
        // loop's last frame. Recreating the decoder from the shared bytes is
        // the clean way to get a genuinely cleared canvas.
        match new_decoder(Cursor::new(Arc::clone(&self.bytes)), self.memory_limit) {
            Ok(mut fresh) => {
                // Same bytes, same answer as at open: an error here cannot occur
                // for a decoder that already opened, and is ignored rather than
                // unwrapped.
                let _ = apply_dispose_clear(&mut fresh);
                self.decoder = fresh;
            }
            Err(_) => {
                // Unreachable in practice (the same bytes decoded successfully
                // to open this codec), but fall back to image-webp's own
                // rewind rather than leaving the decoder unusable.
                if self.info.animated {
                    self.decoder.reset_animation();
                }
            }
        }
        self.next_index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_to_rgba_is_identity_when_already_alpha() {
        let buf = vec![1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(expand_to_rgba(buf.clone(), true), buf);
    }

    #[test]
    fn expand_to_rgba_appends_opaque_alpha_for_rgb() {
        let buf = vec![10, 20, 30, 40, 50, 60];
        let expanded = expand_to_rgba(buf, false);
        assert_eq!(expanded, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }
}
