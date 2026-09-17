//! GIF codec (`gif` 0.14.x, image-rs; MIT OR Apache-2.0), behind the same
//! [`crate::codec::Codec`] seam `webp.rs` uses. Carried obligations of that
//! crate this module exists to satisfy:
//!
//! - The `gif` crate decodes RGBA8 per-frame rectangles but does **not**
//!   composite them; that is entirely this module's job (see [`dispose`]).
//! - `set_memory_limit` as defence in depth, never the pixel cap itself
//!   (checked centrally by `crate::check_pixel_cap`) — see
//!   [`memory_limit`]'s doc comment for the attack it actually stops.
//! - `rewind` recreates the decoder from the shared bytes and zeroes the
//!   canvas, exactly as `webp.rs` does and for the same reason: a fresh
//!   loop must not show anything left over from the previous one.
//! - Straight alpha throughout: `gif`'s `ColorOutput::RGBA` already writes
//!   alpha 0 for the transparent-index and 255 for every opaque pixel (see
//!   its `converter.rs`), so no extra transparency bookkeeping is needed
//!   beyond honouring that alpha 0 during compositing.
mod dispose;

use std::io::{Cursor, Read};
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use gif::{
    ColorOutput, DecodeOptions, Decoder, DecodingError, DisposalMethod, MemoryLimit, Repeat,
};

use crate::codec::Codec;
use crate::error::Error;
use crate::limits::Limits;
use crate::types::{Frame, Info, LoopCount};
use dispose::ClippedRect;

/// Extra headroom above `max_pixels * 4` bytes for `set_memory_limit`, in
/// the same spirit as `webp.rs`'s `MEMORY_LIMIT_HEADROOM_BYTES`: generous,
/// because this limit is defence in depth, never the cap itself.
const MEMORY_LIMIT_HEADROOM_BYTES: u64 = 16 * 1024 * 1024;
const RGBA_CHANNELS: u64 = 4;
/// A GIF frame delay is declared in units of 10 ms.
const GIF_DELAY_UNIT_MS: u64 = 10;
/// More than one frame is what "animated" means for this format.
const ANIMATED_MIN_FRAME_COUNT: u32 = 1;
/// No NETSCAPE2.0 application extension present: play once. See
/// [`map_loop_count`].
const NO_NETSCAPE_BLOCK_TOTAL_PLAYS: u32 = 1;

type Reader = Cursor<Arc<[u8]>>;

pub(crate) struct GifCodec {
    bytes: Arc<[u8]>,
    decoder: Decoder<Reader>,
    info: Info,
    memory_limit: MemoryLimit,
    limits: Limits,
    /// Persistent composited canvas, `info.width * info.height * 4` bytes,
    /// RGBA8 straight alpha. Starts fully transparent (all zero) and stays
    /// that way across whatever [`GifCodec::rewind`] does.
    canvas: Vec<u8>,
    /// The disposal owed to the canvas from the LAST frame drawn, applied
    /// just before the NEXT frame is drawn (never right after drawing this
    /// one) — so that a caller who reads the same frame's output twice
    /// (which `Animation::next_frame` never does, but nothing here assumes
    /// otherwise) sees a stable canvas.
    pending_disposal: Option<PendingDisposal>,
    next_index: u32,
}

struct PendingDisposal {
    method: DisposalMethod,
    rect: ClippedRect,
    /// Only present for `DisposalMethod::Previous`: the canvas pixels under
    /// `rect` from just before this frame was drawn.
    snapshot: Option<Vec<u8>>,
}

/// Reads only headers and frame metadata through a borrowed cursor: no
/// pixel buffer is allocated and the file is not copied.
pub(crate) fn probe(bytes: &[u8], limits: &Limits) -> Result<Info, Error> {
    build_info(Cursor::new(bytes), memory_limit(limits))
}

pub(crate) fn open(bytes: Arc<[u8]>, limits: &Limits) -> Result<Box<dyn Codec>, Error> {
    let limit = memory_limit(limits);
    // A separate, throwaway metadata pass: `next_frame_info` (see
    // `build_info`) has already consumed this reader to EOF by the time it
    // returns, so it cannot also be the decoder kept for playback below.
    let info = build_info(Cursor::new(bytes.as_ref()), limit.clone())?;
    let decoder = new_decoder(Cursor::new(Arc::clone(&bytes)), limit.clone())?;
    let canvas_len = (info.width as usize) * (info.height as usize) * RGBA_CHANNELS as usize;
    Ok(Box::new(GifCodec {
        bytes,
        decoder,
        info,
        memory_limit: limit,
        limits: *limits,
        canvas: vec![0u8; canvas_len],
        pending_disposal: None,
        next_index: 0,
    }))
}

fn new_decoder<R: Read>(reader: R, memory_limit: MemoryLimit) -> Result<Decoder<R>, Error> {
    let mut options = DecodeOptions::new();
    options.set_color_output(ColorOutput::RGBA);
    options.set_memory_limit(memory_limit);
    // `check_frame_consistency` is left at its default `false`: a frame
    // rect that extends past the logical screen must decode (and then be
    // clipped by `dispose::clip_rect`), not error.
    options.read_info(reader).map_err(map_decoding_error)
}

/// Reads the logical screen descriptor, the loop count, and counts frames —
/// all without decoding a single pixel. The `gif` crate has no header field
/// that gives the frame count up front (unlike image-webp's ANIM chunk), so
/// this walks every frame's metadata via `next_frame_info`, which advances
/// past each frame's LZW-compressed data by consuming (not decompressing)
/// its bytes — confirmed from `gif` 0.14.2's own source
/// (`reader/decoder.rs`'s `DecodeSubBlock` arm short-circuits to
/// `Decoded::Nothing` whenever the output sink is `OutputBuffer::None`,
/// which is exactly what `next_frame_info` passes). This is a full pass
/// over the encoded byte stream, but it decodes no pixel data at all —
/// not even frame 0's — which is a stronger guarantee than the contract
/// asks for ("must not decode pixel data beyond the first frame").
fn build_info<R: Read>(reader: R, memory_limit: MemoryLimit) -> Result<Info, Error> {
    let mut decoder = new_decoder(reader, memory_limit)?;
    let width = u32::from(decoder.width());
    let height = u32::from(decoder.height());
    let loop_count = map_loop_count(decoder.repeat());

    let mut frame_count: u32 = 0;
    while decoder
        .next_frame_info()
        .map_err(map_decoding_error)?
        .is_some()
    {
        frame_count = frame_count.saturating_add(1);
    }

    Ok(Info {
        width,
        height,
        animated: frame_count > ANIMATED_MIN_FRAME_COUNT,
        frame_count: Some(frame_count),
        loop_count,
    })
}

/// Maps the `gif` crate's raw NETSCAPE2.0 loop value to richimg's
/// TOTAL-plays [`LoopCount`]. Isolated to this one function, as the plan
/// requires, because the mapping was provisional pending a researcher check
/// against real browsers — it is now confirmed (sources: WebKit's
/// `GIFImageDecoder::repetitionCount`, Blink's `ImageDecoder`/`BitmapImage`,
/// Gecko's `Decoder.cpp`/`imgFrame.h`):
///
/// - `Repeat::Infinite` (raw NETSCAPE count 0) → [`LoopCount::Infinite`].
/// - `Repeat::Finite(0)` → [`LoopCount::Finite`]`(1)`. The `gif` crate can
///   only produce this value as its own pre-parse default
///   (`Repeat::default()`) — real NETSCAPE parsing never assigns a literal
///   `Finite(0)` itself (a raw count of 0 is mapped to `Infinite` instead,
///   see `gif` 0.14.2's `reader/mod.rs`) — so `Finite(0)` unambiguously
///   means "no NETSCAPE2.0 block was present", which every browser plays
///   exactly once.
/// - `Repeat::Finite(n)` for `n > 0` → [`LoopCount::Finite`]`(n + 1)`: the
///   NETSCAPE count is "replays after the first play", so browsers show
///   `n + 1` total plays.
fn map_loop_count(repeat: Repeat) -> LoopCount {
    match repeat {
        Repeat::Infinite => LoopCount::Infinite,
        Repeat::Finite(0) => LoopCount::Finite(NO_NETSCAPE_BLOCK_TOTAL_PLAYS),
        Repeat::Finite(n) => LoopCount::Finite(u32::from(n) + 1),
    }
}

/// Every `DecodingError` variant (truncated input, a bad block header, an
/// LZW error, a memory-limit trip, ...) is a decode failure from richimg's
/// point of view. A panic from crafted input never reaches this function —
/// it is caught centrally by `crate::guarded`.
fn map_decoding_error(_err: DecodingError) -> Error {
    Error::Malformed
}

/// Bounds the per-FRAME allocation `gif`'s `read_next_frame` makes, as
/// defence in depth. `crate::check_pixel_cap` already bounds the CANVAS
/// (the logical screen) before `open` is ever called, but a GIF's
/// individual image descriptor can declare a width/height independent of
/// (and larger than) the logical screen — `check_frame_consistency` is off,
/// so nothing else rejects that — and `gif`'s `read_next_frame` allocates
/// a buffer sized to that per-frame declaration, not to the canvas. Without
/// this, a small logical screen with one enormous frame descriptor would
/// pass the central cap and still trigger a huge allocation the first time
/// a frame is decoded.
fn memory_limit(limits: &Limits) -> MemoryLimit {
    let pixel_bytes = limits.max_pixels.saturating_mul(RGBA_CHANNELS);
    let with_headroom = pixel_bytes.saturating_add(MEMORY_LIMIT_HEADROOM_BYTES);
    let bytes = NonZeroU64::new(with_headroom).unwrap_or(NonZeroU64::MIN);
    MemoryLimit::Bytes(bytes)
}

impl GifCodec {
    fn apply_pending_disposal(&mut self) {
        let Some(pending) = self.pending_disposal.take() else {
            return;
        };
        match pending.method {
            DisposalMethod::Background => {
                dispose::clear(&mut self.canvas, self.info.width, pending.rect);
            }
            DisposalMethod::Previous => {
                if let Some(snapshot) = &pending.snapshot {
                    dispose::restore(&mut self.canvas, self.info.width, pending.rect, snapshot);
                }
            }
            DisposalMethod::Keep | DisposalMethod::Any => {}
        }
    }

    /// Decodes and composites the next frame. Handles both an animated GIF
    /// and a single-frame (still) one uniformly: when `frame_count == 1`,
    /// `next_index` reaches the total after the first call and every
    /// following call takes the `rewind` branch below, which reproduces
    /// frame 0 (index 0) again — the same "still keeps returning frame 0"
    /// behaviour `webp.rs` gives its still images, arrived at here without
    /// a separate code path because, unlike image-webp, `gif`'s decoding
    /// API does not distinguish a still image from a one-frame animation.
    fn composited_frame(&mut self) -> Result<Frame, Error> {
        let total = self.info.frame_count.unwrap_or(0);
        if total == 0 {
            return Err(Error::Malformed);
        }
        if self.next_index >= total {
            self.rewind();
        }
        self.apply_pending_disposal();

        let (top, left, width, height, dispose_method, delay_units, pixels) = {
            let frame = self
                .decoder
                .read_next_frame()
                .map_err(map_decoding_error)?
                .ok_or(Error::Malformed)?;
            (
                u32::from(frame.top),
                u32::from(frame.left),
                u32::from(frame.width),
                u32::from(frame.height),
                frame.dispose,
                frame.delay,
                frame.buffer.to_vec(),
            )
        };

        let rect = dispose::clip_rect(self.info.width, self.info.height, left, top, width, height);
        let snapshot = (dispose_method == DisposalMethod::Previous)
            .then(|| dispose::snapshot(&self.canvas, self.info.width, rect));
        dispose::composite(&mut self.canvas, self.info.width, rect, &pixels);
        self.pending_disposal = Some(PendingDisposal {
            method: dispose_method,
            rect,
            snapshot,
        });

        let index = self.next_index;
        self.next_index += 1;

        Ok(Frame {
            width: self.info.width,
            height: self.info.height,
            rgba: self.canvas.clone(),
            delay: self.limits.effective_delay(Duration::from_millis(
                u64::from(delay_units) * GIF_DELAY_UNIT_MS,
            )),
            index,
        })
    }
}

impl Codec for GifCodec {
    fn info(&self) -> &Info {
        &self.info
    }

    fn next_frame(&mut self) -> Result<Frame, Error> {
        self.composited_frame()
    }

    fn rewind(&mut self) {
        // Recreates the decoder from the shared bytes rather than trying to
        // seek an existing one back to the start: `gif::Decoder` has no
        // rewind of its own, and even if it did, the canvas below still
        // needs a genuine clear (a fresh loop must not show anything
        // composited by the previous one).
        if let Ok(fresh) = new_decoder(
            Cursor::new(Arc::clone(&self.bytes)),
            self.memory_limit.clone(),
        ) {
            self.decoder = fresh;
        }
        // Unreachable in practice otherwise (the same bytes already decoded
        // successfully to open this codec) — if it somehow failed, the
        // stale decoder is left in place rather than leaving the codec with
        // no decoder at all; the next `read_next_frame` on it degrades to
        // `Error::Malformed` rather than panicking.
        self.canvas.fill(0);
        self.pending_disposal = None;
        self.next_index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_loop_count_no_netscape_block_plays_once() {
        assert_eq!(map_loop_count(Repeat::Finite(0)), LoopCount::Finite(1));
    }

    #[test]
    fn map_loop_count_infinite_is_infinite() {
        assert_eq!(map_loop_count(Repeat::Infinite), LoopCount::Infinite);
    }

    #[test]
    fn map_loop_count_finite_n_is_n_plus_one_total_plays() {
        assert_eq!(map_loop_count(Repeat::Finite(1)), LoopCount::Finite(2));
        assert_eq!(map_loop_count(Repeat::Finite(4)), LoopCount::Finite(5));
    }

    #[test]
    fn memory_limit_is_never_zero_bytes() {
        let limits = Limits {
            max_pixels: 0,
            ..Limits::default()
        };
        match memory_limit(&limits) {
            MemoryLimit::Bytes(bytes) => assert!(bytes.get() > 0),
            MemoryLimit::Unlimited => panic!("expected a bounded memory limit"),
        }
    }
}
