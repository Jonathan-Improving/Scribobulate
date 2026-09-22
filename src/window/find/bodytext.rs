//! The preview buffer's text, made searchable by [`super::matcher::Matcher`], and the
//! map back from a match to the buffer offsets that name it.
//!
//! # Why this exists at all
//!
//! The preview's body search used to be `GtkTextIter::forward_search` under
//! `TEXT_ONLY | CASE_INSENSITIVE`, which is a whole matcher the application does not
//! control: its case folding, its notion of a word, and its complete absence of a
//! regular-expression mode are GTK's. Giving the find bar match options means the body
//! path has to be driven by the same [`Matcher`](super::matcher::Matcher) the table
//! cells and the collapsed disclosures are, and that matcher takes a `&str`.
//!
//! # The two traps, both about offsets
//!
//! **`GtkTextBuffer::text()` is the wrong reader.** It omits the characters standing in
//! for anchored children, so its character offsets do not line up with the buffer's and
//! every match past the first table lands on the wrong text (ScrAP-74). `slice()` is the
//! right one: it yields `U+FFFC` for each anchor, so its offsets ARE the buffer's.
//!
//! **But `U+FFFC` must not be matchable.** `TEXT_ONLY` made `forward_search` step over
//! those placeholders, so a query never saw them and never matched one. Leaving them in
//! the haystack would let a regular-expression query match the character a table is
//! rendered as, and would put a literal `.` in reach of every anchor on the page. They
//! are therefore dropped — and dropping them is what puts the haystack back out of step
//! with the buffer, which is what the rest of this module exists to undo.
//!
//! The map is not a per-character table. An anchor is rare — a handful per document
//! against hundreds of thousands of characters — so what is recorded is where the
//! anchors were, and the shift at any point is how many of them lie at or before it.

use super::matcher::Range;

/// The character `GtkTextBuffer::slice()` yields for each `GtkTextChildAnchor` — the
/// tables, images and other widgets the preview embeds. Unicode's OBJECT REPLACEMENT
/// CHARACTER; GTK's own `gtk_text_buffer_get_slice` documents it by that name.
const ANCHOR: char = '\u{FFFC}';

/// A preview buffer's searchable text, plus what is needed to name a match in the
/// buffer's own coordinates.
pub(super) struct BodyText {
    /// The buffer's slice with every [`ANCHOR`] removed. This is what the matcher runs
    /// over, and the only string whose byte offsets this type's inputs may index.
    text: String,
    /// For each anchor that was dropped, the index — in CHARACTERS of [`text`](Self::text)
    /// — that it sat immediately before. Ascending by construction, which is what lets
    /// the shift be a binary search rather than a scan.
    anchors_before: Vec<usize>,
}

impl BodyText {
    /// Extract the searchable text from a buffer `slice`, which must have been taken
    /// with hidden characters INCLUDED so that its characters correspond one-for-one
    /// with the buffer's own offsets.
    pub(super) fn extract(slice: &str) -> Self {
        let mut text = String::with_capacity(slice.len());
        let mut anchors_before = Vec::new();
        let mut chars = 0usize;
        for ch in slice.chars() {
            if ch == ANCHOR {
                anchors_before.push(chars);
                continue;
            }
            text.push(ch);
            chars += 1;
        }
        Self {
            text,
            anchors_before,
        }
    }

    /// The haystack. Every byte range handed back to
    /// [`to_buffer_ranges`](Self::to_buffer_ranges) must index THIS string.
    pub(super) fn text(&self) -> &str {
        &self.text
    }

    /// How many buffer positions lie at or before extracted character `i` that the
    /// extraction removed.
    fn shift(&self, i: usize) -> usize {
        self.anchors_before.partition_point(|&a| a <= i)
    }

    /// Translate ascending, non-overlapping byte ranges in [`text`](Self::text) into
    /// buffer CHARACTER ranges.
    ///
    /// **Ascending is a precondition, not a convenience**: the byte-to-character walk is
    /// a single pass over the text, which is what keeps this linear in the document
    /// rather than quadratic in the number of matches. `Matcher::ranges` returns them in
    /// that order, and it is the only producer.
    ///
    /// A match's END takes the shift at its LAST character, never at its end index. The
    /// difference is an anchor sitting immediately after the match: taking the shift at
    /// the end index would stretch the range over that anchor and highlight a whole
    /// table because the word before it matched. Anchors INSIDE a match are still
    /// covered, which is correct — the match spans them.
    pub(super) fn to_buffer_ranges(&self, ranges: &[Range]) -> Vec<(i32, i32)> {
        let mut out = Vec::with_capacity(ranges.len());
        // The single ascending walk: `walk` is the next character's byte offset,
        // `at` is that character's index, and both only ever move forward.
        let mut bytes = self.text.char_indices().map(|(byte, _)| byte);
        let next = bytes.next();
        let mut walk = Walk { bytes, at: 0, next };
        for &Range { start, end } in ranges {
            let cs = walk.char_index_of(start);
            let ce = walk.char_index_of(end);
            if ce <= cs {
                // A zero-length match has nothing to highlight and nowhere to scroll to.
                // `Matcher` already drops these to agree with the editor's engine; this
                // is the structural guarantee that one can never reach a buffer range.
                continue;
            }
            let bs = cs + self.shift(cs);
            let be = ce + self.shift(ce - 1);
            out.push((bs as i32, be as i32));
        }
        out
    }
}

/// The single forward walk [`BodyText::to_buffer_ranges`] resolves every endpoint
/// through. A type rather than a closure so the three pieces of state that must move
/// together — the character iterator, the index reached, and the byte offset of the
/// character not yet consumed — cannot be advanced independently.
struct Walk<I: Iterator<Item = usize>> {
    bytes: I,
    at: usize,
    next: Option<usize>,
}

impl<I: Iterator<Item = usize>> Walk<I> {
    /// The character index of `byte`. **Only callable with non-decreasing `byte`** —
    /// the walk cannot go back, so an out-of-order call silently answers the index it
    /// had already reached.
    fn char_index_of(&mut self, byte: usize) -> usize {
        while let Some(b) = self.next {
            if b >= byte {
                break;
            }
            self.at += 1;
            self.next = self.bytes.next();
        }
        self.at
    }
}

#[cfg(test)]
mod tests {
    use super::{BodyText, Range};

    fn r(start: usize, end: usize) -> Range {
        Range { start, end }
    }

    /// With no anchors the extraction is the identity and the offsets pass straight
    /// through — the case every other one is a deviation from.
    #[test]
    fn a_slice_with_no_anchors_maps_one_to_one() {
        let b = BodyText::extract("hello world");
        assert_eq!(b.text(), "hello world");
        assert_eq!(b.to_buffer_ranges(&[r(6, 11)]), vec![(6, 11)]);
    }

    /// The whole point: a match AFTER an anchor names the buffer offset the anchor
    /// occupies too. Reading the buffer with `text()` instead of `slice()` is exactly
    /// this failure (ScrAP-74), shifted by one per table.
    #[test]
    fn a_match_after_an_anchor_is_shifted_by_it() {
        let b = BodyText::extract("ab\u{FFFC}cd");
        assert_eq!(b.text(), "abcd");
        // "cd" is extracted chars 2..4; in the buffer it is 3..5.
        assert_eq!(b.to_buffer_ranges(&[r(2, 4)]), vec![(3, 5)]);
        // "ab" is before the anchor and is not shifted.
        assert_eq!(b.to_buffer_ranges(&[r(0, 2)]), vec![(0, 2)]);
    }

    /// Several anchors accumulate, and each match takes the shift in force where it
    /// actually sits rather than the document's total.
    #[test]
    fn each_match_takes_the_shift_in_force_where_it_sits() {
        let b = BodyText::extract("a\u{FFFC}b\u{FFFC}\u{FFFC}c");
        assert_eq!(b.text(), "abc");
        assert_eq!(
            b.to_buffer_ranges(&[r(0, 1), r(1, 2), r(2, 3)]),
            vec![(0, 1), (2, 3), (5, 6)]
        );
    }

    /// An anchor immediately AFTER a match is not swallowed by it. Taking the shift at
    /// the end index rather than at the last character stretches the range over the
    /// anchor, which highlights the whole table that follows the matched word.
    #[test]
    fn an_anchor_just_after_a_match_is_not_included_in_it() {
        let b = BodyText::extract("ab\u{FFFC}cd");
        assert_eq!(b.to_buffer_ranges(&[r(0, 2)]), vec![(0, 2)]);
    }

    /// An anchor INSIDE a match is included, because the match spans it. This is the
    /// arm the previous case must not be "fixed" into: the two differ only in whether
    /// the anchor falls before or after the last matched character.
    #[test]
    fn an_anchor_inside_a_match_is_spanned_by_it() {
        let b = BodyText::extract("ab\u{FFFC}cd");
        // The whole extracted text, which straddles the anchor.
        assert_eq!(b.to_buffer_ranges(&[r(0, 4)]), vec![(0, 5)]);
    }

    /// Byte offsets are not character offsets, and the matcher speaks bytes. A
    /// multibyte haystack is where a byte-for-character substitution stops being
    /// invisible.
    #[test]
    fn multibyte_text_maps_by_character_not_by_byte() {
        let b = BodyText::extract("héllo wörld");
        // "wörld" starts at byte 7 (h é(2) l l o space = 1+2+1+1+1+1) and is 6 bytes.
        assert_eq!(&b.text()[7..13], "wörld");
        // In characters it is 6..11.
        assert_eq!(b.to_buffer_ranges(&[r(7, 13)]), vec![(6, 11)]);
    }

    /// The walk is single-pass and ascending, so a batch of matches must give the same
    /// answers as the same matches resolved one at a time.
    #[test]
    fn a_batch_of_matches_resolves_the_same_as_one_at_a_time() {
        let b = BodyText::extract("aa\u{FFFC}aa\u{FFFC}aa");
        let all = [r(0, 1), r(2, 3), r(4, 5)];
        let batch = b.to_buffer_ranges(&all);
        let singly: Vec<(i32, i32)> = all
            .iter()
            .flat_map(|one| b.to_buffer_ranges(std::slice::from_ref(one)))
            .collect();
        assert_eq!(batch, singly);
    }

    /// A match running to the very end of the text resolves rather than running the
    /// walk off its end.
    #[test]
    fn a_match_at_the_end_of_the_text_resolves() {
        let b = BodyText::extract("x\u{FFFC}end");
        assert_eq!(b.to_buffer_ranges(&[r(1, 4)]), vec![(2, 5)]);
    }

    /// An empty buffer has no text and no matches; nothing indexes anything.
    #[test]
    fn an_empty_slice_yields_nothing() {
        let b = BodyText::extract("");
        assert_eq!(b.text(), "");
        assert!(b.to_buffer_ranges(&[]).is_empty());
    }

    /// A slice that is nothing BUT anchors extracts to an empty haystack — and the
    /// anchor positions recorded for it never index a character that exists.
    #[test]
    fn a_slice_of_only_anchors_extracts_to_nothing() {
        let b = BodyText::extract("\u{FFFC}\u{FFFC}");
        assert_eq!(b.text(), "");
        assert!(b.to_buffer_ranges(&[]).is_empty());
    }
}
