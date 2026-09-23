//! The display-free decisions behind the footer status bar (TDD 16.10–16.17, 25.23):
//! what each indicator reads, how the persistent base message is composed, and how a
//! document's readable text is counted.
//!
//! Everything here is a function of plain data so it sits inside the coverage gate;
//! `window::statusbar` owns the widgets and only applies these answers.

use super::{BackingLoss, ViewMode};
use crate::export::{Block, ExportDoc, Inline};

/// Joins the base message's segments ("File deleted on disk — … · Unsaved changes").
const BASE_SEPARATOR: &str = " · ";

/// The persistent message shown while a document's file could not be watched.
pub(crate) const LIVE_RELOAD_OFF: &str = "Live reload is off for this document";

/// The persistent message shown while a document has unsaved edits (TDD 4.4).
pub(crate) const UNSAVED_CHANGES: &str = "Unsaved changes";

/// Confirmation for Copy Document (TDD 16.15).
pub(crate) const DOCUMENT_COPIED: &str = "Document copied";

/// Confirmation for Copy Link Location (TDD 16.15).
pub(crate) const LINK_LOCATION_COPIED: &str = "Link location copied";

/// Confirmation for Replace All (TDD 11.17), which is otherwise the only find command
/// whose outcome the find bar cannot report: after it, the match count is normally zero,
/// and "No matches" is what the reader is left looking at for the one action guaranteed
/// to have changed the most.
pub(crate) fn replacements_made(count: u32) -> String {
    match count {
        1 => "1 replacement made".to_string(),
        n => format!("{n} replacements made"),
    }
}

/// The line separator a document uses, classified from its buffer (TDD 16.13).
///
/// A property of the DOCUMENT, never of the host (POLICY § Cross-platform): an
/// empty or new document reads `Lf` on every platform because the editor inserts
/// `"\n"` for Enter everywhere and nothing converts it on save.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LineEndings {
    Lf,
    Crlf,
    Mixed,
}

impl LineEndings {
    /// Classify `text`. A lone `\r` cannot reach an editor buffer (`lineendings.rs`),
    /// so every `\r` counted here is the first half of a `\r\n`.
    pub(crate) fn classify(text: &str) -> Self {
        let newlines = text.bytes().filter(|&b| b == b'\n').count();
        let crlf = text.matches("\r\n").count();
        match (crlf, newlines) {
            (0, _) => Self::Lf,
            (c, n) if c == n => Self::Crlf,
            _ => Self::Mixed,
        }
    }

    /// The indicator's text.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::Crlf => "CRLF",
            Self::Mixed => "Mixed",
        }
    }

    /// What a screen reader announces for the indicator (TDD 16.17).
    pub(crate) fn accessible_name(self) -> String {
        format!("Line endings, {}", self.label())
    }
}

/// Words and characters of some readable text.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct TextCount {
    pub(crate) words: usize,
    pub(crate) chars: usize,
}

impl TextCount {
    /// Count already-readable text (the preview's rendered text, a table cell's label).
    ///
    /// A word is a run of letters and digits; an apostrophe or hyphen joins two such
    /// runs only when it sits between them ("don't", "well-known" are one word each).
    /// Characters are every character except line breaks, so a count does not depend
    /// on how the text happened to be wrapped into lines.
    pub(crate) fn of_text(text: &str) -> Self {
        let mut words = 0;
        let mut in_word = false;
        let mut chars = text.chars().peekable();
        let mut count = 0;
        let mut prev_alnum = false;
        while let Some(c) = chars.next() {
            if c != '\n' && c != '\r' {
                count += 1;
            }
            if c.is_alphanumeric() {
                if !in_word {
                    words += 1;
                    in_word = true;
                }
                prev_alnum = true;
                continue;
            }
            let joins = is_word_joiner(c)
                && prev_alnum
                && chars.peek().is_some_and(|next| next.is_alphanumeric());
            if !joins {
                in_word = false;
            }
            prev_alnum = false;
        }
        Self {
            words,
            chars: count,
        }
    }

    /// Count the readable text of Markdown `source`: what a reader sees, not the
    /// syntax that produced it (TDD 16.11).
    ///
    /// Built over the export pipeline's `ExportDoc` — the display-free consumer of the
    /// same event stream the preview renders — so "what is document content" has one
    /// answer. Link and image URLs, image alt text and annotation comments are not
    /// counted; the text an annotation claims is. No document folder is passed, so no
    /// image file is ever read to produce a count.
    pub(crate) fn of_markdown(source: &str) -> Self {
        let doc = crate::export::doc::build(
            source,
            &crate::export::RenderOptions {
                doc_dir: None,
                allow_unsafe_images: false,
            },
        );
        Self::of_text(&readable_text(&doc))
    }
}

/// A document's word count and line-ending classification, and the buffer generation
/// they were computed from — so a count that finishes after further typing can be
/// recognised as describing an older buffer (TDD 16.11, 16.13).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TextStats {
    pub(crate) generation: u64,
    pub(crate) count: TextCount,
    pub(crate) endings: LineEndings,
}

/// What a screen reader announces for the line/column indicator (TDD 16.17).
pub(crate) fn position_accessible_name(line: i32, col: u32) -> String {
    format!("Line {line}, column {col}")
}

fn is_word_joiner(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '-')
}

/// The readable text of `doc`, one block per line, with inline formatting boundaries
/// adding nothing — `**bo**ld` stays the one word it reads as.
fn readable_text(doc: &ExportDoc) -> String {
    let mut out = String::new();
    push_blocks(&doc.blocks, &mut out);
    out
}

fn push_blocks(blocks: &[Block], out: &mut String) {
    for block in blocks {
        match block {
            Block::Heading { inlines, .. } | Block::Paragraph(inlines) => {
                push_inlines(inlines, out);
            }
            Block::CodeBlock { text, .. } => out.push_str(text),
            Block::BlockQuote(inner) => push_blocks(inner, out),
            Block::List { items, .. } => {
                for item in items {
                    push_blocks(&item.blocks, out);
                }
            }
            Block::Table { head, rows, .. } => {
                for cell in head.iter().chain(rows.iter().flatten()) {
                    push_inlines(cell, out);
                    out.push('\n');
                }
            }
            Block::Disclosure { summary, body, .. } => {
                push_inlines(summary, out);
                out.push('\n');
                push_blocks(body, out);
            }
            Block::Rule => {}
        }
        out.push('\n');
    }
}

fn push_inlines(inlines: &[Inline], out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text { text, .. } => out.push_str(text),
            Inline::Code(code) => out.push_str(code),
            Inline::Break => out.push('\n'),
            // Alt text describes an image; it is not prose on the page.
            Inline::Image(_) => {}
            Inline::Emphasis(inner)
            | Inline::Strong(inner)
            | Inline::Strikethrough(inner)
            | Inline::Superscript(inner)
            | Inline::Subscript(inner)
            | Inline::Highlight(inner)
            | Inline::Claim { inner, .. }
            | Inline::Link { inner, .. } => push_inlines(inner, out),
        }
    }
}

/// `1234567` → `"1,234,567"`.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(d);
    }
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", grouped(n), if n == 1 { one } else { many })
}

/// The word-count indicator's text, tooltip and accessible name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct WordCountText {
    pub(crate) label: String,
    pub(crate) tooltip: String,
    pub(crate) accessible_name: String,
}

/// Compose the word-count indicator for the whole document and, when text is
/// selected, the selection (TDD 16.11).
pub(crate) fn word_count_text(total: TextCount, selection: Option<TextCount>) -> WordCountText {
    match selection {
        Some(sel) => {
            let label = format!(
                "{} of {}",
                grouped(sel.words),
                plural(total.words, "word", "words")
            );
            WordCountText {
                tooltip: format!(
                    "{} of {}",
                    grouped(sel.chars),
                    plural(total.chars, "character", "characters")
                ),
                accessible_name: format!("Word count, {label} selected"),
                label,
            }
        }
        None => {
            let label = plural(total.words, "word", "words");
            WordCountText {
                tooltip: plural(total.chars, "character", "characters"),
                accessible_name: format!("Word count, {label}"),
                label,
            }
        }
    }
}

/// The zoom indicator's text, or `None` when no preview is visible to be zoomed
/// (TDD 16.12) — the same fact `update_zoom_action_state` gates the zoom actions on.
pub(crate) fn zoom_text(mode: ViewMode, zoom: f64) -> Option<String> {
    mode.is_preview_visible()
        .then(|| format!("{}%", (zoom * 100.0).round() as i64))
}

/// What a screen reader announces for the zoom indicator (TDD 16.17).
pub(crate) fn zoom_accessible_name(percent: &str) -> String {
    format!("Zoom {percent}, reset to 100%")
}

/// The persistent base message for the ACTIVE tab: every condition that holds for as
/// long as it is true, most urgent first (TDD 4.4, 16.16).
///
/// A lost file leads because the buffer is then the document's only copy; live reload
/// being off follows because it explains why the view may be out of date; unsaved
/// changes close the line.
pub(crate) fn base_message(
    loss: Option<BackingLoss>,
    live_reload_off: bool,
    dirty: bool,
) -> String {
    let segments: Vec<&str> = [
        loss.map(BackingLoss::notice),
        live_reload_off.then_some(LIVE_RELOAD_OFF),
        dirty.then_some(UNSAVED_CHANGES),
    ]
    .into_iter()
    .flatten()
    .collect();
    segments.join(BASE_SEPARATOR)
}

/// The export progress message (TDD 25.23). `done` pages have been drawn, so the page
/// being drawn now is the next one, capped at the last. Until pagination has counted the
/// pages (`total == 0`) there is no page to name, so it says only that it is exporting.
pub(crate) fn export_progress_text(done: usize, total: usize) -> String {
    if total == 0 {
        return EXPORTING.to_string();
    }
    format!("Exporting page {} of {total}…", (done + 1).min(total))
}

/// The export message before the document has been paginated.
const EXPORTING: &str = "Exporting…";

/// The progress bar's fraction for `done` of `total` pages, within `0.0..=1.0`.
pub(crate) fn export_progress_fraction(done: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (done as f64 / total as f64).clamp(0.0, 1.0)
}

/// Confirmation for a successful rename (TDD 16.15): the new file name only.
pub(crate) fn renamed_text(new_path: &std::path::Path) -> String {
    let name = new_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| new_path.display().to_string());
    format!("Renamed to {name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_classify_the_document_not_the_host() {
        assert_eq!(LineEndings::classify(""), LineEndings::Lf);
        assert_eq!(LineEndings::classify("no newline"), LineEndings::Lf);
        assert_eq!(LineEndings::classify("a\nb\n"), LineEndings::Lf);
        assert_eq!(LineEndings::classify("a\r\nb\r\n"), LineEndings::Crlf);
        assert_eq!(LineEndings::classify("a\r\nb\n"), LineEndings::Mixed);
        assert_eq!(LineEndings::Crlf.label(), "CRLF");
        assert_eq!(LineEndings::Mixed.accessible_name(), "Line endings, Mixed");
    }

    #[test]
    fn words_join_on_internal_apostrophes_and_hyphens_only() {
        assert_eq!(TextCount::of_text("don't stop").words, 2);
        assert_eq!(TextCount::of_text("well-known COVID-19").words, 2);
        assert_eq!(TextCount::of_text("it\u{2019}s").words, 1);
        assert_eq!(
            TextCount::of_text("a - b").words,
            2,
            "a spaced dash is not a joiner"
        );
        assert_eq!(TextCount::of_text("'quoted'").words, 1);
        assert_eq!(TextCount::of_text("trailing- dash").words, 2);
        assert_eq!(TextCount::of_text("   ").words, 0);
        assert_eq!(TextCount::of_text("naïve café 42").words, 3);
    }

    #[test]
    fn characters_ignore_line_breaks() {
        assert_eq!(TextCount::of_text("ab\ncd").chars, 4);
        assert_eq!(TextCount::of_text("ab\r\ncd").chars, 4);
        assert_eq!(TextCount::of_text("a b").chars, 3);
    }

    /// TDD 16.11 — Markdown syntax, URLs, alt text and annotation comments are not
    /// words; formatting boundaries do not split a word.
    #[test]
    fn markdown_counts_only_readable_text() {
        let md = "# Title here\n\nSome **bo**ld and [a link](https://example.com/long/url) \
                  ![alt words here](img.png) `code`.\n";
        // Title, here, Some, bold, and, a, link, code
        assert_eq!(TextCount::of_markdown(md).words, 8);

        let annotated = "Keep {==this claim==}{>>a comment with many words<<} only.\n";
        // Keep, this, claim, only
        assert_eq!(TextCount::of_markdown(annotated).words, 4);
    }

    #[test]
    fn markdown_counts_tables_lists_quotes_and_code_blocks() {
        let md = "| h1 | h2 |\n|---|---|\n| one | two |\n\n- item\n  - nested\n\n> quoted\n\n```\nfn main\n```\n";
        // h1, h2, one, two, item, nested, quoted, fn, main
        assert_eq!(TextCount::of_markdown(md).words, 9);
    }

    #[test]
    fn table_cells_do_not_run_together() {
        let md = "| ab | cd |\n|---|---|\n";
        assert_eq!(TextCount::of_markdown(md).words, 2);
    }

    #[test]
    fn word_count_text_whole_document_and_selection() {
        let total = TextCount {
            words: 1234,
            chars: 5678,
        };
        let whole = word_count_text(total, None);
        assert_eq!(whole.label, "1,234 words");
        assert_eq!(whole.tooltip, "5,678 characters");
        assert_eq!(whole.accessible_name, "Word count, 1,234 words");

        let sel = word_count_text(
            total,
            Some(TextCount {
                words: 12,
                chars: 60,
            }),
        );
        assert_eq!(sel.label, "12 of 1,234 words");
        assert_eq!(sel.tooltip, "60 of 5,678 characters");

        let one = word_count_text(TextCount { words: 1, chars: 1 }, None);
        assert_eq!(one.label, "1 word");
        assert_eq!(one.tooltip, "1 character");
    }

    #[test]
    fn grouping_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(1234567), "1,234,567");
    }

    #[test]
    fn zoom_text_hides_without_a_preview() {
        assert_eq!(zoom_text(ViewMode::Edit, 1.1), None);
        assert_eq!(zoom_text(ViewMode::Preview, 1.1), Some("110%".into()));
        assert_eq!(zoom_text(ViewMode::Split, 0.75), Some("75%".into()));
        assert_eq!(zoom_accessible_name("110%"), "Zoom 110%, reset to 100%");
    }

    /// TDD 16.16 — every condition that holds shows, in urgency order.
    #[test]
    fn base_message_composes_every_standing_condition() {
        assert_eq!(base_message(None, false, false), "");
        assert_eq!(base_message(None, false, true), UNSAVED_CHANGES);
        assert_eq!(
            base_message(Some(BackingLoss::Deleted), true, true),
            format!(
                "{} · {LIVE_RELOAD_OFF} · {UNSAVED_CHANGES}",
                BackingLoss::Deleted.notice()
            )
        );
        assert_eq!(
            base_message(Some(BackingLoss::Truncated), false, false),
            BackingLoss::Truncated.notice()
        );
    }

    #[test]
    fn export_progress_names_the_page_being_drawn() {
        assert_eq!(export_progress_text(0, 12), "Exporting page 1 of 12…");
        assert_eq!(export_progress_text(2, 12), "Exporting page 3 of 12…");
        assert_eq!(export_progress_text(12, 12), "Exporting page 12 of 12…");
        assert_eq!(
            export_progress_text(0, 0),
            "Exporting…",
            "before pagination there is no page count to claim"
        );
        assert_eq!(export_progress_fraction(3, 12), 0.25);
        assert_eq!(export_progress_fraction(5, 0), 0.0);
        assert_eq!(export_progress_fraction(20, 12), 1.0);
    }

    #[test]
    fn rename_confirmation_names_the_file_only() {
        assert_eq!(
            renamed_text(std::path::Path::new("/some/dir/new name.md")),
            "Renamed to new name.md"
        );
    }
}
