use std::sync::Arc;

use crate::error::Error;
use crate::format::Format;
use crate::limits::Limits;
use crate::types::{Frame, Info};
use crate::{apng, gif, webp};

/// One decoder implementation per format, so WP4 (GIF) and WP5 (APNG) each own
/// a single file behind this seam. `Send` so an `Animation` (which holds a
/// `Box<dyn Codec>`) can move to a worker thread.
pub(crate) trait Codec: Send {
    fn info(&self) -> &Info;
    fn next_frame(&mut self) -> Result<Frame, Error>;
    fn rewind(&mut self);
}

/// Dispatch to the format's `probe`. Allocates no pixel buffer; the caller
/// (`crate::probe`, `Animation::new`, `crate::first_frame`) is responsible for
/// any pixel-cap decision.
pub(crate) fn probe(format: Format, bytes: &[u8], limits: &Limits) -> Result<Info, Error> {
    match format {
        Format::WebP => webp::probe(bytes, limits),
        Format::Gif => gif::probe(bytes, limits),
        Format::Apng => apng::probe(bytes, limits),
    }
}

/// Dispatch to the format's `open`, which does its first pixel allocation.
/// Callers must already have checked the pixel cap against a prior `probe`.
pub(crate) fn open(
    format: Format,
    bytes: Arc<[u8]>,
    limits: &Limits,
) -> Result<Box<dyn Codec>, Error> {
    match format {
        Format::WebP => webp::open(bytes, limits),
        Format::Gif => gif::open(bytes, limits),
        Format::Apng => apng::open(bytes, limits),
    }
}
