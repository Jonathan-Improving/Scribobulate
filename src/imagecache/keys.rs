//! How the local-image cache decides two loads are the same load: the file stamp and
//! the key strings built from it.
//!
//! **Split out of [`super::loader`] because it is the part with no display in it.**
//! `loader` is excluded from the coverage ratchet as GTK wiring — it decodes into
//! `GdkTexture`s and is asserted by the memory-gate class rather than by unit tests —
//! but "this file needs a display" was never true of the cache-key arithmetic that was
//! sitting inside it. `file_stamp`, `local_cache_key` and `local_animation_key` touch
//! nothing but `std::fs::metadata` and `format!`, so they were measurable all along and
//! simply were not being measured (QA round 2; two reviewers reached it independently).
//!
//! That is the rule `scripts/coverage.sh`'s own `IGNORE` comment states for every entry
//! in it — what is excluded is the WIRING, and every decision it takes gets extracted
//! into a file that stays in scope. This module is that extraction rather than a
//! widening of the exclusion, which is what the same comment says to do when logic turns
//! up back inside an ignored file.

/// What the cache uses to decide a local file is the same file: its modification
/// time AND its length, from ONE `metadata()` call, so this costs no read.
///
/// **Length is here because mtime alone is not a change signal on any platform.**
/// Windows' `CopyFileExW` — which is `std::fs::copy`, Explorer's copy/paste and
/// robocopy's default — carries the SOURCE's mtime onto the destination, so
/// replacing an image with an older-dated one leaves the stamp unmoved or moves it
/// BACKWARD and the reader keeps seeing the stale decode (MEASURED by the Windows
/// seat: two fixtures shared an mtime to the nanosecond, and a `copy` over one of
/// them did not move it; `std::fs::write` to the same path did). It is not a
/// Windows quirk — `cp -p`, `rsync --times`, `unzip`, `tar -p` and a git checkout
/// all set mtimes that are not "now" on every platform.
///
/// ⚠ **The residual, stated rather than hidden**: a replacement with the same
/// length AND the same mtime is still a hit. Closing that needs a content digest,
/// which means reading up to `MAX_LOCAL_IMAGE_BYTES` on every render of every
/// image — the exact cost this cache exists to avoid (TDD 6.8). Two files that
/// collide on both are the case to accept, not to pay for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileStamp {
    mtime: u128,
    len: u64,
}

impl std::fmt::Display for FileStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let FileStamp { mtime, len } = self;
        write!(f, "{mtime}:{len}")
    }
}

/// Stamp `path` as it is on disk right now. A file that cannot be stat'ed at all
/// stamps as `0:0` — deliberately a VALUE rather than an error, so an unreadable path
/// is a consistent cache key instead of a branch every caller has to handle; the read
/// that follows will fail on its own and report the real reason.
pub(crate) fn file_stamp(path: &std::path::Path) -> FileStamp {
    let meta = std::fs::metadata(path).ok();
    let mtime = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    FileStamp { mtime, len }
}

/// The texture-cache key for a local image: the path, its stamp, and the SIZE it is
/// being drawn at (a vector source re-rendered for zoom is a different entry from the
/// same file at another zoom).
pub(crate) fn local_cache_key(path: &std::path::Path, stamp: FileStamp, size: &str) -> String {
    format!("local:{}:{stamp}:{size}", path.display())
}

/// The [`crate::animation::source`] registry key for a local file's animation
/// bytes — shared between `loader::decode_local_raster` (which registers them on a
/// miss) and `loader::recover_local_animation` (which looks them up, or re-registers a
/// fresh read, on a hit), so the two can never drift onto different keys.
///
/// Deliberately a DIFFERENT namespace from [`local_cache_key`]'s: the same file at the
/// same stamp has one entry of encoded animation bytes but potentially several decoded
/// textures (one per size), so sharing a key would make one evict the other.
pub(crate) fn local_animation_key(path: &std::path::Path, stamp: FileStamp) -> String {
    format!("anim:local:{}:{stamp}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// TDD 6.8: the stamp moves when the CONTENT is replaced even if the mtime does
    /// not — the `CopyFileExW` case in `FileStamp`'s own doc comment. Asserted here on
    /// the value directly rather than through a render, so it holds without a display.
    #[test]
    fn length_alone_distinguishes_two_files_that_share_an_mtime() {
        let dir = tempfile::tempdir().expect("temp dir");
        let a = dir.path().join("a.png");
        let b = dir.path().join("b.png");
        std::fs::write(&a, b"short").expect("write a");
        std::fs::write(&b, b"considerably longer contents").expect("write b");
        // Force the two to share an mtime exactly, which is what a preserving copy
        // does and what makes length load-bearing.
        let stamp_time = std::fs::metadata(&a)
            .and_then(|m| m.modified())
            .expect("mtime of a");
        std::fs::File::options()
            .write(true)
            .open(&b)
            .and_then(|f| f.set_modified(stamp_time))
            .expect("pin b's mtime to a's");

        let sa = file_stamp(&a);
        let sb = file_stamp(&b);
        assert_ne!(
            sa.to_string(),
            sb.to_string(),
            "same mtime, different length must still be different stamps"
        );
    }

    /// An unreadable path stamps as a value, not a panic — see [`file_stamp`].
    #[test]
    fn a_missing_file_stamps_as_zero_rather_than_failing() {
        let stamp = file_stamp(Path::new("/definitely/not/a/real/path.png"));
        assert_eq!(stamp.to_string(), "0:0");
    }

    /// The texture key and the animation-bytes key must never collide for the same
    /// file: one file has one set of encoded bytes but several decoded sizes, so a
    /// shared namespace would let one evict the other.
    #[test]
    fn the_texture_key_and_the_animation_key_occupy_different_namespaces() {
        let path = Path::new("/tmp/x.webp");
        let stamp = file_stamp(path);
        assert_ne!(
            local_cache_key(path, stamp, "64x64"),
            local_animation_key(path, stamp)
        );
    }

    /// Size is part of the texture key: the same file drawn at two sizes is two
    /// entries, which is what lets a vector source cache a re-render per zoom.
    #[test]
    fn the_texture_key_varies_with_the_size_drawn() {
        let path = Path::new("/tmp/x.svg");
        let stamp = file_stamp(path);
        assert_ne!(
            local_cache_key(path, stamp, "64x64"),
            local_cache_key(path, stamp, "128x128")
        );
    }
}
