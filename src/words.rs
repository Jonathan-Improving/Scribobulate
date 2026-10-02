//! Where words begin and end in plain text — the one rule a table cell's double-click
//! selection and the status bar's word count share.
//!
//! A word is a run of letters and digits. An apostrophe or hyphen between two of them
//! stays inside the word, so `don't` and `well-known` are one word each, as a reader
//! expects (GTK4Rs/AP-162). Everything else separates words, the underscore included:
//! `snake_case` is two words, as `snake case` is. Pango's own word attributes
//! (`pango_layout_get_log_attrs`) have no Rust binding, which is why this exists.
//!
//! Find's whole-word option is deliberately NOT this rule: it must agree with the
//! editor's search engine, whose `\b` counts `_` as a word character
//! (`window::find::matcher`).

/// Whether the character at `i` belongs to a word.
pub(crate) fn is_word_char_at(chars: &[char], i: usize) -> bool {
    let c = chars[i];
    c.is_alphanumeric()
        || (matches!(c, '\'' | '\u{2019}' | '-')
            && i > 0
            && i + 1 < chars.len()
            && chars[i - 1].is_alphanumeric()
            && chars[i + 1].is_alphanumeric())
}

/// Whether a word starts and/or ends at one position between characters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WordEdge {
    pub(crate) start: bool,
    pub(crate) end: bool,
}

/// Word edges for every position of `text` (its character count plus one).
pub(crate) fn word_edges(text: &str) -> Vec<WordEdge> {
    let chars: Vec<char> = text.chars().collect();
    let is_word: Vec<bool> = (0..chars.len())
        .map(|i| is_word_char_at(&chars, i))
        .collect();
    (0..=chars.len())
        .map(|i| {
            let before = i > 0 && is_word[i - 1];
            let after = i < chars.len() && is_word[i];
            WordEdge {
                start: after && !before,
                end: before && !after,
            }
        })
        .collect()
}

/// How many words `text` holds by [`word_edges`].
pub(crate) fn word_count(text: &str) -> usize {
    word_edges(text).iter().filter(|e| e.start).count()
}

#[cfg(test)]
mod tests {
    use super::word_count;

    /// The joiners stay inside a word only between two letters or digits, and an
    /// underscore separates, as a space does (operator decision, 2026-10-02).
    #[test]
    fn words_are_letter_runs_joined_only_by_inner_apostrophes_and_hyphens() {
        assert_eq!(word_count("don't stop"), 2);
        assert_eq!(word_count("well-known fact"), 2);
        assert_eq!(word_count("snake_case"), 2);
        assert_eq!(word_count("a - b"), 2);
        assert_eq!(word_count("'quoted'"), 1);
        assert_eq!(word_count(""), 0);
        assert_eq!(word_count("über naïve 42"), 3);
    }
}
