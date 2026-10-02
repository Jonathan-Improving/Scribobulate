//! Rounding a byte offset to a `char` boundary of a string — the one pair of helpers
//! every module that slices text at a computed offset uses, because
//! `str::floor_char_boundary` is unstable.
//!
//! A leaf on purpose: no allocation, no locks and no other module, so the crash
//! forensics (which runs while the process is dying, and may not allocate) can call it.

/// The largest `char` boundary of `s` at or below `i`; `s.len()` for any `i` past the
/// end. Total: never panics.
pub(crate) fn floor_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let bytes = s.as_bytes();
    let mut i = i;
    while i > 0 && bytes[i] & 0xC0 == 0x80 {
        i -= 1;
    }
    i
}

/// The smallest `char` boundary of `s` at or above `i`; `s.len()` for any `i` past the
/// end. Total: never panics.
pub(crate) fn ceil_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let bytes = s.as_bytes();
    let mut i = i;
    while i < bytes.len() && bytes[i] & 0xC0 == 0x80 {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::{ceil_char_boundary, floor_char_boundary};

    /// Inside a multi-byte character each rounds to its own side, at a boundary both
    /// stay put, and past the end both answer the length.
    #[test]
    fn offsets_round_to_the_nearest_boundary_on_their_side() {
        let s = "aé€b"; // a(1) é(2) €(3) b(1)
        assert_eq!(floor_char_boundary(s, 2), 1);
        assert_eq!(ceil_char_boundary(s, 2), 3);
        assert_eq!(floor_char_boundary(s, 4), 3);
        assert_eq!(ceil_char_boundary(s, 4), 6);
        assert_eq!(floor_char_boundary(s, 3), 3);
        assert_eq!(ceil_char_boundary(s, 3), 3);
        assert_eq!(floor_char_boundary(s, 99), s.len());
        assert_eq!(ceil_char_boundary(s, 99), s.len());
        assert_eq!(floor_char_boundary("", 0), 0);
    }
}
