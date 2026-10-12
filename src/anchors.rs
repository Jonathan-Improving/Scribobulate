//! Heading anchors: the slug every heading answers to, and the one place a
//! `#fragment` is resolved against them (TDD 2.17).
//!
//! Every heading has two slugs:
//!
//! * its **GitHub slug** ([`slugify`]) — the canonical one. It is what GitHub
//!   generates, so a document written for GitHub keeps every link it already has,
//!   including the leading/trailing hyphen an emoji leaves behind
//!   (`## 🚨 Escalations` → `#-escalations`).
//! * its **emoji-free slug** — the same algorithm applied to the heading as if its
//!   emoji had never been typed ([`strip_emoji`]), so `#escalations` also reaches
//!   `## 🚨 Escalations`. That is the form a person writes by hand, and the form
//!   that failed before this second slug existed.
//!
//! Each kind is made unique on its own, by the github-slugger `-1`, `-2` rule
//! ([`unique_slug`]) over every heading in document order. **A GitHub slug always
//! wins**: a fragment is matched against the GitHub slugs first and the emoji-free
//! ones only when none matches, so an emoji-free slug that collides with some
//! heading's GitHub slug is not an alias at all ([`HeadingAnchors::new`]). No link
//! that resolved before the emoji-free slug existed changes its target.
//!
//! The preview's anchor map and the export's heading ids are both built by
//! [`HeadingSlugger`], so the two cannot disagree about what a heading answers to.

use std::collections::{HashMap, HashSet};

/// Convert heading text to a GitHub-style anchor slug: lowercase, then keep only
/// letters, numbers, `_` and `-`, with spaces turned into `-`.  Matches the
/// github-slugger algorithm (lowercase → strip everything else → spaces to
/// hyphens); no hyphen/space collapsing or trimming.  `char::is_alphanumeric` is
/// Unicode-aware so non-ASCII headings slug like GitHub's (precomposed forms).
///
/// **Do not add trimming here, however reasonable the bug report sounds.** A
/// heading that begins or ends with an emoji slugs with a *leading or trailing
/// hyphen* — the symbol is dropped but the space beside it still becomes one — so
/// `## Stem Repair 🩹` is `#stem-repair-`. That is exactly what GitHub does,
/// verified against `github-slugger@2.0.0` in the tests below; trimming would
/// silently break every anchor GitHub actually generates. The hand-written
/// `#stem-repair` is answered by the heading's *emoji-free* slug instead
/// ([`HeadingSlugger`]), which leaves this one intact.
pub(crate) fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == ' ' {
            out.push('-');
        } else if ch == '-' || ch == '_' || ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        }
        // everything else (punctuation, symbols) is dropped
    }
    out
}

/// Disambiguate duplicate slugs the github-slugger way: the first occurrence
/// keeps the bare slug; later collisions get `-1`, `-2`, … (1-based, document
/// order).  Each produced slug is also recorded, so a literal heading that
/// already produced e.g. `notes-1` doesn't collide with a later duplicate.
pub(crate) fn unique_slug(base: &str, seen: &mut HashMap<String, u32>) -> String {
    let mut result = base.to_string();
    while seen.contains_key(&result) {
        let count = seen.entry(base.to_string()).or_insert(0);
        *count += 1;
        result = format!("{base}-{count}");
    }
    seen.insert(result.clone(), 0);
    result
}

/// Inclusive code-point ranges of Unicode's `Extended_Pictographic` property
/// (emoji-data.txt, Unicode 15.1), sorted — every character that can be an emoji
/// base, whether or not it is presented as one by default.
const PICTOGRAPHIC: &[(u32, u32)] = &[
    (0x00A9, 0x00A9),
    (0x00AE, 0x00AE),
    (0x203C, 0x203C),
    (0x2049, 0x2049),
    (0x2122, 0x2122),
    (0x2139, 0x2139),
    (0x2194, 0x2199),
    (0x21A9, 0x21AA),
    (0x231A, 0x231B),
    (0x2328, 0x2328),
    (0x2388, 0x2388),
    (0x23CF, 0x23CF),
    (0x23E9, 0x23F3),
    (0x23F8, 0x23FA),
    (0x24C2, 0x24C2),
    (0x25AA, 0x25AB),
    (0x25B6, 0x25B6),
    (0x25C0, 0x25C0),
    (0x25FB, 0x25FE),
    (0x2600, 0x2605),
    (0x2607, 0x2612),
    (0x2614, 0x2685),
    (0x2690, 0x2705),
    (0x2708, 0x2712),
    (0x2714, 0x2714),
    (0x2716, 0x2716),
    (0x271D, 0x271D),
    (0x2721, 0x2721),
    (0x2728, 0x2728),
    (0x2733, 0x2734),
    (0x2744, 0x2744),
    (0x2747, 0x2747),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2763, 0x2767),
    (0x2795, 0x2797),
    (0x27A1, 0x27A1),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2934, 0x2935),
    (0x2B05, 0x2B07),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x3030, 0x3030),
    (0x303D, 0x303D),
    (0x3297, 0x3297),
    (0x3299, 0x3299),
    (0x1F000, 0x1F0FF),
    (0x1F10D, 0x1F10F),
    (0x1F12F, 0x1F12F),
    (0x1F16C, 0x1F171),
    (0x1F17E, 0x1F17F),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F1AD, 0x1F1E5),
    (0x1F201, 0x1F20F),
    (0x1F21A, 0x1F21A),
    (0x1F22F, 0x1F22F),
    (0x1F232, 0x1F23A),
    (0x1F23C, 0x1F23F),
    (0x1F249, 0x1F3FA),
    (0x1F400, 0x1F53D),
    (0x1F546, 0x1F64F),
    (0x1F680, 0x1F6FF),
    (0x1F774, 0x1F77F),
    (0x1F7D5, 0x1F7FF),
    (0x1F80C, 0x1F80F),
    (0x1F848, 0x1F84F),
    (0x1F85A, 0x1F85F),
    (0x1F888, 0x1F88F),
    (0x1F8AE, 0x1F8FF),
    (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945),
    (0x1F947, 0x1FAFF),
    (0x1FC00, 0x1FFFD),
];

/// Combining keycap — the last code point of `1️⃣`, `#️⃣`, `*️⃣`.
const KEYCAP: char = '\u{20E3}';

fn is_pictographic(ch: char) -> bool {
    let cp = u32::from(ch);
    PICTOGRAPHIC
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// A code point that only ever exists as part of an emoji sequence: zero-width
/// joiner, the variation selectors, the keycap, regional indicators (a flag's two
/// halves), skin-tone modifiers and hair components (both inside the pictographic
/// plane, listed for the reader), and the tag characters of a subdivision flag.
fn is_emoji_component(ch: char) -> bool {
    matches!(ch,
        '\u{200D}'
        | '\u{FE0E}' | '\u{FE0F}'
        | KEYCAP
        | '\u{1F1E6}'..='\u{1F1FF}'
        | '\u{1F3FB}'..='\u{1F3FF}'
        | '\u{1F9B0}'..='\u{1F9B3}'
        | '\u{E0020}'..='\u{E007F}')
}

/// The heading text as if its emoji had never been typed.
///
/// An emoji is any `Extended_Pictographic` character together with the sequence
/// it heads — variation selectors, skin tones, ZWJ-joined partners, flag halves —
/// and a keycap sequence is removed *with* its digit, `#` or `*`, which a plain
/// slug would otherwise keep (`1️⃣ Setup` is not about the number one). The
/// whitespace an emoji sat in goes with it: at either end of the heading it is
/// trimmed, and between two words it closes up to the single space that would
/// have been typed (`Roadmap 🚀 Plans` → `Roadmap Plans`). Whitespace no emoji
/// touched is left exactly as written, so a heading without emoji is returned
/// unchanged and its two slugs coincide.
pub(crate) fn strip_emoji(text: &str) -> String {
    // Pass 1: each char is kept, or replaced by a removal marker (None).
    let chars: Vec<char> = text.chars().collect();
    let mut marked: Vec<Option<char>> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        let keycap_len = keycap_sequence_len(&chars[i..]);
        if keycap_len > 0 {
            marked.extend(std::iter::repeat_n(None, keycap_len));
            i += keycap_len;
            continue;
        }
        marked.push((!(is_pictographic(ch) || is_emoji_component(ch))).then_some(ch));
        i += 1;
    }
    // Pass 2: a maximal run of whitespace and markers that holds at least one
    // marker vanishes at an edge and becomes one space between two words.
    let mut out = String::with_capacity(text.len());
    let mut j = 0;
    while j < marked.len() {
        if !is_gap(marked[j]) {
            if let Some(ch) = marked[j] {
                out.push(ch);
            }
            j += 1;
            continue;
        }
        let start = j;
        while j < marked.len() && is_gap(marked[j]) {
            j += 1;
        }
        let run = &marked[start..j];
        if run.iter().all(Option::is_some) {
            out.extend(run.iter().flatten());
        } else if start > 0 && j < marked.len() && run.iter().any(Option::is_some) {
            out.push(' ');
        }
    }
    out
}

/// Whether a marked char belongs to a gap run: a removed emoji char, or whitespace.
fn is_gap(c: Option<char>) -> bool {
    c.is_none_or(char::is_whitespace)
}

/// Length of a keycap sequence (`[0-9#*] FE0F? 20E3`) at the head of `chars`, or 0.
fn keycap_sequence_len(chars: &[char]) -> usize {
    match chars {
        [base, '\u{FE0F}', KEYCAP, ..] | [base, KEYCAP, ..]
            if base.is_ascii_digit() || matches!(base, '#' | '*') =>
        {
            if chars[1] == KEYCAP {
                2
            } else {
                3
            }
        }
        _ => 0,
    }
}

/// The two slugs one heading answers to, as [`HeadingSlugger::next`] issued them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeadingSlugs {
    /// The GitHub slug, made unique — the heading's canonical anchor.
    pub(crate) slug: String,
    /// The emoji-free slug, made unique, when it differs from `slug` — a candidate
    /// alias, still subject to [`HeadingAnchors::new`]'s "a GitHub slug wins" rule.
    pub(crate) alias: Option<String>,
}

/// Issues each heading's slugs in document order. One per document walk; the
/// renderer and the export walker each own one, which is what keeps their `-1`,
/// `-2` numbering in step.
#[derive(Debug, Clone, Default)]
pub(crate) struct HeadingSlugger {
    github: HashMap<String, u32>,
    emoji_free: HashMap<String, u32>,
}

impl HeadingSlugger {
    /// The slugs for the next heading, whose plain text is `text`.
    pub(crate) fn next(&mut self, text: &str) -> HeadingSlugs {
        let slug = unique_slug(&slugify(text), &mut self.github);
        let alias = unique_slug(&slugify(&strip_emoji(text)), &mut self.emoji_free);
        HeadingSlugs {
            alias: (alias != slug).then_some(alias),
            slug,
        }
    }

    /// Whether `slug` is a GitHub slug some heading of this walk has been issued.
    /// An emoji-free alias equal to one is shadowed and must not be offered.
    pub(crate) fn issued(&self, slug: &str) -> bool {
        self.github.contains_key(slug)
    }
}

/// Where a document's headings sit, by anchor: `slug → position`, plus the
/// emoji-free aliases that reach a heading no GitHub slug already claims.
///
/// Generic over the position so the preview (a buffer offset) and a test can share
/// it. The canonical map is private on purpose: every lookup goes through
/// [`Self::resolve`], so no caller can resolve a fragment by GitHub slug alone and
/// silently miss the emoji-free form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeadingAnchors<P> {
    by_slug: HashMap<String, P>,
    /// alias → the canonical slug of the heading it reaches.
    aliases: HashMap<String, String>,
}

impl<P> Default for HeadingAnchors<P> {
    fn default() -> Self {
        Self {
            by_slug: HashMap::new(),
            aliases: HashMap::new(),
        }
    }
}

impl<P: Copy> HeadingAnchors<P> {
    /// Build from each heading's issued slugs and position, in document order.
    /// An alias equal to any heading's GitHub slug is dropped — that fragment
    /// already names a heading, the one GitHub would choose.
    pub(crate) fn new(headings: impl IntoIterator<Item = (HeadingSlugs, P)>) -> Self {
        let mut by_slug = HashMap::new();
        let mut candidates = Vec::new();
        for (HeadingSlugs { slug, alias }, pos) in headings {
            if let Some(alias) = alias {
                candidates.push((alias, slug.clone()));
            }
            by_slug.insert(slug, pos);
        }
        let aliases = candidates
            .into_iter()
            .filter(|(alias, _)| !by_slug.contains_key(alias))
            .collect();
        Self { by_slug, aliases }
    }

    /// The heading `fragment` names, as `(canonical slug, position)`: by GitHub
    /// slug first, then by emoji-free alias. The canonical slug is what a durable
    /// record (Back/Forward history) should hold, whichever form the link used.
    pub(crate) fn resolve<'a>(&'a self, fragment: &'a str) -> Option<(&'a str, P)> {
        let slug = match self.by_slug.get_key_value(fragment) {
            Some((slug, _)) => slug.as_str(),
            None => self.aliases.get(fragment)?.as_str(),
        };
        self.by_slug.get(slug).map(|&pos| (slug, pos))
    }

    /// Every heading's canonical (GitHub) slug.
    pub(crate) fn slugs(&self) -> HashSet<String> {
        self.by_slug.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{slugify, strip_emoji, unique_slug, HeadingAnchors, HeadingSlugger, HeadingSlugs};
    use std::collections::HashMap;

    #[test]
    fn slugify_matches_github() {
        assert_eq!(slugify("2. Skill Loading"), "2-skill-loading");
        assert_eq!(slugify("Anti-patterns to avoid"), "anti-patterns-to-avoid");
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("C++ Guide"), "c-guide");
        // underscores and existing hyphens survive; no collapsing of repeats.
        assert_eq!(slugify("a_b"), "a_b");
        assert_eq!(slugify("a  b"), "a--b");
    }

    /// An emoji in a heading drops out but the space beside it does not, so the
    /// GitHub slug gains a leading or trailing hyphen. Pinned because it looks like
    /// a defect and the "obvious" fix — trimming — would break every anchor GitHub
    /// really generates; see [`slugify`]'s doc comment.
    ///
    /// Expectations measured against `github-slugger@2.0.0` (the algorithm
    /// GitHub's own pipeline uses), not derived from this implementation, so the
    /// test cannot agree with itself about a wrong answer.
    #[test]
    fn an_emoji_in_a_heading_leaves_a_hyphen_where_it_stood() {
        assert_eq!(
            slugify("Plant Support & Stem Repair 🩹"),
            "plant-support--stem-repair-"
        );
        assert_eq!(slugify("🚀 Getting Started"), "-getting-started");
        // …and a heading whose emoji is not space-separated keeps no hyphen.
        assert_eq!(slugify("Roadmap🚀"), "roadmap");
    }

    #[test]
    fn unique_slug_suffixes_duplicates() {
        let mut seen = HashMap::new();
        assert_eq!(unique_slug("notes", &mut seen), "notes");
        assert_eq!(unique_slug("notes", &mut seen), "notes-1");
        assert_eq!(unique_slug("notes", &mut seen), "notes-2");
        // a different slug is unaffected
        assert_eq!(unique_slug("intro", &mut seen), "intro");
    }

    #[test]
    fn unique_slug_avoids_literal_collision() {
        // A literal heading that already produced "notes-1" must not collide with
        // a later duplicate of "notes".
        let mut seen = HashMap::new();
        assert_eq!(unique_slug("notes-1", &mut seen), "notes-1");
        assert_eq!(unique_slug("notes", &mut seen), "notes");
        assert_eq!(unique_slug("notes", &mut seen), "notes-2"); // skips the taken notes-1
    }

    fn emoji_free_slug(text: &str) -> String {
        slugify(&strip_emoji(text))
    }

    #[test]
    fn an_emoji_at_either_end_leaves_no_separator_behind() {
        assert_eq!(emoji_free_slug("🚨 Escalations"), "escalations");
        assert_eq!(emoji_free_slug("Stem Repair 🩹"), "stem-repair");
        assert_eq!(emoji_free_slug("  🚨  Escalations 🩹 "), "escalations");
    }

    #[test]
    fn an_emoji_between_words_closes_up_to_one_space() {
        assert_eq!(emoji_free_slug("Roadmap 🚀 Plans"), "roadmap-plans");
        assert_eq!(emoji_free_slug("Roadmap 🚀🚀 Plans"), "roadmap-plans");
        assert_eq!(emoji_free_slug("Roadmap🚀Plans"), "roadmapplans");
        // whitespace no emoji touched is left as written, as `slugify` leaves it.
        assert_eq!(emoji_free_slug("a  b 🚀"), "a--b");
    }

    #[test]
    fn every_part_of_an_emoji_sequence_is_removed() {
        // ZWJ sequence (family), with skin-tone modifiers.
        assert_eq!(emoji_free_slug("👨🏽\u{200D}👩🏻\u{200D}👧 Family"), "family");
        // flag: two regional indicators.
        assert_eq!(emoji_free_slug("🇨🇦 Canada"), "canada");
        // subdivision flag: tag characters.
        assert_eq!(
            emoji_free_slug("🏴\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F} Scotland"),
            "scotland"
        );
        // text-default symbol forced to emoji presentation by U+FE0F.
        assert_eq!(emoji_free_slug("⚠\u{FE0F} Warnings"), "warnings");
        assert_eq!(emoji_free_slug("❤️ Thanks"), "thanks");
        // keycap: the digit goes with it, though `slugify` alone would keep it.
        assert_eq!(emoji_free_slug("1️⃣ Setup"), "setup");
        assert_eq!(emoji_free_slug("#\u{20E3} Tags"), "tags");
        // a pictographic letterlike symbol `slugify` alone would keep.
        assert_eq!(emoji_free_slug("ℹ️ Info"), "info");
    }

    #[test]
    fn text_without_emoji_is_returned_unchanged() {
        for text in [
            "Hello, World!",
            "a  b",
            " padded ",
            "2. Skill Loading",
            "café 1",
        ] {
            assert_eq!(strip_emoji(text), text);
        }
        // a digit or `#` that is not a keycap stays.
        assert_eq!(emoji_free_slug("Step 1 #2"), "step-1-2");
    }

    #[test]
    fn an_emoji_only_heading_has_no_alias() {
        let mut s = HeadingSlugger::default();
        assert_eq!(
            s.next("🚀"),
            HeadingSlugs {
                slug: String::new(),
                alias: None
            }
        );
    }

    #[test]
    fn the_slugger_issues_an_alias_only_where_the_slugs_differ() {
        let mut s = HeadingSlugger::default();
        assert_eq!(s.next("Intro").alias, None);
        let esc = s.next("🚨 Escalations");
        assert_eq!(esc.slug, "-escalations");
        assert_eq!(esc.alias.as_deref(), Some("escalations"));
    }

    #[test]
    fn duplicate_emoji_headings_number_both_slugs_alike() {
        let mut s = HeadingSlugger::default();
        let first = s.next("🚨 Escalations");
        let second = s.next("🚨 Escalations");
        assert_eq!(
            (first.slug.as_str(), first.alias.as_deref()),
            ("-escalations", Some("escalations"))
        );
        assert_eq!(
            (second.slug.as_str(), second.alias.as_deref()),
            ("-escalations-1", Some("escalations-1"))
        );
        let anchors = HeadingAnchors::new([(first, 10), (second, 20)]);
        assert_eq!(anchors.resolve("escalations"), Some(("-escalations", 10)));
        assert_eq!(
            anchors.resolve("escalations-1"),
            Some(("-escalations-1", 20))
        );
        assert_eq!(
            anchors.resolve("-escalations-1"),
            Some(("-escalations-1", 20))
        );
    }

    /// Both forms reach an emoji heading: GitHub's, so a document written for
    /// GitHub keeps working, and the hand-written emoji-free one.
    #[test]
    fn both_the_github_and_the_emoji_free_fragment_resolve() {
        let mut s = HeadingSlugger::default();
        let anchors = HeadingAnchors::new([(s.next("Intro"), 1), (s.next("🚨 Escalations"), 2)]);
        assert_eq!(anchors.resolve("-escalations"), Some(("-escalations", 2)));
        assert_eq!(anchors.resolve("escalations"), Some(("-escalations", 2)));
        assert_eq!(anchors.resolve("intro"), Some(("intro", 1)));
        assert_eq!(anchors.resolve("nope"), None);
        assert_eq!(
            anchors.slugs(),
            ["intro", "-escalations"].map(String::from).into()
        );
    }

    /// A GitHub slug always wins: with `## 🚨 Escalations` above `## Escalations`,
    /// `#escalations` keeps reaching the plain heading, exactly as on GitHub, and
    /// the emoji heading is reached by its GitHub form.
    #[test]
    fn a_github_slug_shadows_an_equal_alias() {
        let mut s = HeadingSlugger::default();
        let emoji = s.next("🚨 Escalations");
        let plain = s.next("Escalations");
        assert!(s.issued("escalations"));
        let anchors = HeadingAnchors::new([(emoji, 1), (plain, 2)]);
        assert_eq!(anchors.resolve("escalations"), Some(("escalations", 2)));
        assert_eq!(anchors.resolve("-escalations"), Some(("-escalations", 1)));
    }
}
