//! APNG codec (`png` 0.18.1). Composites full-canvas RGBA8 straight-alpha
//! frames per the APNG spec (<https://wiki.mozilla.org/APNG_Specification>):
//! three dispose ops (none, background, previous) and two blend ops (source,
//! over) live in [`compose`]; this module owns decoding through the `png`
//! crate and the two APNG-specific rules from `sdd/PLAN.memory-gates.md`
//! ("APNG animates too, through the pure-Rust `png` crate"):
//!
//! - **The default image may not be a frame.** When no `fcTL` precedes
//!   `IDAT`, that `IDAT` is the non-APNG-aware fallback, not part of the
//!   animation.
//! - **Delays are fractions** (`delay_num / delay_den`, `delay_den == 0`
//!   meaning 1/100 s), floored the same way as every other codec
//!   ([`Limits::effective_delay`]).
//!
//! ## Verified against the `png` 0.18.1 source (not general knowledge)
//!
//! Per the plan's demand to verify rather than assume `png`'s APNG support:
//!
//! - `Reader::read_info` populates `Info::animation_control` from `acTL` and
//!   `Info::frame_control` from a leading `fcTL`, if any
//!   (`png-0.18.1/src/decoder/mod.rs:229-242`). The crate's own comment there
//!   states the default-image rule verbatim: "No `fcTL` before `IDAT` =>
//!   `IDAT` is not part of the animation, but represents an *extra*, default
//!   frame for non-APNG-aware decoders" — and `AnimationControl::num_frames`
//!   (the `acTL` field) already excludes that default image.
//! - `Reader::next_frame_info` (`decoder/mod.rs:341-364`) advances past the
//!   current subframe, discarding its pixel data via `finish_decoding`
//!   without our ever allocating a buffer for it — exactly what skips the
//!   default image for free when one is present.
//! - `Reader::next_frame` (`decoder/mod.rs:416-488`) requires a buffer sized
//!   to `output_buffer_size()`, which is the **full IHDR canvas** size
//!   (`decoder/mod.rs:668-680` reads `self.info().size()`, not the
//!   subframe's), even though only the leading `subframe.height` rows (each
//!   `OutputInfo::line_size`, sized from the **subframe** width) are
//!   actually written (`decoder/mod.rs:444-482`). So this module allocates
//!   the oversized buffer `next_frame` demands, then reads only the first
//!   `OutputInfo::buffer_size()` bytes back out as the real subframe pixels
//!   (see [`ApngCodec::decode_one_frame`]) — a carried obligation like
//!   `image-webp`'s in `webp.rs`, just for a different crate.
//! - The `png` crate's `Transformations` (`common.rs:895-906`) does **not**
//!   include libpng's `GRAY_TO_RGB` (only `IDENTITY`, `STRIP_16`, `EXPAND`,
//!   `ALPHA` exist); `create_transform_fn` (`decoder/transform.rs:30-77`)
//!   confirms grayscale/grayscale+alpha are never turned into RGB by the
//!   crate. So with `EXPAND | ALPHA | STRIP_16` requested,
//!   `Reader::output_color_type` (`decoder/mod.rs:629-661`) can only ever
//!   yield `Rgba` (from `Rgb`, `Rgba` or `Indexed`, all forced to carry
//!   alpha) or `GrayscaleAlpha` (from `Grayscale` or `GrayscaleAlpha`), both
//!   always 8-bit — [`normalize_to_rgba`] does the remaining gray-to-RGB
//!   expansion `png` does not.
//! - `acTL.num_plays` is **total plays**: `0` means infinite, `N > 0` means
//!   `Finite(N)` (`AnimationControl::num_plays` doc, `common.rs:299-306`:
//!   "Number of times to loop this APNG. 0 indicates infinite looping."; also
//!   matches Blink's `png_image_reader.cc`
//!   `SetRepetitionCount(repetition_count - 1)`, whose own counter is
//!   *extra* loops beyond the first play).
//! - **Fuzzing**: `png` 0.18.1's own `README.md` states "No `unsafe` code,
//!   battle-tested, and fuzzed on [OSS-Fuzz](https://github.com/google/oss-fuzz)"
//!   (`png-0.18.1/README.md:9`) — like `image-webp`, `#![forbid(unsafe_code)]`
//!   (`png-0.18.1/src/lib.rs`).
//! - `png::Decoder::set_limits` (`decoder/mod.rs:65-87,177-179`) is `png`'s
//!   own resource ceiling, used here exactly as `image-webp`'s
//!   `set_memory_limit` is in `webp.rs`: defence in depth, never the pixel
//!   cap itself (that check is centralised in `crate::check_pixel_cap`).
//! - `png` itself already rejects an out-of-canvas or zero-sized `fcTL`
//!   (`FormatErrorInner::BadSubFrameBounds`/`InvalidDimensions`,
//!   `decoder/stream.rs:1963-1998`) and an out-of-order `fcTL`/`fdAT`
//!   sequence number (`FormatErrorInner::ApngOrder`, `decoder/stream.rs:1013-1022,1173-1188`),
//!   both surfacing as a `DecodingError` this module maps to
//!   [`Error::Malformed`]. [`compose::Compositor::composite`] re-validates
//!   geometry anyway, as defence in depth for [`normalize_to_rgba`]'s own
//!   slicing.

mod compose;

use std::io::{BufRead, Cursor, Seek};
use std::sync::Arc;
use std::time::Duration;

use png::{
    BitDepth, BlendOp, ColorType, Decoder, DecodingError, DisposeOp, FrameControl,
    Reader as PngReader, Transformations,
};

use crate::codec::Codec;
use crate::error::Error;
use crate::limits::Limits;
use crate::types::{Frame, Info, LoopCount};

use compose::{Blend, Compositor, Dispose, FrameGeometry, RGBA_CHANNELS};

/// Extra headroom above `max_pixels * 4` bytes for `png::Limits::bytes`, to
/// cover incidental chunk reads (palette, tRNS, text, `acTL`/`fcTL`
/// themselves) that are not part of the pixel canvas. Deliberately generous,
/// exactly like `webp.rs`'s `MEMORY_LIMIT_HEADROOM_BYTES`: this limit is
/// defence in depth, never the cap.
const MEMORY_LIMIT_HEADROOM_BYTES: u64 = 16 * 1024 * 1024;

const GRAY_ALPHA_CHANNELS: usize = 2;
const STILL_FRAME_COUNT: u32 = 1;
const NANOS_PER_SECOND: u64 = 1_000_000_000;
/// APNG spec: `delay_den == 0` means the delay is in 1/100ths of a second.
const DELAY_DEN_ZERO_SUBSTITUTE: u16 = 100;

type Reader = PngReader<Cursor<Arc<[u8]>>>;

pub(crate) struct ApngCodec {
    bytes: Arc<[u8]>,
    reader: Reader,
    info: Info,
    limits: Limits,
    frame_count: u32,
    next_index: u32,
    /// Whether the leading `IDAT` itself carries a preceding `fcTL` (so it
    /// *is* frame 0) rather than being the non-animated default image.
    has_leading_frame_control: bool,
    compositor: Compositor,
    /// Cached frame 0 for the `frame_count == 1` "still" case: `png::Reader`
    /// is a forward-only iterator (unlike `image-webp`, which lets
    /// `webp.rs`'s still path just call `read_image` again), so a second
    /// "decode" is a clone of this instead of a second pass over the reader.
    still_cache: Option<Frame>,
}

/// Reads the headers through a borrowed cursor: a probe copies no bytes,
/// decodes no pixel row, and allocates no canvas.
pub(crate) fn probe(bytes: &[u8], limits: &Limits) -> Result<Info, Error> {
    let reader = build_reader(Cursor::new(bytes), byte_limit(limits))?;
    build_info(&reader)
}

pub(crate) fn open(bytes: Arc<[u8]>, limits: &Limits) -> Result<Box<dyn Codec>, Error> {
    Ok(Box::new(ApngCodec::build(bytes, limits)?))
}

fn build_reader<R: BufRead + Seek>(reader: R, byte_limit: usize) -> Result<PngReader<R>, Error> {
    let mut decoder = Decoder::new_with_limits(reader, png::Limits { bytes: byte_limit });
    // EXPAND: paletted -> RGB(A), sub-8-bit gray -> 8-bit gray, tRNS -> alpha.
    // ALPHA: force an alpha channel even where EXPAND alone would not add one
    // (opaque RGB/gray get alpha 255; see `normalize_to_rgba`'s doc comment
    // for the two color types this combination can still leave behind).
    // STRIP_16: truncate a 16-bit sample to its high byte (the standard PNG
    // 16->8 reduction; `decoder/transform.rs`'s `transform_row_strip16`).
    decoder.set_transformations(
        Transformations::EXPAND | Transformations::ALPHA | Transformations::STRIP_16,
    );
    decoder.read_info().map_err(map_decoding_error)
}

fn build_info<R: BufRead + Seek>(reader: &PngReader<R>) -> Result<Info, Error> {
    let (width, height) = reader.info().size();
    let animation = reader.info().animation_control.ok_or(Error::Malformed)?;
    if animation.num_frames == 0 {
        return Err(Error::Malformed);
    }
    Ok(Info {
        width,
        height,
        animated: true,
        frame_count: Some(animation.num_frames),
        loop_count: loop_count_from(animation.num_plays),
    })
}

/// `acTL.num_plays` is the TOTAL number of plays (`0` means infinite),
/// confirmed against `png`'s own doc comment on `AnimationControl::num_plays`
/// and Blink's `png_image_reader.cc` (`SetRepetitionCount(repetition_count -
/// 1)`, whose counter is extra loops beyond the first play — i.e. Blink's
/// `repetition_count` is also total plays before that adjustment).
fn loop_count_from(num_plays: u32) -> LoopCount {
    if num_plays == 0 {
        LoopCount::Infinite
    } else {
        LoopCount::Finite(num_plays)
    }
}

/// `delay_num / delay_den` seconds, `delay_den == 0` meaning `1/100` s
/// (APNG spec, `fcTL`). Computed as exact integer nanoseconds — no `f64`, so
/// no float drift for common fractions such as `7/100` or `1/3`.
fn delay_duration(delay_num: u16, delay_den: u16) -> Duration {
    let den = if delay_den == 0 {
        u64::from(DELAY_DEN_ZERO_SUBSTITUTE)
    } else {
        u64::from(delay_den)
    };
    let nanos = u64::from(delay_num) * NANOS_PER_SECOND / den;
    Duration::from_nanos(nanos)
}

fn to_geometry(fctl: &FrameControl) -> FrameGeometry {
    FrameGeometry {
        x_offset: fctl.x_offset,
        y_offset: fctl.y_offset,
        width: fctl.width,
        height: fctl.height,
        dispose: match fctl.dispose_op {
            DisposeOp::None => Dispose::None,
            DisposeOp::Background => Dispose::Background,
            DisposeOp::Previous => Dispose::Previous,
        },
        blend: match fctl.blend_op {
            BlendOp::Source => Blend::Source,
            BlendOp::Over => Blend::Over,
        },
    }
}

/// Expands a `png`-decoded subframe (already forced through `EXPAND | ALPHA
/// | STRIP_16`) into straight-alpha RGBA8. Per this module's doc comment,
/// that transformation set can only ever produce `Rgba` or `GrayscaleAlpha`
/// (never plain `Grayscale`/`Rgb`/`Indexed`, and never above 8 bits) — `png`
/// itself has no gray-to-RGB expansion, so that half is done here.
fn normalize_to_rgba(
    buf: &[u8],
    color_type: ColorType,
    bit_depth: BitDepth,
) -> Result<Vec<u8>, Error> {
    if bit_depth != BitDepth::Eight {
        // Unreachable given `STRIP_16 | EXPAND | ALPHA` (see this module's
        // and `normalize_to_rgba`'s doc comments) — defence in depth against
        // a future `png` version changing that mapping.
        return Err(Error::Malformed);
    }
    match color_type {
        ColorType::Rgba => Ok(buf.to_vec()),
        ColorType::GrayscaleAlpha => {
            if !buf.len().is_multiple_of(GRAY_ALPHA_CHANNELS) {
                return Err(Error::Malformed);
            }
            let mut rgba = Vec::with_capacity(buf.len() / GRAY_ALPHA_CHANNELS * RGBA_CHANNELS);
            for pixel in buf.chunks_exact(GRAY_ALPHA_CHANNELS) {
                let gray = pixel[0];
                let alpha = pixel[1];
                rgba.extend_from_slice(&[gray, gray, gray, alpha]);
            }
            Ok(rgba)
        }
        _ => Err(Error::Malformed),
    }
}

fn map_decoding_error(_err: DecodingError) -> Error {
    // `png`'s `DecodingError` is `#[non_exhaustive]` and every variant here
    // (truncated input, a bad chunk, an out-of-order `fcTL`/`fdAT` sequence
    // number, an out-of-canvas `fcTL`, a `png::Limits` trip, ...) is a
    // decode failure from richimg's point of view. A panic on crafted input
    // never reaches this function — it is caught centrally by
    // `crate::guarded`.
    Error::Malformed
}

fn byte_limit(limits: &Limits) -> usize {
    let pixel_bytes = limits.max_pixels.saturating_mul(RGBA_CHANNELS as u64);
    let with_headroom = pixel_bytes.saturating_add(MEMORY_LIMIT_HEADROOM_BYTES);
    usize::try_from(with_headroom).unwrap_or(usize::MAX)
}

impl ApngCodec {
    fn build(bytes: Arc<[u8]>, limits: &Limits) -> Result<ApngCodec, Error> {
        let limit = byte_limit(limits);
        let reader = build_reader(Cursor::new(Arc::clone(&bytes)), limit)?;
        let info = build_info(&reader)?;
        let frame_count = info.frame_count.ok_or(Error::Malformed)?;
        let has_leading_frame_control = reader.info().frame_control.is_some();
        Ok(ApngCodec {
            bytes,
            reader,
            compositor: Compositor::new(info.width, info.height),
            info,
            limits: *limits,
            frame_count,
            next_index: 0,
            has_leading_frame_control,
            still_cache: None,
        })
    }

    /// Returns the `fcTL` that governs the frame about to be decoded,
    /// advancing the reader as needed. Frame 0 either already has its
    /// control info loaded (a leading `fcTL`) or requires skipping the
    /// default image first (see this module's doc comment); every later
    /// frame is a plain `next_frame_info` call.
    fn advance_to_next_frame_control(&mut self) -> Result<FrameControl, Error> {
        if self.next_index == 0 && self.has_leading_frame_control {
            return self.reader.info().frame_control.ok_or(Error::Malformed);
        }
        self.reader
            .next_frame_info()
            .copied()
            .map_err(map_decoding_error)
    }

    fn decode_one_frame(&mut self) -> Result<Frame, Error> {
        let fctl = self.advance_to_next_frame_control()?;
        let geometry = to_geometry(&fctl);

        // `output_buffer_size` is sized to the FULL canvas, not this
        // subframe (see this module's doc comment) — allocate what `png`
        // demands, then use `OutputInfo::buffer_size()` below to find the
        // actually-written subframe prefix.
        let output_len = self.reader.output_buffer_size().ok_or(Error::TooLarge)?;
        let mut raw = vec![0u8; output_len];
        let output_info = self
            .reader
            .next_frame(&mut raw)
            .map_err(map_decoding_error)?;
        let subframe_len = output_info.buffer_size();
        if subframe_len > raw.len() {
            // Unreachable per `png`'s own invariant that `next_frame` only
            // ever writes within the buffer it required; defence in depth
            // against the following slice.
            return Err(Error::Malformed);
        }
        let subframe_rgba = normalize_to_rgba(
            &raw[..subframe_len],
            output_info.color_type,
            output_info.bit_depth,
        )?;

        self.compositor.composite(geometry, &subframe_rgba)?;

        let delay = self
            .limits
            .effective_delay(delay_duration(fctl.delay_num, fctl.delay_den));
        let index = self.next_index;
        self.next_index += 1;

        Ok(Frame {
            width: self.compositor.width(),
            height: self.compositor.height(),
            rgba: self.compositor.canvas_pixels().to_vec(),
            delay,
            index,
        })
    }
}

impl Codec for ApngCodec {
    fn info(&self) -> &Info {
        &self.info
    }

    fn next_frame(&mut self) -> Result<Frame, Error> {
        if let Some(cached) = &self.still_cache {
            return Ok(cached.clone());
        }
        if self.next_index >= self.frame_count {
            self.rewind();
        }
        let frame = self.decode_one_frame()?;
        if self.frame_count == STILL_FRAME_COUNT {
            self.still_cache = Some(frame.clone());
        }
        Ok(frame)
    }

    fn rewind(&mut self) {
        // `png::Reader` has no in-place seek-to-start (unlike `image-webp`'s
        // `reset_animation`, which `webp.rs` also finds insufficient on its
        // own), so a rewind is a full rebuild from the shared bytes — which
        // also naturally clears `still_cache` and the composited canvas.
        match ApngCodec::build(Arc::clone(&self.bytes), &self.limits) {
            Ok(fresh) => *self = fresh,
            Err(_) => {
                // Unreachable in practice: these are the same bytes that
                // already decoded successfully to open this codec. There is
                // no cheaper fallback available; leave state untouched
                // rather than partially mutate it or panic.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_count_zero_plays_is_infinite() {
        assert_eq!(loop_count_from(0), LoopCount::Infinite);
    }

    #[test]
    fn loop_count_nonzero_plays_is_finite_total_plays() {
        assert_eq!(loop_count_from(1), LoopCount::Finite(1));
        assert_eq!(loop_count_from(5), LoopCount::Finite(5));
    }

    #[test]
    fn delay_duration_zero_denominator_means_hundredths() {
        assert_eq!(delay_duration(7, 0), Duration::from_millis(70));
    }

    #[test]
    fn delay_duration_common_fractions_are_exact_or_floor_without_drift() {
        assert_eq!(delay_duration(1, 1), Duration::from_secs(1));
        assert_eq!(delay_duration(50, 100), Duration::from_millis(500));
        assert_eq!(delay_duration(0, 100), Duration::from_millis(0));
        // 1/3 s floors to a whole nanosecond rather than drifting via f64.
        assert_eq!(delay_duration(1, 3), Duration::from_nanos(333_333_333));
    }

    #[test]
    fn normalize_to_rgba_is_identity_for_rgba() {
        let buf = vec![1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(
            normalize_to_rgba(&buf, ColorType::Rgba, BitDepth::Eight).unwrap(),
            buf
        );
    }

    #[test]
    fn normalize_to_rgba_expands_grayscale_alpha_by_replicating_gray_into_rgb() {
        let buf = vec![10, 255, 200, 0];
        let expanded = normalize_to_rgba(&buf, ColorType::GrayscaleAlpha, BitDepth::Eight).unwrap();
        assert_eq!(expanded, vec![10, 10, 10, 255, 200, 200, 200, 0]);
    }

    #[test]
    fn normalize_to_rgba_rejects_a_color_type_this_module_should_never_see() {
        assert_eq!(
            normalize_to_rgba(&[0, 0, 0], ColorType::Rgb, BitDepth::Eight),
            Err(Error::Malformed)
        );
    }

    #[test]
    fn normalize_to_rgba_rejects_non_eight_bit_depth() {
        assert_eq!(
            normalize_to_rgba(&[0, 0, 0, 0], ColorType::Rgba, BitDepth::Sixteen),
            Err(Error::Malformed)
        );
    }
}
