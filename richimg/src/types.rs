use std::time::Duration;

/// How many times an animation repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopCount {
    Infinite,
    Finite(u32),
}

/// What [`crate::probe`] learns without allocating a pixel buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    pub animated: bool,
    /// `None` when the format cannot report a count without walking the
    /// whole file (not currently produced by any implemented codec, but part
    /// of the contract's shape).
    pub frame_count: Option<u32>,
    pub loop_count: LoopCount,
}

/// One composited, decoded frame.
///
/// `rgba` is RGBA8 with **straight** (non-premultiplied) alpha, stride
/// `width * 4`, always 4 channels — an RGB8 source is expanded with alpha
/// 255. `delay` is the *effective* delay: the declared delay with the
/// short-delay floor already applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub delay: Duration,
    pub index: u32,
}
