use std::fmt;

/// Everything that can go wrong decoding an animated raster.
///
/// Every decoder call in this crate runs under `std::panic::catch_unwind`
/// (see [`crate::contained_panic_in_progress`]), so a codec panic degrades to
/// [`Error::DecoderPanicked`] exactly like an ordinary decode error rather
/// than taking the caller down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The bytes are not a format this crate decodes (`sniff` returned
    /// `None`), or the format is recognised but not yet implemented (the
    /// GIF and APNG stub codecs).
    Unsupported,
    /// The bytes claim to be a supported format but are truncated, corrupt,
    /// or otherwise fail to decode.
    Malformed,
    /// The image's pixel count (`width * height`) exceeds `Limits::max_pixels`.
    /// Raised before any canvas is allocated.
    TooLarge,
    /// The underlying decoder panicked; the panic was caught and this error
    /// is returned instead. Once this happens the `Animation` that produced
    /// it is poisoned: every later call returns this same error.
    DecoderPanicked,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Error::Unsupported => "unsupported image format",
            Error::Malformed => "malformed image data",
            Error::TooLarge => "image exceeds the configured pixel cap",
            Error::DecoderPanicked => "the decoder panicked while decoding this image",
        };
        f.write_str(message)
    }
}

impl std::error::Error for Error {}
