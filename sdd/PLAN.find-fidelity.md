# Plan: Find & Replace fidelity

## Problem

Find & Replace is the least capable surface in the application. It offers one
case-insensitive literal query, Next/Prev, Replace and Replace All — and nothing else.
A reader reviewing agent-written prose cannot search for a word rather than a
substring, cannot distinguish `Note` from `note`, cannot express a pattern, cannot
confine a replacement to the passage they have selected, and cannot get back to a term
they searched for two minutes ago without retyping it.

Four capabilities close the gap:

1. **Match options** — whole word, case sensitivity, regular expression.
2. **Replace next / Replace All** that act on the match the reader is looking at.
3. **Search in selection only** — scoping, above all, for Replace All.
4. **Query and replacement history** — a drop-down on each field, per tab.

### Root cause

None of this is missing by decision; the bar was built minimal in an early phase and
never revisited. Two structural facts make the work larger than "set three properties":

- **There are two search engines, not one.** The editor uses
  `GtkSourceSearchContext`/`SearchSettings`, which already has `case-sensitive`,
  `at-word-boundaries` and `regex-enabled` properties. The preview has its own path
  (`window/find.rs`) built from `TextIter::forward_search` with fixed flags, a
  hand-written `ci_match_ranges` for table-cell `GtkLabel`s, and a third scan over the
  source text of collapsed disclosures. Three matchers, each hard-wired to
  case-insensitive literal, each needing every option.
- **`GtkSourceSearchContext` has no bounded region.** `replace_all` is whole-buffer and
  `occurrences-count` counts the whole buffer. "In selection" cannot be expressed by a
  property; it has to be a range the application enforces on stepping, counting and
  replacing.

## What changes on screen

![Find bar layouts, before and after: today's bar with a search field, previous and next buttons, a match count and a close button over a replace row; the new bar adding a history drop-down to each field and four option toggles (match case, whole word, regular expression, search in selection); the same bar wrapped onto extra rows at the window's minimum width; and the history drop-down open beneath the search field.](find-bar-layouts.svg)

The find bar is the only surface that changes shape. Each option toggle also gains an
Edit-menu item, which is the second surface of the same action rather than a second
piece of state.

### The layouts, in words

**Where the bar sits is unchanged.** It is a `GtkRevealer` in the window's outer box,
between the content area and the footer status bar, so it survives every view-mode
switch and content swap. Inside it is a vertical box of two rows: the find row, always
present, and the replace row beneath it, hidden until Ctrl+H.

**Find row, left to right.** The search field keeps the leftmost position and the focus
it already grabs on open. Immediately to its right — touching it, so the two read as one
control — is the new history button, a narrow `GtkMenuButton` with a downward chevron.
After a gap comes the option group, four toggles in a fixed order: match case, whole
word, regular expression, search in selection. They sit *after* the field and *before*
the navigation buttons because that is where they qualify the query rather than the
traversal: everything to the left of them is what you are looking for, everything to the
right is how you move through what was found. Then the existing previous/next chevrons,
the match-count readout, and the close button at the end — all four keep their present
positions and behaviour.

**Replace row, left to right.** The replacement field, its own history button in the
same touching position as the find row's, then Replace and Replace All — again
unchanged positions. The row as a whole stays insensitive wherever the editor is not
visible, with the explanatory description it already carries.

**Sensitivity.** Only two controls are ever insensitive. The replace row, as today, when
the editor is not visible. And the search-in-selection toggle, when there is no
selection to capture and none already captured — it is the one option whose meaning
depends on the document's current state rather than on the query.

**The readout carries one new state.** Beside "N of M", "No matches" and the "…" it
already shows while the editor's engine is still scanning, it now also reports that the
pattern does not compile. That is a fourth state rather than a zero count, for the
reason TDD 11.8 gives: a confidently wrong "No matches" in place of a missing answer is
the worse failure.

**Narrow windows.** Both rows become wrap boxes, and both fields are width-capped where
they are built. As the window narrows the find row spills in three stages, always in
reading order: the option group and the navigation buttons drop to a second row first,
then the readout and close button to a third. The replace row spills the same way, its
two buttons dropping below the field. Nothing is squeezed and nothing is hidden — a
wrapped child keeps its natural size, which is what makes the bar's contribution to the
window's minimum width equal to its *widest single control* rather than to the sum of
them (TDD 9.38).

**The history drop-down.** Pressing the chevron opens a popover directly beneath the
field it belongs to, listing that tab's recent entries most-recent-first. Choosing one
fills the field and searches for it immediately. The field itself stays a
`GtkSearchEntry` and keeps every key binding it has now, including the Escape that
closes the bar — the popover is a sibling control, not a replacement for the entry.

## Possible approaches

### ✅ 1. One option set, two engines, one matcher core

Hold the four options (case, word, regex, in-selection) as per-tab state. Feed the
editor's three to `SearchSettings` — its native support is exactly right and already
correct for regex backreferences in Replace. Give the preview a single GTK-free
`Matcher` that replaces all three of its hard-wired matchers, and drive the
in-selection bound in application code on both sides.

**Pros**: the editor half is nearly free; the preview's three divergent matchers
collapse to one unit-testable core, which is also what the coverage gate wants (POLICY
step 6's extraction rule). **Cons**: the preview's body search stops being
`forward_search` and becomes a scan of extracted buffer text with an offset map — new
code on a path that is currently correct.

### ❌ 2. Rust `regex` crate for the preview

Rejected. It is a new dependency, and — decisively — it is a *different syntax* from
the editor's. `GtkSourceSearchSettings` regex is GRegex (PCRE), so the same pattern
typed into the same box would mean two things depending on which pane is visible.
`glib::Regex` is already in the tree via glib 0.21 and is that same engine.

### ❌ 3. Search the preview by delegating to a hidden `GtkSourceSearchContext`

Rejected. The preview's matches are not all in a buffer: table-cell matches live in
`GtkLabel` children and collapsed-disclosure matches live in no widget at all. A second
search context could only ever see one of the three sources, which is the confidently
wrong count TDD 11.8 exists to refuse.

## Recommendation

Approach 1, delivered in three batches. Each batch is a self-contained behaviour change
that can be ratified by the Mac and Windows seats on its own.

**Batch A — match options. LANDED.** Three toggles (`Aa`, `Words`, `Reg-Ex`) in the find bar and
three check items in the Edit menu, one stateful `win.` action each, classified as
**uncommon commands**. Per-tab state beside `find_query`, persisted in `TabSession`
(additive, no version bump). The preview's three hard-wired matchers collapsed onto
`window/find/matcher.rs`; the body sweep became a `slice()` extraction plus an offset map
(`window/find/bodytext.rs`) rather than `TextIter::forward_search`. An invalid pattern is
a fourth readout state in both panes. Both bar rows became wrap boxes and both fields are
width-capped.

**Batch B — scope and replace semantics. LANDED.** "Search in selection" is a check box
beside the three toggles and a fourth Edit-menu item, scoping **finding** in both panes. The editor
holds a pair of `GtkTextMark`s (left/right gravity) so the passage tracks the edits
Replace All makes inside it; the preview holds a char range keyed on the same
`view_serial`+`generation` the hit cache uses, and an unresolvable one clears the box
rather than being reinterpreted. Both take a Document-Reference CAM row (15, 16).
Replace acts on the current match and advances; Replace All is bounded by the scope and
reports how many it made.

**The one trap worth carrying forward**: `GtkSourceSearchContext::forward` **wraps**
(`wrap-around` is on by default and this application needs it on — TDD 11.3), so a
`while let Some(..) = sc.forward(&it)` enumeration never terminates. The third tuple
element is the engine saying it has been round, and it is the only reliable stop. Routed
to the `gtk4-rs` skill.

**Batch C — history. LANDED.** Each field gains a `GtkMenuButton` (a bundled
`document-open-recent-symbolic` plus `always-show-arrow`) whose menu is
built **on demand** from the active tab's own list, so there is no model to resync on a
tab switch — only the button's sensitivity, which is a property of that tab's list. One
parameterised `win.pick-find-history` action serves both drop-downs; its target is the
entry verbatim behind a one-character field marker, because an entry is arbitrary text
and no separator is safe in it. Entries are recorded on COMMIT (Enter, Next/Prev,
Replace, Replace All, and choosing a row), never on `search-changed` — the find field
searches as you type, so that signal would record every prefix of every query. Persisted
per tab and repaired on load rather than rejected.

### Ratified scope decisions

- **Per tab, and persisted.** The four options and both histories are `TabSession`
  fields, so they survive a restart with the tab that owns them. `TabSession` is
  `#[serde(default)]`, so this is additive — no schema version bump and no migration
  function, the same property `doc_id` relied on.
- **The live query is still not persisted.** A committed query is already the head of
  the persisted history, and restoring a session must not put the application into a
  search the reader did not just ask for. The find bar restores closed, as it does now.
- **The captured selection bound is not persisted** either — it indexes a buffer that
  does not exist until the document is re-read. The toggle restores off.

### Re-ratifying after a UI rearrangement

Functionality on this branch is ratified on all three platforms. **Moving the bar's
existing widgets around does not re-open that**, and the reason is that the one thing a
rearrangement reliably breaks is already machine-gated on the Linux host:
`window::gtk_integration_tests::no_chrome_sets_the_windows_width_floor_above_the_backstop` asserts the
window's minimum is EXACTLY `MIN_WINDOW_WIDTH` with every toolbar section shown, both
sidebars open, the find bar open with its replace row, and a document whose headings
stretch anything that stretches. A control that raises the floor fails there. That is
worth more than a seat's eye on it, because the macOS failure mode is silent — the window
is **not** grown to meet a risen minimum, so the control is simply not drawn (TDD 9.38).
Accessibility naming is gated the same way: `clippy.toml` bans the bare tooltip setter,
so a control that is not named through `a11y::` fails the build.

So a pure rearrangement needs a green pipeline and nothing else. **Three changes are not
rearrangements and each owes a narrow platform check** — narrow, not a repeat of the full
pass:

| Change | Who has to look | Why Linux cannot answer it |
|---|---|---|
| A new glyph or non-ASCII character in a label | both seats | Different font stacks. The `▾` the history buttons once carried had to be confirmed as not tofu in the bundled macOS font. |
| A new ICON NAME | neither, IF it is bundled | `tests/icon_resolution.rs` answers it per platform, but only for the theme that machine has: this host's Adwaita is 41, which says nothing about the Adwaita 50 the Windows tree stages. Bundling under the requested name settles it before a seat sees a placeholder, and the host theme still wins where it has one. |
| A new control, as opposed to a moved one | this seat first | Construction details do not show up in a layout test. `set_label` builds `box[label, arrow]`, which drew a second chevron beside the toolkit's own — identical on every platform, and caught only by looking. A widget-tree dump plus a screenshot here is the check; it does not need a seat. |
| Anything touching the titlebar | `windows` | Windows requires a NATIVE frame, and adding a `GtkHeaderBar` or `set_titlebar()` silently defeats `GTK_CSD=0` (MANUAL-TEST §7.0a). |

⚠️ **If a floor is ever re-measured by hand rather than by that test, converge first.**
Both seats independently established that a single resize reports the *pre-wrap* floor —
macOS by a coarse drag that stops short, Windows by a `SetWindowPos` refused at the
unwrapped minimum — and that the window reports the short width faithfully, so nothing
looks wrong. Repeat the resize until it stops changing before reading it. The full recipe
for each platform is in MANUAL-TEST §A.2 and §A.3. The automated test avoids this entirely
by MEASURING rather than dragging, which is the third reason to lean on it.

**This section outlives the plan** and must be migrated on retirement rather than deleted
with it — it is a rule about when a change needs platform ratification, so POLICY is its
home, not this file.

## Proposed TDD Rubrics

### 11.13 Match options apply to whichever pane is being searched
- **Given** a document containing `note`, `Note` and `notebook`, in edit mode and again in pure-preview mode
- **When** the reader enables **case sensitive** and searches `Note`
- **Then** only the capitalised occurrences are counted and navigated, in both panes, with the same count
- **And when** the reader enables **whole word** and searches `note`
- **Then** `notebook` is not a match, in both panes
- **And** the options survive switching view mode, and are restored per tab on a tab switch alongside the query (§15.12)

### 11.14 A regular expression means the same thing in both panes
- **Given** the regular-expression option enabled
- **When** the reader searches a pattern using a character class, an anchor and a quantifier
- **Then** the editor and the preview report matches under the **same** engine — GRegex — so a pattern accepted in one pane is accepted in the other, and neither pane silently reinterprets it
- **And** a match is still found inside a table cell and inside a collapsed disclosure, which are searched by their own paths rather than by the buffer

### 11.15 A malformed regular expression says so
- **Given** the regular-expression option enabled
- **When** the reader types a pattern that does not compile
- **Then** the readout says the pattern is invalid rather than "No matches", nothing is highlighted, and Next/Prev do nothing
- **And** completing the pattern into a valid one recovers without closing and reopening the bar

### 11.16 Search in selection confines the search, in either pane
- **Given** edit mode with a selection spanning part of the document, containing some but not all occurrences of a term
- **When** the reader enables **in selection**
- **Then** the count, the highlights and Next/Prev describe only the occurrences inside the selection, and wrapping wraps within it
- **And when** the reader invokes Replace All
- **Then** only the occurrences inside the selection are replaced — not because Replace All is scoped, but because the search it acts on is — and the scope still covers the same passage afterwards even though the replacements changed its length
- **And given** pure-preview mode with a selection made in the rendered preview
- **When** the reader enables **in selection**
- **Then** the same confinement holds there: body matches, table-cell matches and matches inside a collapsed disclosure are each counted only when they fall inside the selected passage
- **And when** the preview is re-rendered beneath the scope — a live-preview re-render, a fold splice, a theme switch or an external reload
- **Then** the scope is not silently reinterpreted against the new render: the toggle turns itself off and the search covers the whole pane, because a reference that cannot be resolved obliges a re-derivation rather than a confident wrong answer
- **And given** no selection, and none already captured
- **When** the reader looks at the in-selection control
- **Then** it is insensitive with an explanatory description

### 11.17 Replace acts on the match the reader is looking at
- **Given** a match selected and highlighted as the current match
- **When** the reader invokes Replace
- **Then** *that* match is replaced and the selection advances to the next one — never the next match after the caret, which re-finds the match already on screen (ScrAP-27)
- **And** with the regular-expression option enabled, a backreference in the replacement expands against the match it replaced
- **And** Replace All reports the number of replacements made

### 11.18 Each field offers that tab's own recent entries
- **Given** the reader has committed several distinct search terms and replacement texts in one tab
- **When** they open the drop-down beside either field
- **Then** the entries appear most-recent-first, without duplicates, capped at the stated limit, and choosing one fills the field and searches for it
- **And** a term typed but never committed — no Enter, no Next, no Replace — does not enter the history
- **And given** a second tab
- **When** its drop-downs are opened
- **Then** they show that tab's own history, never the first tab's
- **And given** the session is saved and the application restarted
- **When** the restored tab's drop-downs are opened
- **Then** its own history is still there, in the same order — while the find bar itself restores closed and no search is in force, because a restore must not put the reader into a search they did not just ask for

### 9.38 (extension) The find bar never sets the window's minimum width
- **Given** the find bar open with the replace row shown and every option control visible
- **When** the window is narrowed to `MIN_WINDOW_WIDTH`
- **Then** the bar's rows wrap and every control stays reachable, and the window's minimum width is unchanged from what it is with the bar closed

## Technical details preserved

- `sourceview5` 0.10 exposes `case-sensitive`, `at-word-boundaries` and `regex-enabled`
  on `SearchSettings`, and `regex-error` on `SearchContext` — the readout's source for
  rubric 11.15 on the editor side.
- `glib::Regex` (glib 0.21) binds GRegex, the same engine `GtkSourceSearchSettings`
  compiles a pattern with. No new dependency is needed for regex in the preview.
- `GtkComboBoxText` and `GtkEntryCompletion` are deprecated from GTK 4.10; the gtk4-rs
  bindings carry `#[deprecated]`, so either one fails POLICY build-pipeline step 2.
- The preview's body search currently relies on `TextSearchFlags::TEXT_ONLY` to step
  over the `U+FFFC` characters that embed tables. A matcher over extracted text has to
  reproduce that by filtering those characters out and mapping result offsets back to
  buffer char offsets.
- `GtkTextBuffer::text()` omits anchored children, so its character offsets do not line
  up with the buffer's (ScrAP-74). The extraction has to use `slice()`, which yields the
  `U+FFFC` placeholder for each anchor and therefore keeps the offsets aligned; the
  matcher then drops those characters and carries a byte → buffer-char-offset map back.
- A preview body match must still be rejected when it lands on text tagged
  `TagName::DisclosurePreview`. That text is the *preview* of a collapsed block's body,
  and the same occurrence is already counted from the source by the collapsed-block
  scan; counting both double-counts one occurrence.
- `glib::Regex::match_()` takes a `&GStr`, so the haystack has to be materialised as a
  `GString` that outlives the `MatchInfo` borrowing it. A NUL byte in the haystack would
  truncate the search, which a `GtkTextBuffer` cannot contain.

### What the probes settled

`window/find/parity.rs` runs both engines over one fixture and compares. It answered four
of the five questions and is now a standing gate (TDD 11.14):

1. **Whole-word wrapping** — GtkSourceView wraps a pattern as `\b(?:…)\b`, matching the
   matcher. `cat|dog` agrees.
2. **The literal word-character predicate** — `is_alphanumeric() || '_'` agrees with
   GtkSourceView's `gtk_text_iter` word family on `note_2`, `note2`, `note-2` and
   non-ASCII letters.
3. **Compile flags** — both anchor `^`/`$` per line, so MULTILINE agrees.
4. **Zero-width matches — THE ONE THAT DIVERGED.** GtkSourceView discards them outright:
   `a*` over `abcabc` is 2 matches to it and was 8 to the matcher; `\b`, `x?` and `^` are
   0 to it and were one per position. The matcher now drops them. Beyond parity it is the
   only navigable answer — an empty match has nothing to highlight and nowhere to scroll
   to, so counting one promises a position the reader can never be taken to.
5. **Backreference syntax in a replacement** — still open; it is Batch B's, and Batch B
   adds its own parity case for it.

### Where the work lands

| Batch | Files |
|---|---|
| A | `src/window/find.rs` (hit-list key gains the options; the three preview matchers collapse onto `Matcher`; body text extraction + offset map), `src/window/find/plan.rs` (`hidden_match_count` folds into the matcher), `src/window/chrome.rs` (the toggles; both rows become wrap boxes; fields width-capped), `src/window/findbar.rs` (the actions and their wiring), `src/window/actions.rs` + the Edit menu model, `src/winstate/tab.rs` (per-tab options), `src/session/schema.rs` + `src/window/tabs/switch.rs` (persist and restore), `scripts/coverage.scope` |
| B | `src/window/find.rs`, `src/window/findbar.rs`, `src/winstate/tab.rs` (the captured bound, editor marks and preview range), `src/window/reload.rs` (drop the bound when the buffer is replaced) |
| C | `src/window/chrome.rs` (the two menu buttons and their popovers), `src/window/findbar.rs` (commit points), `src/winstate/tab.rs`, `src/session/schema.rs` |

Every batch also updates `sdd/TDD.md` (§11 and §15), `tests/MANUAL-TEST.md` §11, and
`sdd/CAM.md` — Batch A an Action CAM row per option under **Uncommon commands**, Batch B
a Document-Reference CAM row for each of the two selection bounds, Batch C a
Document-Reference row only if the history is keyed on anything but its own text.

### Not in scope

Deliberately excluded, so the next session does not treat them as omissions: an
incremental-search-as-you-type toggle, multi-file search, a match list or results pane,
regular-expression syntax help in the UI, and any persistence of the live query or of
the captured selection bound.
