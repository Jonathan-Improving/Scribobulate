//! `richimg` — Scribobulate's GTK-free animated-raster decoder.
//!
//! Bytes in; dimensions and composited RGBA8 frames out. No GTK, no glib, no
//! dependency on the application crate — moving this crate out-of-tree later
//! is a `Cargo.toml` change, not a refactor. See
//! `sdd/PLAN.memory-gates.md` ("Phase 2 — decisions", "The richimg contract").
//!
//! Every decoder call in this crate — `sniff` excepted, which does no
//! decoding — runs under `std::panic::catch_unwind` centrally, here, so the
//! per-format codec modules (`webp`, `gif`, `apng`) don't each do it. A panic
//! becomes [`Error::DecoderPanicked`] and poisons the `Animation` that
//! produced it: every later call on that `Animation` returns the same error.
//! [`contained_panic_in_progress`] is `true` for the duration of such a call,
//! so the application's crash-report panic hook — which fires on every panic,
//! caught or not — can log a contained decoder panic instead of writing a
//! false crash report.

mod apng;
mod codec;
mod error;
mod format;
mod gif;
mod limits;
mod types;
mod webp;

pub use error::Error;
pub use format::{sniff, Format};
pub use limits::{Limits, SHORT_DELAY_THRESHOLD};
pub use types::{Frame, Info, LoopCount};

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe, UnwindSafe};
use std::sync::Arc;

use codec::Codec;

thread_local! {
    /// Reentrant depth counter rather than a bool: `guarded` calls can nest
    /// (e.g. `Animation::new` calling into `codec::open`, which itself may
    /// call back into a guarded helper), and the flag must stay true for the
    /// full outer call.
    static PANIC_GUARD_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// True for the duration of a guarded decoder call on this thread, including
/// while a caught panic is unwinding through it. See the module-level doc
/// comment for why the application relies on this.
pub fn contained_panic_in_progress() -> bool {
    PANIC_GUARD_DEPTH.with(|depth| depth.get() > 0)
}

fn guarded<T>(f: impl FnOnce() -> Result<T, Error> + UnwindSafe) -> Result<T, Error> {
    PANIC_GUARD_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let outcome = panic::catch_unwind(f);
    PANIC_GUARD_DEPTH.with(|depth| depth.set(depth.get() - 1));
    outcome.unwrap_or(Err(Error::DecoderPanicked))
}

/// Learn dimensions and animation metadata without allocating a pixel buffer.
/// Does **not** itself enforce `limits.max_pixels` — that check is centralised
/// in [`Animation::new`] and [`first_frame`], which call this first and then
/// decide whether to allocate a canvas at all.
pub fn probe(bytes: &[u8], limits: &Limits) -> Result<Info, Error> {
    guarded(AssertUnwindSafe(|| {
        let format = sniff(bytes).ok_or(Error::Unsupported)?;
        codec::probe(format, bytes, limits)
    }))
}

fn check_pixel_cap(info: &Info, limits: &Limits) -> Result<(), Error> {
    let pixels = u64::from(info.width)
        .checked_mul(u64::from(info.height))
        .ok_or(Error::TooLarge)?;
    if pixels > limits.max_pixels {
        return Err(Error::TooLarge);
    }
    Ok(())
}

/// A decoded animation: an open decoder plus its playback position.
///
/// `Send` (via the `Codec: Send` bound), so it can be moved to a worker
/// thread — this crate has no GTK in it, which is the whole point.
pub struct Animation {
    codec: Box<dyn Codec>,
    poisoned: bool,
}

impl Animation {
    /// Opens an animation, checking the pixel cap **before** any canvas is
    /// allocated: `probe` first (no pixel allocation), refuse `TooLarge` if
    /// `width * height > limits.max_pixels`, then `open`.
    pub fn new(bytes: Arc<[u8]>, limits: &Limits) -> Result<Animation, Error> {
        guarded(AssertUnwindSafe(|| {
            let format = sniff(&bytes).ok_or(Error::Unsupported)?;
            let info = codec::probe(format, &bytes, limits)?;
            check_pixel_cap(&info, limits)?;
            let codec = codec::open(format, bytes, limits)?;
            Ok(Animation {
                codec,
                poisoned: false,
            })
        }))
    }

    pub fn info(&self) -> &Info {
        self.codec.info()
    }

    /// The next **composited** frame. After the last frame this wraps: it
    /// behaves as `rewind()` followed by frame 0 (index 0 again), so callers
    /// count loops by watching the index wrap rather than by a separate
    /// "loop ended" signal.
    ///
    /// Once a call panics (caught, reported as [`Error::DecoderPanicked`]),
    /// the `Animation` is poisoned and every later call returns the same
    /// error without touching the codec again.
    pub fn next_frame(&mut self) -> Result<Frame, Error> {
        if self.poisoned {
            return Err(Error::DecoderPanicked);
        }
        let outcome = guarded(AssertUnwindSafe(|| self.codec.next_frame()));
        if let Err(Error::DecoderPanicked) = outcome {
            self.poisoned = true;
        }
        outcome
    }

    /// Back to frame 0, with the canvas cleared — not merely the frame
    /// pointer rewound (see `webp::WebpCodec::rewind`'s doc comment for why
    /// that distinction matters for `image-webp` 0.2.4 specifically).
    pub fn rewind(&mut self) {
        if self.poisoned {
            return;
        }
        let outcome = guarded(AssertUnwindSafe(|| {
            self.codec.rewind();
            Ok(())
        }));
        if outcome.is_err() {
            self.poisoned = true;
        }
    }
}

/// Still-use convenience: decode just the first frame.
pub fn first_frame(bytes: &[u8], limits: &Limits) -> Result<Frame, Error> {
    guarded(AssertUnwindSafe(|| {
        let format = sniff(bytes).ok_or(Error::Unsupported)?;
        let info = codec::probe(format, bytes, limits)?;
        check_pixel_cap(&info, limits)?;
        let shared: Arc<[u8]> = Arc::from(bytes);
        let mut codec = codec::open(format, shared, limits)?;
        codec.next_frame()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn animation_is_send() {
        assert_send::<Animation>();
    }

    #[test]
    fn contained_panic_in_progress_is_false_outside_any_guarded_call() {
        assert!(!contained_panic_in_progress());
    }

    #[test]
    fn contained_panic_in_progress_true_during_a_guarded_panic_and_false_after() {
        assert!(!contained_panic_in_progress());
        let result: Result<(), Error> = guarded(AssertUnwindSafe(|| {
            assert!(contained_panic_in_progress());
            panic!("deliberate test panic to exercise the guard");
        }));
        assert_eq!(result, Err(Error::DecoderPanicked));
        assert!(!contained_panic_in_progress());
    }

    #[test]
    fn guarded_returns_the_inner_result_on_no_panic() {
        let ok: Result<u32, Error> = guarded(AssertUnwindSafe(|| Ok(7)));
        assert_eq!(ok, Ok(7));
        let err: Result<u32, Error> = guarded(AssertUnwindSafe(|| Err(Error::Malformed)));
        assert_eq!(err, Err(Error::Malformed));
    }

    #[test]
    fn check_pixel_cap_rejects_over_cap_and_admits_at_cap() {
        let limits = Limits {
            max_pixels: 100,
            ..Limits::default()
        };
        let over = Info {
            width: 11,
            height: 10,
            animated: false,
            frame_count: Some(1),
            loop_count: LoopCount::Finite(1),
        };
        let at_cap = Info {
            width: 10,
            height: 10,
            ..over
        };
        assert_eq!(check_pixel_cap(&over, &limits), Err(Error::TooLarge));
        assert_eq!(check_pixel_cap(&at_cap, &limits), Ok(()));
    }

    #[test]
    fn check_pixel_cap_does_not_overflow_on_huge_dimensions() {
        let limits = Limits::default();
        let huge = Info {
            width: u32::MAX,
            height: u32::MAX,
            animated: false,
            frame_count: Some(1),
            loop_count: LoopCount::Finite(1),
        };
        assert_eq!(check_pixel_cap(&huge, &limits), Err(Error::TooLarge));
    }

    #[test]
    fn gif_shaped_garbage_is_malformed_not_unsupported() {
        // GIF has a real codec, so a file whose magic says GIF is routed to it
        // and fails on its contents. `tests/stub_formats.rs` holds the same
        // check for whichever format is still a stub.
        let limits = Limits::default();
        let gif_bytes: Arc<[u8]> = Arc::from(&b"GIF89a not really a gif"[..]);
        assert_eq!(probe(&gif_bytes, &limits), Err(Error::Malformed));
        assert_eq!(
            Animation::new(Arc::clone(&gif_bytes), &limits).err(),
            Some(Error::Malformed)
        );
    }

    #[test]
    fn unrecognised_bytes_are_unsupported_everywhere() {
        let limits = Limits::default();
        let bytes: Arc<[u8]> = Arc::from(&b"not an image"[..]);
        assert_eq!(probe(&bytes, &limits), Err(Error::Unsupported));
        assert_eq!(
            Animation::new(Arc::clone(&bytes), &limits).err(),
            Some(Error::Unsupported)
        );
        assert_eq!(first_frame(&bytes, &limits).err(), Some(Error::Unsupported));
    }
}
