# Plan: Usable Window on Narrow (sub-720px) Displays

## Problem

The window cannot be made to fit displays narrower than roughly 720–900
logical px, and on such a display part of the chrome — up to the whole
right-hand portion of the toolbar, tab bar, and content — renders off-screen
with no way to reach it (confirmed 2026-09-16 against a live RDP session
whose virtual display was 540×1140 logical px; screenshot showed the
toolbar/tab bar/outline sidebar all clipped past the right edge, un-scrollable
and un-reachable by dragging, since the window cannot shrink below its
content-derived minimum).

This matters because "small screen" here isn't just older 1280×720 laptops —
narrow RDP/remote-desktop sessions (portrait client devices, constrained
virtual displays) are a real, encountered case, and today the app is simply
unusable in them: no combination of user action (dragging, hiding toolbar
sections via View ▸ Toolbar, resizing) gets the whole window on-screen,
because there is no floor low enough to reach and no mechanism to shrink the
toolbar's own content further than "hide a whole section."

### Root cause

Two compounding, both intentional:

1. **`MIN_WINDOW_WIDTH = 720`** ([`src/window/mod.rs:348`](../src/window/mod.rs)) is an
   explicit `set_size_request` floor, chosen assuming the *screen* is at least
   as wide as "mainstream small" (1280×720 / 1366×768) — i.e. it was sized to
   be comfortably smaller than the assumed screen, not to itself be a
   screen-fitting floor. A 540px-wide screen is below this floor outright.

2. Even if (1) were lowered or removed, GTK computes toplevel minimum width as
   `MAX(content-derived minimum, size_request)`
   ([`update_toolbar_min_width` doc comment](../src/window/viewactions.rs)), and the
   **toolbar is a single non-wrapping horizontal `GtkBox`** of six
   separator-delimited sections (`file, edit, format, view, split, zoom` —
   [`crate::app::TBTN_SECTION_IDS`](../src/app/commands.rs)), each holding several
   flat icon buttons with no reflow behavior. With all six sections visible
   this content minimum is documented at ~1633px; with the default three
   (`file, edit, view` — [`ToolbarSections::default()`](../src/session.rs)) it is
   smaller but still, per the observed clipping, above 540px once the ~240px
   outline sidebar (visible by default) and tab bar are added. The only
   existing lever to shrink it is `View ▸ Toolbar` hiding a section wholesale —
   a manual, per-session, easy-to-forget action, not an automatic fit.

A narrower, related precedent already exists and partially compensates for
symptom (2): [`window/chrome_fit.rs`](../src/window/chrome_fit.rs)'s
`overflow_inset` pulls the bottom-right status-bar indicator and toast action
buttons back onto the visible monitor when the window is forced wider than
it (TDD §I5 note, TDD.md:1285). It is a targeted fix for two specific
right-anchored widgets, not a general answer — the toolbar and tab bar
themselves are not covered and stay off-edge.

## Previously attempted

Nothing has been attempted toward a general fix yet. The status-bar/toast
inset (`chrome_fit.rs`) was built for a narrower problem (specific
right-anchored controls going unreachable), not this one, and is called out
above only because any general solution should either subsume it or
co-exist with it without duplicating the monitor-geometry read.

## Possible approaches

### 1. Toolbar overflow ("more") menu

When the toolbar's natural content width would exceed the available window
width, collapse the lowest-priority visible sections (or their least-used
buttons) into a `GtkMenuButton` overflow popover, the way GNOME's
`AdwHeaderBar`/`HdyHeaderBar` squeeze mode or a browser's overflow chevron
does. The always-narrow floor becomes small (one row of "always visible"
essentials + one overflow button), independent of how many sections the user
has toggled on.

**Pros**: general fix, works at any width, no change to the six-section
model's semantics (a hidden-by-squeeze button is still reachable, just via
the overflow button rather than the bar).
**Cons**: the largest lift. Needs a width-driven relayout pass (GTK doesn't
do this automatically for a plain `GtkBox`; likely needs `GtkFlowBox`,
manual `size-allocate` handling, or a rewrite onto `AdwToolbarView`/similar
libadwaita widget the project doesn't currently depend on). Touches CAM.md's
Action CAMs (every toolbar button is a command surface) since each button
would now have two possible presentations (bar vs. overflow popover) that
must both reflect the same GAction state — a new obligation. Also touches
invariants I1–I7 in `viewactions.rs`, which currently assume a section is
either fully shown or fully hidden, never partially squeezed.

### 2. Two-row (wrapping) toolbar

Let the toolbar wrap to a second row instead of clipping horizontally,
analogous to how `GtkFlowBox` or a manual wrap container would lay out
buttons top-to-bottom, left-to-right when the row is too narrow.

**Pros**: simpler mental model than an overflow menu; every button stays
visible and reachable without an extra click.
**Cons**: vertical space is at a premium in an editor/preview app (every row
the toolbar grows is a row taken from the document), and a wrapping toolbar
whose row count changes with window width makes the window's *height*
layout unstable during a horizontal-only drag — likely surprising. Doesn't
address the tab bar or outline sidebar, which have the same non-wrapping
problem at small widths.

### 3. Auto-hide sections below a width threshold ("active-shrink" for the toolbar)

Extend `update_toolbar_min_width`'s reconciliation so that, instead of only
recomputing the *floor* from currently-visible sections, it can also
auto-hide the lowest-priority visible sections when the window narrows below
their combined content minimum, and auto-restore them when it widens back
past that point.

**Pros**: reuses the existing show/hide section mechanism (`win.show-tbtn-<id>`
actions) rather than inventing a new overflow surface; smallest new
"presentation state" to reason about, since a section is still simply
shown or hidden.
**Cons**: directly contradicts the documented **active-shrink is deliberately
NOT done** operator decision in `update_toolbar_min_width`'s doc comment
(a frame changing what's visible without the user asking is called out there
as jarring UX) — reversing that call needs the user's/maintainer's sign-off,
not just a workaround. Also: auto-hiding writes to the same `S_i` toggle
state that `View ▸ Toolbar` is the user-facing control for (I4: "disable" is
not "uncheck" — the existing design deliberately keeps those separate),
so this would need a third state (auto-hidden vs. user-hidden) to avoid the
window remembering "no toolbar sections" as the user's actual preference
after a temporary narrow session.

### 4. Accept the floor, but lower it and stop the clipping being *silent*

Don't try to fit an arbitrarily narrow screen. Instead: (a) lower
`MIN_WINDOW_WIDTH` to something closer to what the *default* three-section
toolbar actually needs (today it's set below even that — see root cause 1 —
so this alone is a bug independent of the 540px case), and (b) extend
`chrome_fit.rs`'s existing "window wider than monitor" detection to also
pull the *toolbar itself* onto the visible area (e.g. auto-hide non-essential
sections once, on window creation / monitor-attach, if the initial content
minimum already exceeds the monitor — a one-time corrective action, not a
live drag-time behavior) with a persistent, dismissible notice explaining
why.

**Pros**: smallest change, respects the existing active-shrink prohibition
(it's a one-time "this window doesn't fit, minimizing to essentials"
correction rather than continuous reflow), builds on the monitor-geometry
primitives `chrome_fit.rs` already has.
**Cons**: still degrades on an *interactive* narrowing drag (only fixes the
open-on-a-small-monitor case, not "I resized my RDP client mid-session"),
and doesn't fully solve arbitrarily narrow widths (540px may still be
narrower than three sections + sidebar + tab bar even minimized) — likely
needs pairing with hiding the outline sidebar too.

## Recommendation

Start with **approach 4** as a fast, low-risk mitigation (it directly fixes
the "opens already too wide for this monitor" case observed on 2026-09-16,
and corrects the `MIN_WINDOW_WIDTH` bug where the floor is set below what the
default toolbar itself needs), then evaluate whether **approach 1** (overflow
menu) is worth the larger investment based on how often narrow-session use
actually recurs. Approaches 2 and 3 are weaker fits: 2 trades away vertical
space this app treats as precious, and 3 requires reversing a documented,
deliberate UX decision that should get explicit maintainer sign-off before
any code changes, not be reversed as a side effect of a narrow-screen fix.

Any implementation must, per `AGENTS.md`'s task triggers:
- Update `sdd/system-overview.svg` if the toolbar/chrome architecture changes
  (POLICY build pipeline step 8 validates this).
- Walk the CAM.md Action CAM checklist for every toolbar button whose
  presentation becomes conditional on width (approach 1 only).
- Add/update TDD.md rubrics for the new fit behavior (extending §I5's
  existing "forced wider than monitor" rubric at TDD.md:1285), and update
  `tests/MANUAL-TEST.md` if the fix needs a real narrow-monitor manual check
  (`cargo test` can't drive real monitor geometry).
- Route the "active-shrink is deliberately NOT done" reversal (if approach 3
  is ever chosen instead) through an explicit decision, not a silent code
  change — it currently has an operator's name behind it in the code comment.

## Technical details preserved

- Confirmed reproduction: RDP session, virtual display 540×1140 logical px
  (1080×2280 physical, 2x client-side scale), `AppliedDPI` 96 (no OS-side
  scaling) — i.e. the narrowness is real logical pixels, not a DPI
  misreading. `Get-CimInstance Win32_VideoController` reports the physical
  1080×2280; `[System.Windows.Forms.Screen]::AllScreens` reports the logical
  540×1140 bounds actually available to window placement.
- `MIN_WINDOW_WIDTH = 720` at [`src/window/mod.rs:348`](../src/window/mod.rs),
  applied via `window.set_size_request(MIN_WINDOW_WIDTH, -1)` at
  `src/window/mod.rs:413`.
- Toolbar sections: `crate::app::TBTN_SECTION_IDS = ["file", "edit", "format",
  "view", "split", "zoom"]` ([`src/app/commands.rs:237`](../src/app/commands.rs));
  default-visible set is `file, edit, view`
  ([`ToolbarSections::default()`](../src/session.rs)); built as six
  separator-delimited `GtkBox` sections in `build_toolbar`
  ([`src/window/toolbar.rs`](../src/window/toolbar.rs)); visibility reconciled by
  `reconcile_toolbar_chrome` / `update_toolbar_min_width`
  ([`src/window/viewactions.rs:595-633`](../src/window/viewactions.rs)), which
  documents invariants I3–I7 and the deliberate "no active-shrink" decision.
- Existing overflow-inset precedent: `window::chrome_fit::overflow_inset(win_width,
  monitor_width)` ([`src/window/chrome_fit.rs:40`](../src/window/chrome_fit.rs)),
  consumed by `statusbar.rs` and `toast.rs` to keep two specific
  right-anchored widgets on-monitor; contract at TDD.md:1285. Any general fix
  should read monitor geometry the same way (`window_monitor_widths`,
  `src/window/chrome_fit.rs:72`) rather than re-deriving it.
- Outline sidebar defaults visible (`outline_visible: true` in
  `ChromeSession::default()`, `src/session.rs:449`) and contributes further
  width (~240px per `MIN_WINDOW_WIDTH`'s own doc comment) on top of the
  toolbar's minimum — any fix targeting the toolbar alone will not by itself
  get a fresh default window under ~540px; the sidebar needs the same
  treatment.

### 2026-09-16 implementation: monitor-aware floor, shipped; corrective notice, not yet reliable

Approach 4 was implemented, in the smaller, safer form the persistence risk below
argues for — not the fuller "auto-hide sections" version originally sketched:

- **Shipped**: `window::chrome_fit::effective_min_window_width` (and
  `primary_monitor_width`) make the `set_size_request` floor and the initial
  `default_width` monitor-aware — `MIN_WINDOW_WIDTH` (720) no longer gets
  requested on a monitor narrower than that, which was root cause 1 above and
  is a real, independent bug fix regardless of the toolbar/sidebar content
  minimum (unit-tested, `src/window/chrome_fit.rs`). Confirmed against the live
  540×1140 RDP session: `primary_monitor_width()` correctly reads `540`.
- **Deliberately NOT done**: auto-hiding toolbar sections or the outline on
  detecting overflow. `register_view_actions` seeds each `win.show-tbtn-<id>`/
  `win.outline` GAction's *state* directly from `ChromeSession`, and
  `read_window_chrome`'s doc explicitly treats that live action state as the
  sole source of truth for what gets persisted at close — "no cache to fall
  out of step with it." Silently flipping those actions to correct a narrow
  monitor would therefore be written back to disk as if the reader had chosen
  it, so a temporary narrow-RDP session would poison every future window's
  toolbar/outline on any monitor, wide or narrow, until manually re-enabled.
  Undoing that needs a genuine third state (auto-corrected vs. user-chosen)
  that does not exist today — exactly the complexity approach 3's "Cons"
  section above already flagged, just encountered from the other direction.
  Fixing it properly is future work; a plain state-seeding path is not the
  place to bolt it on speculatively.
- **Shipped, but with a known gap**: a one-shot informational notice
  (`window::toast::show_narrow_window_toast`, fired from `build_window`'s
  `warn_if_wider_than_monitor`) that only *tells* the reader to use the
  existing manual `View ▸ Toolbar`/outline-× controls, touching no persisted
  state. Host-measured against the same 540×1140 session: the code path runs
  (confirmed by log — `window_monitor_widths` returns the correct
  `(1321, 540)` and `InfoToast::show` is called), but nothing appears
  on screen. `chrome_fit::apply_visible_area_inset`'s margin-based pull-in was
  built and unit-tested against modest overflows (the module's own tests use
  ~230px); at the 781px inset this case needs, the margin exceeds
  `content_overlay`'s own width and the toast renders somewhere off the
  visible pane. The matching status-bar announcement
  (`WindowChrome::push_timed_notice`) was expected to be a left-aligned
  fallback immune to this, but was equally not visible in the same capture —
  not yet root-caused; the status bar's own layout needs the same scrutiny
  `chrome_fit.rs` gives the toast before trusting it as a fallback.
- **Left as follow-up work, not done here**: making the corrective notice
  actually reach the reader on a screen this narrow (either fix
  `apply_visible_area_inset` to clamp/degrade gracefully past its tested
  range, or find a placement that isn't overlay-margin-based, e.g. the
  titlebar or a modal-free first-run banner), and the harder auto-hide
  question above, which needs a maintainer decision on the third-state design
  before any code.

### 2026-09-16 follow-up: the notice was reverted, not just left broken

The notice above (`show_narrow_window_toast` / `warn_if_wider_than_monitor`)
was removed after landing, rather than kept in its known-broken state:

- It pushes onto the same status-bar notice stack
  (`WindowChrome::push_timed_notice`) that several existing GTK integration
  tests assert against, and it fired unconditionally from `build_window` —
  i.e. from every window any test builds. It clobbered
  `window::reload::gtk_integration_tests::an_emptied_file_keeps_its_buffer_until_its_own_content_returns`'s
  expected "File was truncated — save to restore it" message with its own
  "Window is wider than this screen — …" text, because the test harness's
  window is itself narrower than its (real or virtual) monitor.
- Given the notice was already confirmed not to render visibly at the
  overflow ratio it exists for (host-measured, see above), and it also turned
  out to actively corrupt an unrelated test's status-bar assertion, keeping it
  had no upside — only cost (a real regression) and a documented gap (no
  benefit). It was removed in full: `warn_if_wider_than_monitor` in
  `src/window/mod.rs` and `show_narrow_window_toast` in `src/window/toast.rs`.
- **What remains shipped**: only `effective_min_window_width` /
  `primary_monitor_width` (the monitor-aware floor and initial size) — the
  part that is independently correct, unit-tested, and does not touch the
  status-bar/toast notice stack at all.
- **Future work, if this is picked back up**: any reader-facing notice for
  this condition needs to (a) actually render at extreme overflow ratios
  (the `apply_visible_area_inset` gap above), and (b) not share a stack with
  content that tests (and, in real use, other transient notices) depend on
  being retractable/predictable — e.g. by not firing at all when
  `window_monitor_widths` can't distinguish "genuinely narrow monitor" from
  "test harness built a small window," or by keeping the visual toast but
  dropping the status-bar push, or by giving this class of notice its own
  slot instead of sharing `push_timed_notice`'s stack.

### 2026-09-16 second follow-up: approach 2 implemented for the toolbar (hard lock fixed)

The narrower, still-reachable symptom that remained after the above — dragging
the window narrower than the toolbar's non-wrapping content width was a genuine
hard lock, not just an off-screen inconvenience, since `MAX(content_derived_minimum,
size_request)` meant GTK would not honor a narrower size at all — is fixed by
implementing **approach 2** (two-row/wrapping toolbar) for the toolbar
specifically, at **section granularity** rather than per-button:

- `build_toolbar` (`src/window/toolbar.rs`) now holds the six section boxes in a
  `GtkFlowBox` (`selection_mode: None`, `homogeneous: false`,
  `min_children_per_line: 1`) instead of a plain `GtkBox`. A `GtkFlowBox` wraps
  whichever sections don't fit the current width onto additional rows instead of
  clipping them, so the window's content-derived minimum width drops from the
  sum of every visible section (~1633px, all six shown) to the width of the
  single widest section — and the window can now be dragged down to that,
  section-by-section, with no floor above `MIN_WINDOW_WIDTH`/
  `effective_min_window_width` biting first on most monitors.
- Wrapping is **whole-section, not per-button**: a section's buttons never
  split across two rows. This was a deliberate scope-narrowing from the
  original approach-2 sketch (which didn't specify a grain) — it keeps
  invariants I1–I7 in `viewactions.rs` (which all reason about "a section" as
  the atomic show/hide unit) exactly as they were; only the *container*
  arranging the six sections changed, not what a section IS. It also sidesteps
  approach 1's CAM concern (no button gets a second, popover presentation to
  keep in sync) since every button still has exactly one presentation, just a
  possibly-different row.
- Touched call sites: `chrome::build_chrome` and
  `viewactions::{register_view_actions, register_chrome_visibility_actions}`
  take `&gtk::FlowBox` where they took `&gtk::Box` for the toolbar container;
  the six section boxes themselves (`section_boxes: &[gtk::Box; 6]`) are
  unchanged, since section-level show/hide logic didn't need to change at all.
- Accepted trade-off, matching the original approach 2 "Cons": the toolbar's
  *height* now grows by a row on a narrow window, taking that space from
  `content_paned` (vexpand). This is the trade the user asked for explicitly
  when reopening this plan, so it's accepted here rather than flagged as a gap.
- **Not addressed by this change** (still open, per approach 2's original
  "Cons" and the plan's "Technical details preserved" section): the tab strip
  and the outline sidebar are unrelated non-wrapping/fixed-width contributors
  and are NOT covered by this fix. A window narrow enough that a single toolbar
  section plus the ~240px sidebar plus the tab bar still doesn't fit will still
  hard-lock or clip on those, just at a much lower width than before. Not
  reproduced/measured against the original 540px RDP session in this pass.
- Build/tests: `cargo check` clean; the full `gtk_suite` integration harness
  (`cargo test --features gtk-integration-tests --test gtk_suite`) passes at
  558/559, the one failure (`saferizer::viewport::…viewport_top_iter_matches_manual_read_on_a_scrolled_view`)
  unrelated to toolbar/chrome layout — not investigated further here, flag if
  it recurs on a clean `master`.
- **Not done / left as follow-up**: no manual verification against a real
  narrow display (RDP or otherwise) in this pass — only `cargo check` +
  `gtk_suite`, neither of which drives real window geometry/wrapping visually.
  `tests/MANUAL-TEST.md` was not updated with a wrap-specific check. TDD.md's
  §I5 rubric was not extended for the new wrap behavior. `sdd/system-overview.svg`
  was not touched (no module/dependency changed, only the toolbar's internal
  container widget — judged not an architecture change, but flag if reviewed
  otherwise).

### 2026-09-16 third follow-up: MIN_WINDOW_WIDTH floor was masking the wrap; GtkFlowBox replaced with a custom left-packing widget

Two further rounds of user feedback after the FlowBox change above landed,
both real bugs in what shipped, not test gaps:

**"Nothing changes at all" / still hard-locks.** The FlowBox wrap was
genuinely working (verified in isolation), but `MIN_WINDOW_WIDTH = 720`
(`src/window/mod.rs`) was never lowered alongside it. GTK's toplevel minimum
is `MAX(content_derived_minimum, size_request)`, so the unchanged 720px
explicit `set_size_request` floor kept winning that comparison on any normal
monitor and fully masked the wrap end-to-end — the toolbar could measure a
tiny content-derived minimum and it still wouldn't matter. Fixed by lowering
`MIN_WINDOW_WIDTH` to `360`, matching `chrome_fit::ABSOLUTE_MIN_WIDTH` as a
pure sanity backstop rather than a real floor, plus a new regression test
(`the_toolbar_wraps_instead_of_summing_every_sections_width`,
`src/window/mod.rs`) that asserts `MIN_WINDOW_WIDTH < widest_section` so a
future regression can't silently re-mask the wrap the same way again. (Two
full rounds of this being reported were also partly a testing artifact
unrelated to the bug: the user was re-testing a stale, already-running
process that predated each rebuild — worth remembering when "nothing
changed" is reported after a real fix.)

**"That is janky. It leaves odd gaps on the left... never hug the right
side."** Once wrapping was actually reachable, `GtkFlowBox` turned out to
have a real layout defect for this use case: it computes a single shared
column grid across every row it produces, so a wrapped row whose sections
differ in width from the row that established that grid renders
offset/padded rather than packed flush against the left edge. This isn't
fixable by FlowBox properties (tried `homogeneous: false`,
`min_children_per_line: 1` already) — it's inherent to how FlowBox lays out
a shared grid. Replaced it outright with a new hand-written widget,
`ToolbarWrapBox` (`src/widgets/wrapbox.rs`): a `gtk::Widget` subclass that
does its own pure greedy left-to-right, top-to-bottom bin-packing via direct
`measure()`/`size_allocate()` overrides — no grid, so every row starts at
x=0 by construction, and rows never drift toward the right edge. Two new
`#[gtktest::test]` cases in that file pin this directly: a child that
doesn't fit lands on a new row at x=0, and a hidden child leaves no gap.
`build_toolbar`/`build_chrome`/`viewactions` were re-pointed from
`&gtk::FlowBox` to `&crate::widgets::wrapbox::ToolbarWrapBox`; the six
`section_boxes: &[gtk::Box; 6]` and every I1–I7 invariant in
`viewactions.rs` are unchanged, since only the outer container swapped, not
what a section is or how it's shown/hidden.

Build/tests: `cargo check --lib` clean. `cargo test --features
gtk-integration-tests --test gtk_suite wrap` — 7/7 pass, including the
pre-existing toolbar-minimum-width regression test now pointed at
`ToolbarWrapBox`. Full `gtk_suite` re-run after this pass to check for
unrelated regressions (see task notes for that run's result at the time this
was written).

**Not done / left as follow-up:** still no manual verification against a
real narrow *display* (RDP or otherwise) — verification here is via a
freshly rebuilt local `scribobulate.exe` and the integration test suite, not
a live narrow monitor. Tab strip and sidebar remain out of scope, per the
prior section. `tests/MANUAL-TEST.md` and `TDD.md` §I5 were not extended for
the new widget.

### 2026-09-16 fourth follow-up: wrapping moved from whole-section to per-button, with a few explicit "closely related" clusters

Further user feedback on the now-working left-packed wrap: *"It should always
try to keep more icons at the top and only move over when needed not in
groups unless they are closely related like the arrow keys."* Wrapping whole
sections (the prior pass's deliberate scope-narrowing, to avoid touching I1–
I7) was too coarse: a section as big as Format wrapping in one piece could
push several small, otherwise-fitting sections down with it, when only a
handful of individual buttons actually needed to move.

`ToolbarWrapBox` itself needed no change — it already treats each of its
*direct children* as one atomic pack item; the fix was entirely in what
`build_toolbar` (`src/window/toolbar.rs`) hands it as children. A section is
no longer one `gtk::Box` holding all its buttons (which is what made
whole-section wrapping unavoidable — a plain `GtkBox` cannot itself wrap).
Instead, each section is now a flat `Vec<gtk::Widget>` of individually-
wrappable pack items — its own leading separator, then either a single
button or a small `cluster()` box — and every item in that list is appended
**directly** to the shared `ToolbarWrapBox`, so the wrap box's own
left-to-right packing operates per button/cluster across the whole toolbar,
not per section.

"Closely related" pairs/triples are kept atomic by wrapping them in a small
plain `gtk::Box` (`cluster()`, spacing 2, no separator) before appending —
since `ToolbarWrapBox` already treats each direct child as one unit, a
cluster just becomes one slightly-wider unit that is never split. Applied to:
Undo/Redo (edit), Back/Forward (view, the user's own "arrow keys" example),
the Preview/Edit/Split segmented mode group (view), Split Swap/Orientation
(split), and Zoom In/Reset/Out (zoom). The Format section is the one
deliberate exception: it stays a single opaque box exactly as before — not
decomposed into individually-wrappable items — because `format_box` is also
the Format focus-gate's `is_ancestor` root and the Stage-2 caret overlay's
row, both of which depend on it being one container; splitting its own
buttons across rows would have to thread that same ancestor relationship
through every possible row, for what is already the toolbar's single most
tightly-related command group (its own version of the "closely related"
carve-out).

Invariant I2 (`viewactions.rs`) is generalised, not broken: "a section is
shown/hidden as one atomic unit" now means every widget in that section's
item list gets `set_visible` together (a `Vec<WeakRef<gtk::Widget>>`
captured per section-toggle closure), rather than one container's
`visible`. `win.show-tbtn-<id>`'s observable behaviour — I1, I3–I7 — is
unaffected; only the *how* of I2 changed. Touched call sites:
`register_view_actions`/`register_chrome_visibility_actions`
(`viewactions.rs`) take `&[Vec<gtk::Widget>; 6]` where they took
`&[gtk::Box; 6]`; `Cmd` was re-exported from `crate::app` (it already existed
`pub(crate)` in `app::commands`, just not re-exported) so `toolbar.rs` could
build a shared `cmd_button`/`cmd_toggle_button` helper instead of repeating
the button-construction boilerplate per command loop.

Build/tests: `cargo check --lib` clean. `cargo test --features
gtk-integration-tests --test gtk_suite wrap` — 7/7 pass unchanged (the
existing `ToolbarWrapBox`-level tests didn't need touching — the widget's
own contract didn't change, only what gets handed to it). Full `gtk_suite`
re-run after this pass to check for unrelated regressions (see task notes
for that run's result at the time this was written).

**Not done / left as follow-up:** same gaps as the prior pass — no manual
verification against a real narrow *display*, tab strip/sidebar out of
scope, `tests/MANUAL-TEST.md`/`TDD.md` §I5 not extended. Additionally: no
new automated test specifically pins "two unrelated small sections both fit
on the top row even though a wide section between them doesn't" (the exact
symptom this pass fixes) — verified only by visual inspection of a freshly
rebuilt local exe, not asserted in `gtk_suite`.

#### On the "active-shrink is deliberately NOT done" decision's provenance

Checked briefly, per request: the comment (`src/window/viewactions.rs`,
"operator decision") first appears in `70a48ca` ("Initial public release of
Scribobulate"), a single squashed commit with no `Co-Authored-By` trailer
(unlike this project's other commits, which do carry one when an agent
authored the work). There is no separate record distinguishing a human call
from an agent's own judgment before the squash — the commit itself is the only
trace, and it does not say. Not investigated further per the request to keep
this brief.
