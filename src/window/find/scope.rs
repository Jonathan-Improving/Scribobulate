//! **Search in selection** — the bound, and the decisions that can be made about it
//! without a display.
//!
//! The option is not a property of matching, which is why it is not in
//! [`super::options::FindOptions`]: it does not change what counts as a match, it
//! changes *where* matching is applied. It is also the only find option that is a
//! **held reference into the document** — it names a passage, and the passage moves,
//! shrinks and can stop existing while the reader is still looking at the toggle. The
//! two panes therefore hold it differently, and the difference is the whole subject of
//! this module's type:
//!
//! - **The editor** holds a pair of `GtkTextMark`s. Marks rather than a
//!   [`crate::docref::AnchoredSpan`] because the range must track edits made *inside*
//!   it — which is exactly what Replace All does to it — and that is what marks do
//!   natively and what a captured-text anchor cannot. A selection is also unbounded in
//!   size, so anchoring it by its own text is unbounded in cost.
//! - **The preview** holds a character range plus the render it was taken from. There
//!   is nothing to track: a re-render replaces the buffer wholesale, so the honest
//!   answer is that the bound no longer resolves.
//!
//! And when it does not resolve, **the holder re-derives rather than refuses** (CAM
//! § Document-Reference): the toggle turns itself off and the search covers the whole
//! pane. Refusing — reporting no matches, or matching against a range reinterpreted
//! into the new render — is the confidently wrong answer TDD 11.8 exists to reject, and
//! it is the arm the retired `fold_epoch` got wrong while `PreviewFindCache` got it
//! right.
//!
//! The GTK half lives in `window/find.rs` and `window/findbar.rs`; what is here is the
//! arithmetic, which is where the off-by-ones are.

/// Which render a preview scope was taken from.
///
/// The same pair [`super::super::find`]'s hit cache keys on, and for the same reason:
/// the generation alone is instance-local and restarts at 0 on every fresh
/// `CodePreviewView`, so two independently built widgets collide on it the first time
/// each has rendered once. A preview-mode external reload builds a fresh view, which is
/// exactly that case.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct RenderKey {
    pub view_serial: u64,
    pub generation: u64,
}

/// A captured preview scope: a character range in the preview buffer, and the render it
/// indexes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PreviewScope {
    pub key: RenderKey,
    /// Buffer character offsets, `start < end`.
    pub start: i32,
    pub end: i32,
}

impl PreviewScope {
    /// Whether this scope still describes `key`'s render.
    ///
    /// A bare equality, deliberately: there is no "close enough" here. A range that
    /// indexed the previous render and is applied to this one selects whatever text now
    /// happens to sit at those offsets, which is a confident answer about a passage the
    /// reader never chose.
    pub(crate) fn resolves_against(self, key: RenderKey) -> bool {
        self.key == key
    }

    /// Whether a hit at buffer character offset `at` falls inside this scope.
    ///
    /// Half-open at the end, matching a selection: a hit starting exactly at `end` is
    /// the first thing *after* the selected passage, not the last thing in it.
    pub(crate) fn contains(self, at: i32) -> bool {
        at >= self.start && at < self.end
    }
}

/// Whether a `[start, end)` match lies inside the editor scope `[lo, hi)`.
///
/// The match's END is what decides it, not only its start: a match that begins inside
/// the selection and runs past its end is not a match *in* the selection, and replacing
/// it would edit text the reader did not select. This is the arm a start-only test gets
/// wrong, and it can only be seen with a match longer than the distance from its start
/// to the bound.
pub(crate) fn editor_match_is_inside(match_start: i32, match_end: i32, lo: i32, hi: i32) -> bool {
    match_start >= lo && match_end <= hi
}

/// Which entry of a scoped match list the reader steps to, 1-based over `total`.
///
/// Separated from the stepping code because wrapping is where find bars get it wrong
/// and because it is decidable from three integers. `current` is 0 when there is no
/// current match, which is the state after the query changes; a `total` of 0 has no
/// answer and yields `None`.
pub(crate) fn step(current: i32, total: i32, backward: bool) -> Option<i32> {
    if total <= 0 {
        return None;
    }
    let current = current.clamp(0, total);
    Some(if backward {
        if current <= 1 {
            total
        } else {
            current - 1
        }
    } else {
        current % total + 1
    })
}

/// Where in a scoped match list the reader currently is, 1-based, or 0 for "nowhere".
///
/// `matches` is ascending and `at` is the START offset of the match the reader is
/// looking at. A match that is no longer in the list answers the count of matches
/// before where it was — so "next" is the first one at or after it, which is what the
/// preview's own `resume_ordinal` means by the same words, expressed over a different
/// collection.
pub(crate) fn ordinal_at(matches: &[(i32, i32)], at: i32) -> i32 {
    match matches.iter().position(|(start, _)| *start == at) {
        Some(i) => i as i32 + 1,
        None => matches.iter().filter(|(start, _)| *start < at).count() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::{editor_match_is_inside, ordinal_at, step, PreviewScope, RenderKey};

    fn scope(start: i32, end: i32) -> PreviewScope {
        PreviewScope {
            key: RenderKey {
                view_serial: 1,
                generation: 2,
            },
            start,
            end,
        }
    }

    /// The end bound is EXCLUSIVE, which is what makes the scope describe the same
    /// characters the reader dragged over rather than one more.
    #[test]
    fn a_preview_scope_is_half_open() {
        let s = scope(10, 20);
        assert!(!s.contains(9));
        assert!(s.contains(10));
        assert!(s.contains(19));
        assert!(!s.contains(20), "a hit AT the end is the first one past it");
    }

    /// A scope answers only for the render it was taken from. Neither half of the key
    /// is redundant: the generation restarts per widget, and the serial does not move
    /// across an in-place re-render.
    #[test]
    fn a_preview_scope_resolves_only_against_its_own_render() {
        let s = scope(0, 5);
        assert!(s.resolves_against(RenderKey {
            view_serial: 1,
            generation: 2
        }));
        assert!(
            !s.resolves_against(RenderKey {
                view_serial: 1,
                generation: 3
            }),
            "an in-place re-render bumps the generation and replaces the text"
        );
        assert!(
            !s.resolves_against(RenderKey {
                view_serial: 2,
                generation: 2
            }),
            "a widget swap restarts the generation, so the serial is what separates them"
        );
    }

    /// A match must be inside at BOTH ends. The start-only version passes every case
    /// below except the one that matters, which is why it is written out.
    #[test]
    fn an_editor_match_must_fit_entirely_inside_the_scope() {
        assert!(editor_match_is_inside(10, 14, 10, 20));
        assert!(editor_match_is_inside(16, 20, 10, 20), "flush with the end");
        assert!(!editor_match_is_inside(9, 13, 10, 20), "starts before");
        assert!(
            !editor_match_is_inside(18, 24, 10, 20),
            "starts inside and runs past the end — replacing it would edit text \
             outside the selection"
        );
        assert!(!editor_match_is_inside(30, 34, 10, 20), "wholly after");
    }

    /// Stepping wraps within the scope, in both directions, from every starting point
    /// including "nowhere".
    #[test]
    fn stepping_wraps_within_the_scope() {
        assert_eq!(
            step(0, 3, false),
            Some(1),
            "from nowhere, forward is the first"
        );
        assert_eq!(step(1, 3, false), Some(2));
        assert_eq!(step(3, 3, false), Some(1), "forward past the last wraps");
        assert_eq!(step(0, 3, true), Some(3), "from nowhere, back is the last");
        assert_eq!(step(1, 3, true), Some(3), "back past the first wraps");
        assert_eq!(step(2, 3, true), Some(1));
        assert_eq!(step(1, 1, false), Some(1), "one match steps to itself");
        assert_eq!(step(0, 0, false), None, "no matches, no answer");
        assert_eq!(
            step(5, 3, false),
            Some(1),
            "an out-of-range current is clamped"
        );
    }

    /// The reader resumes where they WERE, not where an ordinal points — the same rule
    /// the preview's hit list already follows, over the editor's list.
    #[test]
    fn the_ordinal_is_found_by_position_not_remembered() {
        let matches = [(10, 14), (40, 44), (90, 94)];
        assert_eq!(ordinal_at(&matches, 40), 2);
        assert_eq!(ordinal_at(&matches, 10), 1);
        // The match that was at 40 is gone: "next" must be the first one after it.
        let after_edit = [(10, 14), (90, 94)];
        assert_eq!(
            ordinal_at(&after_edit, 40),
            1,
            "stepping forward from 1 lands on 90, the first match past where it was"
        );
        // Gone, and it was the last.
        assert_eq!(ordinal_at(&[(10, 14)], 40), 1);
        // Nowhere at all.
        assert_eq!(ordinal_at(&[], 40), 0);
        assert_eq!(ordinal_at(&matches, 5), 0, "before every match");
    }
}
