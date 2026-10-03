# Known Issues

**`Platform`** is one of **`Windows`** · **`Mac`** · **`Linux`** · **`Any`** — the platforms an
entry is known to affect, not where it was found. `Any` means reproduced on, or inherent to,
every platform; a named one means the others were checked and do not exhibit it. Before
narrowing an entry to a single platform, have that platform's peer seat fail to reproduce it
(POLICY § Manual integration testing) — behaviour found on one platform is not
platform-specific until someone else looks.

**`Scope`** is one of **`Test`** · **`Production`** · **`Project`** · **`Upstream`**.
`Test` affects only the suite or the pipeline; `Production` affects what a user runs;
`Project` is both. **`Upstream` means the defect is in a third-party library and we cannot
FIX it** — a workaround may exist, but the repair is not ours to make, so an `Upstream`
entry is not work waiting to be scheduled here. It is orthogonal to severity: an `Upstream`
entry can still be the worst thing in the register.

**Read an entry sceptically before building on it.** Across the five batches that emptied
this register down from eighteen entries, **four** recorded root causes were measured and
found WRONG, and three entries turned out not to be defects at all — one whose stated worry
was structurally impossible while a different, real defect sat underneath it, reachable only
because the reproduction was built anyway. An entry is a report plus somebody's best
inference at the time, and the inference ages worse than the symptom. Reproduce first; fix
the thing you measured, not the thing that was written down.

**Two tables, and the split is the point.** The first lists **open** debt — things someone
is expected to fix — carrying a severity you triage on. The second lists **closed** entries:
problems investigated to a finding of *no reachable fix*, kept precisely so nobody spends a
session rediscovering a settled dead end. A closed entry is **not** a fixed one; a fixed
issue is deleted outright, because this file is a snapshot of what is currently broken and
not a changelog. Closed entries carry a `CLSD-dd` number that is never reused or renumbered,
and they hold no severity, because they are not queued work.

**One defect can be filed twice.** A missing reading position, seen from two ends, was
carried here as two unrelated entries and was nearly fixed twice before anyone noticed they
were one thing. Before opening work on an entry, scan the others for the same mechanism
described from a different vantage point.

| ID | Platform | Scope | Issue | Severity |
|----|----------|-------|-------|----------|
| A | Any | Production | A large document leaves the process spinning a CPU core at ~100% while idle — a GTK/Pango relayout pass that re-shapes text every main-loop iteration and never converges | High |
| B | Mac | Upstream | macOS only: every native file-chooser invocation (Open, Save, Export) grows RSS by ~1.1 MB and does not give it back. Roughly four fifths is AppKit's own price for presenting an `NSSavePanel` — reproduced with no GTK in the process — with about a fifth GTK-attributable. Caching the panel upstream would recover ~95% | Medium |
| D | Any | Production | The preview's Annotate bubble sits over the line above a selection, so a click there can land on the bubble: in a table, a double- or triple-click on the cell above a selected cell can lose a press and act as a single click | Low |
| E | Any | Test | Flaky test: closing the outline's filter sometimes leaves the outline scrolled to the top rather than to the highlighted row. Failed twice on Linux CI, green on rerun and locally; cause not established | Low |
| G | Windows | Upstream | After an edit the editor's scrollbar slider is sometimes not drawn until the next scroll (2 of 40 Enters); a GTK defect still open upstream | Low |
| J | Windows | Test | Flaky test: overwriting a crash-recovery snapshot sometimes finds the old, shorter snapshot still on disk after the write loop ends | Low |
| L | Any | Production | After a link jump, Back or Cmd+Home sometimes scrolls only part of the way to its target (reported once; not reproduced on Linux or macOS) | Low |
| M | Windows | Test | Flaky test: a cancelled snapshot write sometimes leaves its temporary file behind on Windows, though the previous snapshot is intact | Low |
| N | Windows | Test | Flaky test: the GTK suite sometimes aborts on Windows on a `g_signal_handler_disconnect` critical near the comment-card tests (1 run in 7) | Low |
| O | Mac | Production | A document's `/net/<host>/…` image is canonicalized on render, which reaches the host through the automounter — but only where the user has enabled `/net`, which stock macOS 27 does not | Low |
| P | Any | Production | In Preview, Find Next can repeat a match inside a table after an external reload adds a match earlier in that table | Low |

## Closed issues

Intractable: no reachable fix, and the limitation is still real and present.
Do not reopen one without a new constraint. These numbers never change and are never
reused — unlike the letters above, which are positional and get reclaimed.

| ID | Platform | Scope | Issue |
|----|----------|-------|-------|
| CLSD-01 | Any | Upstream | Tables are selection islands; cells are individually selectable but not part of the continuous buffer |
| CLSD-02 | Any | Upstream | A paragraph that mixes fonts (any inline-code span) can lay out a few pixels wider than the wrap width it was given, summoning the preview's Automatic horizontal scrollbar and intermittently blanking the pane until a resize |
| CLSD-03 | Windows, Mac | Upstream | No screen reader on Windows or macOS can read the app's accessible names: neither backend publishes a provider tree (no UIA there, no NSAccessibility tree here), so every name the app sets is correct and unreachable. Linux/AT-SPI reads them |
| CLSD-04 | Mac | Upstream | In fullscreen, a click issued while the transition animation is still running is never delivered — AppKit blocks input for its own ~250-500ms window, in any Cocoa application. Not ours to fix, and not GTK's |


## CLSD-01. Tables are selection islands

**Status**: Closed (intractable — every exit is walled *within* GTK's selection
machinery, source-verified to a measured verdict below; the one theoretical escape
leaves those bounds only by becoming a different project. Real and unresolved, not
fixed — retained as a documented permanent limitation. Not actionable.)

The preview is a single `GtkTextView` buffer with `GtkTextTag`s for formatting.
All prose, headings, code blocks, **blockquotes**, and inline content participate in
continuous cross-document selection. Tables are embedded as `GtkTextChildAnchor`
islands (a custom `ScribTableWidget` holding `GtkLabel` cells, each
`set_selectable(true)`); a cell's text is selectable on its own, but a drag-select
cannot span from body text into a table cell, nor across cells, in one gesture.

**Continuous cross-cell selection is unavoidable** (researcher-verified, gtk-4-6): an
anchored child occupies a single `U+FFFC` object-replacement char in the buffer, and
the `GtkTextView` selection model treats it as one opaque unit with no path into the
child's text. There is no cross-widget continuous selection in GTK 4.6. (Blockquotes
moved into buffer text precisely to get continuous selection where it *was* possible;
tables can't, because they need 2-D widget layout.) A drag cannot span from body
text into a cell, but selecting text *within* a cell copies that cell's own Markdown
source character-precisely (each cell carries its own `copymap`, formatting preserved
— TDD 2.8f); a buffer selection overlapping the table anchor copies the whole table
source.

### ⛔ Unmitigable within GTK's selection machinery — investigated, probed, closed

**Do not re-open this on a re-read.** Both routes past the anchor were taken to a measured
verdict (researcher + probe, gtk-4.6.9). The obvious designs all look viable on paper and
fail only at runtime, which is why the negative results are recorded here rather than
rediscovered.

**1. Mid-drag promotion — "let the label start the drag, take over when the pointer
escapes the cell" — is impossible, not merely hard.** `GtkLabel`'s lazily-created
selection machinery (`gtk_label_ensure_select_info`, `gtklabel.c:4826`) includes a
`GtkGestureClick` that **claims on press** (`:4313`). A claimed sequence sets `DENIED` on
*every gesture on parent widgets in the propagation chain* (`gtkgesture.c:84-92`), and
**`DENIED` is terminal** (`:1020-1035`). So by the time the pointer escapes, an ancestor
gesture can never claim. Observation was never the problem — capture-phase ancestors *do*
see the motion; **claiming** is.

**2. GTK's own sanctioned escape hatch — "claim early, decide late"** (`gtkgesture.c:94-99`:
a capture-phase ancestor claims on press, then denies to hand the press back, GTK
*emulating* it) — **was probed and fails twice** (Xvfb, double-click on a word, reading
`selection_bounds()`):

| Setup | Selection | |
|---|---|---|
| control — no ancestor gesture | `(0,5)` = `"alpha"` | ✅ word-select works unaided |
| deny on drag-update only (the documented shape) | **`None`** | ⛔ label receives nothing |
| + deny on release (that gap patched) | **`(0,11)` = `"alpha bravo"`** | ⛔ silently wrong |

- The documented shape **has no branch that fires for a click** — a click produces no
  motion (`drag_update = 0`), so a deny placed on drag-update never runs, the sequence
  stays claimed, and the press never reaches the label.
- Patching that doesn't save it: double-click then selects **two words**. The ancestor's
  claim emits `::cancel` on the gestures underneath (`gtkgesture.c:88-89`) →
  `gtk_gesture_click_cancel` (`gtkgestureclick.c:282-288`) → `_gtk_gesture_click_stop`,
  which **zeroes the counter**: `priv->current_button = 0; priv->n_presses = 0;`
  (`:112-113`). **The claim wipes the multi-click state on the way IN, before any
  emulation** — and the emulation replays *one event*, not the counter, so it is
  structurally incapable of rebuilding it. **`gtkgesture.c:94-99`'s "one similar event will
  be emulated" preserves event *coherence*, not gesture *state*** — the docs tell the
  literal truth and still mislead anyone designing this. Corollary: pre-empting a
  **stateless** gesture is recoverable; pre-empting a **stateful** one is not.
  *(The counter wipe is source-verified; the exact accounting for why the result is
  precisely two words rather than two independent single-clicks is unexplained, and
  deliberately not guessed at.)*
- The failure is **plausible-but-wrong**, not empty — it would feed Copy silently. Adopting
  it would break double-click word-select, which works correctly today.

**3. A keyboard-only trigger survives but isn't worth building.** `GtkLabel::move-cursor` is
a public keybinding signal (`:2205`) that fires *before* the default handler clamps, so a
boundary escape is observable — and it pre-empts no gesture. But a table where Shift+Down
crosses cells and **dragging does not** is less coherent than today's honest dead-stop.

*(Related GTK facts established during this investigation, in case they're wanted
elsewhere: `GtkLabel` exposes no cursor position — the public getter normalises
`anchor`/`end` away, `:2118-2120` — and the PRIMARY-clipboard "hole" GTK4Rs/AP-28 once alleged
**does not exist**; see GTK4Rs/AP-28 / GTK4Rs/AP-120.)*

**The limitation is accepted, and the impact is small.** In-cell selection is already
char-precise (TDD 2.8f); a buffer selection over the table anchor already copies the whole
table source. Nothing here is broken — the feature simply cannot be added through GTK's
selection machinery.

**The one theoretical escape, priced honestly and not recommended**: drop
`set_selectable(true)` and have `ScribTableWidget` own selection outright — hit-test via
Pango `xy_to_index`, draw the highlight in a `snapshot()` override. This sidesteps gesture
arbitration entirely (exactly what defeated the routes above) and is *technically*
possible, so this entry says "unmitigable **within GTK's selection machinery**" rather than
"impossible" outright. But it means reimplementing char selection, double-click-word,
triple-click-line, keyboard selection and PRIMARY ownership — all of which GTK provides
free today, as the probe's control demonstrates — to un-break a minor limitation
nobody has asked for. It would be a deliberate project chosen on product grounds, not an
increment, and it should not be started from this entry.

## A. A large document pegs a CPU core at ~100% while idle (GTK/Pango relayout loop that never converges)

**Severity**: High (the symptom is a full CPU core held at ~100% **indefinitely while idle**,
which directly contradicts the product's negligible-footprint thesis — but it is gated to
LARGE documents, tens of thousands of lines; typical small files are unaffected. Agent-generated
reports and plans, the product's own primary use case, can be that large, so it is reachable in
normal use rather than a corner case.)

Opening a large document leaves the process at ~100% CPU **forever, even after it is fully
rendered and sitting idle with no input**. Characterised headless (Xvfb, release build of
2026-07-26) on `tests/fixtures/large-doc.md` (3 MB / 41,785 lines): `pidstat` averaged **99.83%**
across a 60 s idle window (20 samples, all ~100%, process alive throughout). A normal document
(`tests/fixtures/lists.md`) opened the same way idles at **~0%**, isolating the spin to document
size.

**Second consequence, established 2026-08-07.** While the layout is invalid, GTK keeps its
incremental line-height validation idle permanently ready, and that starves anything the app
schedules below it. Far navigation (Ctrl+Home/End, Go To Line, find, outline) is deferred until
validation completes for correctness reasons (GTK4Rs/AP-260), so on a document caught in this spin
that navigation would never arrive at all. It is bounded rather than exposed — the deferral
carries a timer-based deadline above the validate idle's priority, which degrades to a partial
landing instead of hanging — but that mitigation exists *because of this issue* and would be
unnecessary without it. Measured counter-point: a 200 000-line plain-prose file settles to 0 %
CPU in ~30 s and does **not** reproduce the spin, so whatever drives it is not size alone.

Surfaced during the macOS-port bring-up, where a stack sample suggested a GtkSourceView
incremental-highlighter feedback loop (its progress `mark-set` re-dirtying the highlighter's own
region). Confirmed here to reproduce on Linux — so it is **not platform-specific** — but an
independent trace on this side **does not support the highlighter theory**.

**That disagreement is now sharper, not resolved — read it with the Pango-shaping claim
below, which it contradicts.** MEASURED 2026-09-15 on the status-bar build: the macOS seat's `sample(1)`
put the main thread in 2320 of 2332 samples under `g_application_run` →
`g_main_context_iteration` → `idle_worker` (libgtksourceview-5.0) → `update_syntax` →
`gtk_source_region_add_subregion` → `gtk_text_buffer_set_mark` → `g_signal_emit`, on this
fixture with no interaction — the first trace to NAME the highlighter. The Linux side
re-measured the same fixture that day: 100% of one core across 60 s with no settling,
against 0% for a one-page control. So two traces of one symptom point at different
machinery; reconcile them before choosing a mechanism, and do not treat either as settled.

**The threshold is not size alone, in both directions.** `sdd/TDD.md` (~3,800 lines,
markup-dense) burns ~11 s and then **settles by itself**; `large-doc.md` (41,785 lines)
never converges; the 200,000-line plain-prose file above settles in ~30 s. A reproduction
attempt that varies only line count can therefore miss this entirely — vary the construct
mix too.

**This is not confined to the ordinary idle context, which widens both its reach and its
reproduction.** MEASURED 2026-09-16 on macOS/Quartz: exporting an 18.5 MB document of the
spin-prone shape ran past 100 s without completing, three times, where the same export had
taken 47 s the day before; `sample(1)` put the main thread in
`gtk_print_operation_run` → `print_pages` → `g_main_loop_run` — GtkPrintOperation's **own
nested main loop** — and one of the sources that loop was dispatching was GtkSourceView's
`idle_scan_cb` → `scan_region_forward` → `scan_subregion`, i.e. this entry's highlighter
idle, still re-arming. So the spin competes for any loop that services the same main
context, not just the one the application runs; an export on such a document is a *faster*
reproduction than waiting for it to show up as idle CPU.

⚠ **Do not read a stalled export as a broken Cancel.** The two are separable and look
identical from outside: MEASURED on Linux, a 31 MB export drew no pages for minutes while
the window repainted normally and a Cancel click WAS delivered and logged — cancel takes
effect only between pages, so with no page ever drawn a delivered cancel sits idle. Judge
that path by a log line at the click handler, never by the export ending.

⚠ **The `Any` classification rests on TWO platforms, not three.** Reproduced on macOS and
Linux; **Windows has never been asked**. `Any` is still the right call — the trace lands in
Pango text shaping under a recursive GTK measure/layout pass, which is toolkit machinery
common to every backend, so this is `Any` by *inherence* rather than by a third
reproduction, which the header's definition admits. Recorded because the header also tells a
reader that a platform label is evidence-backed, and here one third of that evidence is an
inference. A Windows reproduction would upgrade it; a Windows *non*-reproduction would be a
significant finding about the backend and must not be read as merely narrowing the label.

**Trace** (gdb `thread apply all bt` on the spinning process, main thread, at idle). Every
worker thread is parked (futex / `cond_wait`); the hot main thread is entirely in text SHAPING
under a recursive GTK measure/layout pass:

```
#0–7   libharfbuzz   hb_shape_plan_execute / hb_shape_full          (text shaping)
#8–14  libpango      pango_shape_item + layout
#15–24 libgtk-4      measure / allocate / snapshot   (frames #18–22 are ONE return address ×5 → recursive widget-tree measure)
#25–27 glib          g_main_context_dispatch → g_main_context_iteration
#28    gio           g_application_run → main
```

There are **no GtkSourceView / highlight / mark / region frames anywhere**, and **no app-own
frames in the hot path**. So the CPU burns in a GTK **relayout / re-shape loop that never
converges** — a `size_allocate` / `queue_resize` pass re-shaping the large widget tree's text
via Pango/HarfBuzz on every main-loop iteration — not the incremental highlighter.

**Symptom vs driver — not yet fully pinned.** One stop-sample shows *where* the CPU is spent
(Pango shaping under GTK measure), not *what keeps scheduling* the pass. The macOS-side
`mark-set`-handler theory therefore survives only as a candidate **driver**: a handler that
re-`queue_resize`s in response to a signal the relayout itself emits would produce exactly this.
The buffer's `mark-set` is listened for in four places — `window/tabs/lifecycle.rs` (`:95`),
`window/editbar/overlay.rs` (`:155`), `preview/interactions.rs` (`:23`),
`preview/annotate/overlay.rs` (`:843`) — which are the suspects to bisect. (Several already
coalesce because `mark-set` is chatty, so the culprit is more likely a coalescing timer that
keeps re-arming than a naive re-dirty.)

**Distinct from F** (same *preview-overlay relayout* family, opposite outcome): F is a **rare,
recoverable blank** from a `GtkOverlay` snapshotted without an allocation; this one is a
**permanent ~100% CPU spin** with no blank. (Both entries previously called each other N and O
— letters that no longer name them.)

⚠ **The `GTK_DEBUG=geometry` probe both entries recommended CANNOT RUN on the reference
host, and its silence is not evidence.** Measured 2026-08-04: a distribution GTK is built
without debug support, so every informational `GTK_DEBUG`/`GDK_DEBUG`/`GSK_DEBUG` key reports
`[unavailable]` and emits nothing — an empty log therefore means *the instrument is dark*, not
*no widget re-queued a resize* (GTK4Rs/AP-251). Restoring that key requires a locally built,
debug-enabled GTK loaded ahead of the distribution one; `sdd/PLAN.profiling.md` records the
cost and the alternatives.

**PREREQUISITE — [`sdd/PLAN.profiling.md`](PLAN.profiling.md) is implemented FIRST, not
alongside** (operator, 2026-08-28). This entry is the one place in the register with no
oracle: the trace says where the CPU goes and not what keeps scheduling the pass, the
`GTK_DEBUG=geometry` key that would answer it is dark on this host, and every mitigation
below opens with "take several samples". Doing that with ad-hoc instrumentation is how the
work becomes open-ended — which is why the budget for it has to be agreed up front. Build
the instrument, then aim it. The plan is also the place that records what a debug-enabled
GTK costs, so the decision about whether to pay it is made once, in the open, rather than
midway through a bisect.

**Mitigation options** (all of them assume the instrument above exists):
- **Root-cause the driver** (recommended; not yet done): take several samples to confirm the
  loop consistently sits in shaping/layout — `perf record` against the unstripped debug binary
  gives named application frames today, with no change to the tree, and is the substitute for
  the unavailable geometry key; then bisect the four `mark-set` handlers by
  disabling each and re-measuring idle CPU. If one stops the spin, that handler is the driver;
  if none does, the driver is not `mark-set`, and the search moves to whatever re-invalidates
  the (likely preview) widget tree's layout every iteration.
- **Likely fix shapes** (pending the driver): make the offending handler idempotent so it does
  not re-invalidate the region it reacts to; coalesce/gate the relayout so it converges; or
  ensure an incremental idle returns `G_SOURCE_REMOVE` once stable. Left open deliberately —
  fixing the wrong layer (e.g. throttling shaping) would mask the loop rather than end it.
- **Accept the limitation**: not viable long-term — an idle full-core spin on the product's own
  primary use case (large agent-generated documents) defeats the negligible-footprint thesis the
  project exists to honour.

## B. Every native file chooser invocation grows RSS on macOS

**Severity**: Medium. Monotonic within everything measured at the per-invocation scale, but the
cost is overwhelmingly AppKit's own price for presenting an `NSSavePanel`, and it is not
reachable by any change this project can make.

**Re-measured 2026-08-27 with a control, which sharpened the claim in both directions.** Ten
`File ▸ Open` invocations, CANCELLED every time so no document ever loaded, grew RSS
222,432 → 232,288 KiB — monotonic, never reclaimed, **≈985 KiB per invocation**. The control
is what makes that a cause rather than a coincidence: ten cycles at the same cadence, same
frontmost-and-Escape driving, chooser never opened, moved RSS by **+32 KiB total**. So the
growth is the chooser, not elapsed time and not the driving method.

**And the counter-evidence, recorded because it is the half that would otherwise be
mis-read.** A separately-observed instance sitting at 257 MiB after ~1.5 h of ordinary use
was NOT this issue accumulating: watched across a further window it went 257 → 251.5 →
230.5 MiB — *downward*. "Idle instance at a high RSS" reads as corroboration and is the
opposite. The per-invocation leak is real; something reclaims at a larger scale, and the
shape of that reclamation is unmeasured. Do not describe this entry as unbounded growth.

**Symptom**: opening a `GtkFileChooserNative` and cancelling it grows resident memory on
macOS, per invocation, and the memory never returns. Neither Linux nor Windows reproduces it.

**Status: ATTRIBUTED TO THE PLATFORM, and OWNER-BLOCKED.** Not a GTK defect and not this
project's reference discipline — both call sites were audited and cleared. Roughly 93% is
recoverable upstream by REUSING the panel rather than releasing it; a patch sketch exists,
must be authored twice because the 4.6 and current variants differ, and two naive forms of it
ship a use-after-free. The upstream filing is written and reviewed and **cannot be submitted
from any seat here** — it needs the operator's credentials or explicit instruction.

**What this project does about it**: nothing, deliberately. There is no application-side fix,
the exposure is one panel per invocation on one platform, and TDD §6's ceiling is not
threatened by it.

**The full investigation is `probes/native-chooser-rss-investigation.md`**, beside the probe
that produced it — measurements with their conditions, the retain-cycle analysis, the patch
sketch and its hazards, and the instrument failures that shaped it. It lives there rather than
here because this entry exists in order to be deleted when the defect is fixed, and the
evidence must outlive it. Do not restate its figures here; several carry caveats that do not
survive summarising, and the transferable lessons already have permanent homes in
`sdd/ANTI-PATTERNS.md`.

## D. The Annotate bubble covers the line above a selection and can swallow a click there

**Severity**: Low (a click lands on the bubble instead of the text beneath it; nothing is lost).

**Observed** (2026-09-30, Linux, bare Xvfb, GTK 4.6.9): select text in a table cell, so the
preview's **Annotate** bubble appears above the selection. The bubble's window covers the
text of the cell above. Double-click and drag in that cell, and only the first press
reaches it (instrumented at the cell's own click gesture): the double-click becomes a
single click and a drag that should extend by words selects characters. The same drive with
the bubble dismissed first behaves correctly. Probably what spoiled a triple-click the
operator reported. In body text the bubble covers the line above in the same way; not
separately driven there.

**Not established**: why the first press still reaches the cell while the second does not —
whether the bubble's window takes the second press, or its hiding/re-showing on the first
press resets the click count. Measure before choosing between the options below. Filed `Any` because the bubble's
placement is shared code; macOS and Windows have not been driven.

**Mitigation options**:
- Keep the bubble out of the pointer's way while a mouse button is down or a multi-click is
  still possible (show it after the double-click time has passed since the last press).
  Slightly slower to appear after a mouse selection; keyboard selection unaffected.
- Place the bubble clear of any text the reader may click next, e.g. in the margin or below
  the selection. Changes a placement other features and tests rely on.
- Accept it: the reader can dismiss the bubble by clicking elsewhere first.

## E. The sidebar filter's focus-restore test is flaky

**Severity**: Low (a test fails intermittently; no user-visible failure has been observed).

**Observed** (2026-09-30 and 2026-10-01, Linux CI): `window::sidebarfilter::gtk_integration_tests::closing_the_filter_returns_the_focus_to_the_highlighted_row`
failed twice — once on master (run 36750840804) and once on `bug/selection-color` under
coverage leg B (run 36804063069) — and passed on rerun and locally. The first failure
panicked at "the highlighted row is scrolled into view, not the top of the list": the
outline scroller's vertical adjustment was still `0.0` after the filter closed.

**Not established**: the cause. Suspected: the restore scroll lands on a later frame than
the frame-clock wait the test makes, which waits only for focus.

**Mitigation options**:
- Make the test wait for the scroll itself rather than for focus.
- Find whether the restore scroll is genuinely racy in the app; if it is, this is a
  Production defect rather than a test one, and the fix belongs there.

## G. On Windows the editor's scrollbar slider sometimes goes missing after an edit

**Severity**: Low (the slider alone, occasionally, and the next scroll brings it back).

**Observed** (2026-10-01, Windows, GTK 4.22.4 gvsbuild, release build, edit-only mode,
`sdd/POLICY.md`, scripted Enter-then-scroll drive): after an Enter, the editor's overlay
scrollbar draws its trough but not its slider. 2 of 40 Enters with the repair below in
place, 7 of 40 without it; 0 of 40 with no edit. Scrolling brings it back, as does a resize.
`Trying to snapshot GtkGizmo … without a current allocation` is logged with or without the
repair, and macOS logs it too with the scrollbar still drawn, so the warning does not
discriminate.

**Cause**: GTK, still open upstream — GtkTextView changes its scroll range from inside
paint (GNOME/gtk#6057, merge request !5222 unmerged), which leaves the scrollbar a frame
behind. Linux on GTK 4.6 had a worse, separate defect (the scrollbar stayed undrawn until a
resize, 24 of 25 Enters; fixed upstream in GTK 4.10 by !5564). The editor now hides and
re-shows its scrollbar after every scroll-range or position change
(`window::splitview::relink_vscrollbar_after_range_changes`), which ended the Linux defect
(0 of 30) and reduced this one. Not seen on macOS (0 of 20).

**Known cost of that repair, accepted**: the hide/show resets GTK's record that the pointer
is over the scrollbar, so with the pointer resting perfectly still on the scrollbar the
scrollbar now fades 1–2 s after each edit (Windows, 13 of 20) where it used to stay shown.
Any pointer motion brings it back. Judged negligible by the operator.

**Mitigation options**:
- Accept it until GTK fixes gtk#6057.
- Give the editor non-overlay scrollbars, as the preview has. Untested whether a classic
  scrollbar shows the same lag.

## J. A crash-recovery overwrite test is flaky on Windows

**Severity**: Low (the test; nothing shows the snapshot write itself is wrong).

`window::swap::tests::overwriting_a_snapshot_never_exposes_a_partial_file` failed on the
GitHub Windows runner on 2026-10-01 with "the new snapshot is the longer one": after the
test's wait loop ended, the file on disk still held the first, shorter snapshot. The same
commit passed on rerun, and on Linux and macOS. Seen once.

The wait loop ends as soon as the tab reports no write in flight. If it checks before the
second write has started, it ends at once and reads the old file; whether that is what
happened was not established.

**Mitigation options**:
- Wait for the snapshot file to change (or for a write-complete signal) rather than for
  "nothing in flight", which is also true before the write begins.
- Accept until it recurs, and capture the timing then.

## L. After a link jump, Back or Cmd+Home sometimes scrolls only part of the way

**Severity**: Low (navigation lands short; nothing is lost or changed).

Reported by the macOS seat, 2026-10-01: after following a link within a document, pressing
Back, or Cmd+Home to go to the top, sometimes stops part-way instead of reaching the
earlier position or the top. Frequency, document shape and whether the editor or the
preview was scrolling were not recorded, and the original document and steps were lost.

**Not reproduced** (2026-10-01, master at "Keep a table cell's triple-click count on GTK
4.8 and later"). Fixture: 1000 sections and 333 tables
(~470 KB), a link at the top to `#section-900`, preview mode, a fresh launch per run.
- Linux (GTK 4.6.9, private Xvfb, release): link then Back, and link then Ctrl+Home, both
  reach the top, warm and while the preview was still laying out (CPU ~100%).
- macOS (GTK 4.22.4/Quartz): Back (toolbar), Ctrl+Home and Cmd+Up all reach the top at
  0.3, 1 and 3 s after the jump, including a link followed at window map with the key
  0.3 s later, and in split mode.
- **Cmd+Home is not bound on macOS**: the view does not move at all. If the report used
  Cmd+Home, what was seen may have been "did not move" rather than "part-way".
- Back lands with the first heading at the viewport top and the page's top padding (a
  few pixels) scrolled off, where Ctrl+Home lands at absolute 0. Whether that is the
  "part-way" reported is not established.
- Windows not checked.

**Mitigation options**:
- Ask the reporter for the document and exact keys the next time it is seen, and
  reproduce from those before changing anything.
- Make Back to the top of a document land at absolute 0, like Ctrl+Home, if the padding
  offset above is judged a defect.
- Accept until it is reproduced.

## M. A cancelled snapshot write sometimes leaves its temp file on Windows

**Severity**: Low (the test's cleanup check; the previous snapshot stayed intact).

`window::swap::close_semantics_tests::a_cancelled_close_discards_the_temp_instead_of_promoting_it`
failed on the GitHub Windows runner on 2026-10-01: the destination still held the previous
snapshot, as the test requires, but its second check found GIO's temporary file
(`.goutputstream-XXXXXX`) still in the directory. The same commit passed on rerun, and on
Linux and macOS. Seen once.

**Not established**: why the temp was still listed. Unlike J, this test has no wait loop:
it writes, closes with a cancelled `Cancellable`, and reads the directory in one synchronous
stretch. One candidate is Windows completing the delete late (a file deleted while another
process, such as a virus scanner, holds a handle stays listed until that handle closes); it
was not measured. J is a different test with a different suspected cause (its wait loop
can end before the write starts); both are Windows-only and read a file straight after a GIO
operation, so check whether one mechanism explains both before fixing either.

**Mitigation options**:
- Poll the directory briefly for the temp to disappear before asserting, keeping the
  destination check strict.
- Accept until it recurs, and capture what holds the file then.

## N. The GTK suite sometimes aborts on Windows on a signal-disconnect critical

**Severity**: Low (one aborted suite run in seven; a rerun passes).

`GLib-GObject-CRITICAL: g_signal_handler_disconnect: assertion 'handler_id > 0' failed`,
promoted to an abort by `G_DEBUG=fatal-criticals` (exit `0xC0000409`, the MSVC fast-fail
GTK4Rs/AP-268 describes). It printed right after
`codeview::card::gtk_integration_tests::an_anchor_whose_annotation_is_gone_resolves_to_nothing`,
with `cancel_returns_the_card_to_its_read_state_and_discards_the_draft` started and not
finished. Measured on the Windows seat (GTK 4.22.4) on 2026-10-02: 1 failing run in 7 of the
full suite, on an unchanged tree. The card tests pass in isolation (3/3) and as a module (9/9).

**Checked elsewhere**: Linux, 0 in 8 passes over those tests (four full runs, each running
them in both the library suite and `gtk_suite`); macOS, 0 in 4 (its log, since macOS cannot
run with fatal criticals). At 1 in 7 those misses are suggestive, not conclusive.

**Not established**: who passes the zero. The Rust bindings' `SignalHandlerId` is non-zero
by type, so the call comes from C, most likely GTK's own teardown of a widget a previous
test left to deferred dispose. On macOS the same test deterministically emits a different
critical (`gdk_surface_thaw_updates`, the popover freeze-count class GTK4Rs/AP-305 records),
which points at the card's popover surface but proves nothing about Windows.

**Mitigation options**:
- Capture a backtrace on the next occurrence (a crash dump naming the GTK frame) before
  changing anything.
- Make the card tests destroy what they build and pump until disposed, so no teardown
  crosses into the next test.

## O. A `/net` image path reaches its host through the macOS automounter

**Severity**: Low (needs a non-default macOS setting).

macOS maps `/net/<host>` to the host's NFS exports when `/etc/auto_master` enables
`/net -hosts`. The image and link gates canonicalize a local path before deciding, so a
document naming `/net/<host>/x.png` would make the automounter contact that host as the
preview renders: an open-tracking beacon and a main-thread stall, not a credential leak.
The Windows UNC refusal does not reach it: `/net` is an ordinary absolute path to Rust's
path parser.

**Checked**: on stock macOS 27 the `/net` line is commented out and `/net` does not exist;
canonicalizing `/net/203.0.113.1/…` returns NotFound in microseconds (mac seat,
2026-10-02). The enabled case was not measured (it needs root).

**Mitigation options**:
- Refuse an absolute candidate that is lexically outside the document folder before
  canonicalizing it. That also closes any future automounted path, at the cost of the
  "blocked" versus "not found" distinction for local escapes, which then cannot be told
  apart without touching the filesystem.
- Accept: the exposure needs a setting the user chose.

## P. In Preview, Find Next can repeat a match inside a table after a reload

**Severity**: Low (one repeated step; the count stays right).

A preview find hit is held by its buffer position, but every cell hit of one table shares
the table's anchor position. When an external reload adds a match earlier in the same
table, the held hit resolves one place early and Find Next lands on the match already
shown. Hidden hits in a collapsed block share their block's summary position the same way.
Found by QA review (round 5); declined as costing more than it is worth, and recorded so
the limit is not rediscovered.

**Mitigation**: give each cell hit a stable key (row, column, byte offset) and compare on
it when resuming.

## CLSD-02. A paragraph that mixes fonts lays out wider than the wrap width it was given

**Status**: Closed (no public API at the GTK 4.6 floor makes the layout report a width the
wrap budget respects; the two reachable correctives both cost more than the defect)

**Platform**: Any — the mechanism is Pango's line-extent accounting, not a backend's. Only
Linux/GTK 4.6.9 was measured; font metrics differ per platform, so *which* window widths
exhibit it will differ, not *whether* it can.

A `GtkTextTag` that changes the font FAMILY over a character range — in this project, every
inline-code span — splits the paragraph into separate Pango items at the tag boundary. A space
that lands on a wrap point is granted for free by the break logic (`find_break_extra_width`),
but the routine that collapses that hanging space afterwards (`zero_line_final_space`) is keyed
on the last run's last glyph, which is a different object once the items are split. The space
therefore stays, sitting a few pixels past the wrap width, and `GtkTextLayout` reports the
line's LOGICAL extent — hanging space included — as the layout width. That becomes
`hadjustment.upper`, which exceeds `page_size`, which summons the Automatic horizontal
scrollbar, whose appearance and disappearance re-arms the width↔height-for-width churn that
leaves the preview stuck blank until a manual resize (GTK4Rs/AP-22, GTK4Rs/AP-23).

MEASURED (GTK 4.6.9, gtk4-rs 0.10, X11/Xvfb, `#[gtktest::test]`, this repository's own
`sdd/ANTI-PATTERNS.md` as the corpus): a sweep of 41 window widths (600–1000 step 10) at zoom
1.0 found 2 widths over-wide, by 5px and 7px. Isolation is decisive in both directions —
removing ONLY the tag's `set_family` takes the overflow to zero at every width and zoom tried;
removing ONLY the tag's `set_wrap_mode` changes nothing (wrap mode is a paragraph attribute
taken from the view, so a character-range tag never alters it), and the tag's background does
not participate in width.

Impact is narrow and real: on roughly 5% of window widths for a code-dense document the reader
gets a horizontal scrollbar it cannot use and a pane that intermittently blanks while
scrolling. It is invisible at every other width.

**What walls each exit** — recorded so the dead ends are not re-explored:

- **Reserve slack in the wrap budget** (extra `right_margin`, CSS padding, or the private
  `gtk_text_layout_set_screen_width`) — REFUTED BY MEASUREMENT, not by argument. Any change to
  the wrap budget moves the breakpoint, so the failure RELOCATES rather than clearing: the same
  41-width sweep failed at exactly 2 widths with 0px, 8px and 16px of extra right margin, only
  at different widths each time. A single-width control cannot see this, and reads as a fix.
- **Derive the slack from the fonts' space advances** — the quantity does not exist. The hang is
  however much of the granted glyph sits past the wrap point plus accumulated shaping error at
  the item seams plus the layout's `ceil`, not a glyph metric: measured hangs of 4px and 6px
  against a body space of 3px and a code space of 8px. Any constant is a guess, and it would
  relocate anyway per the point above.
- **Clamp `hadjustment.upper` down to `page_size` when the excess is below a threshold**, from a
  `size_allocate` override after chaining up. This one WOULD close the invariant without
  relocating, and does not re-arm the churn. Declined: it is a symptom gate, not a wrap fix; the
  threshold can only ever be an observed bound from a width sweep rather than a derived
  quantity; and because the hang is not always pure whitespace it may clip 1–3px off a real
  glyph — trading a rare scrollbar for rare silent truncation of the reader's text.
- **Drop the monospace family on inline code** — removes the trigger completely and is the
  positive control that proves the mechanism. Declined: inline code reading as code is the
  product, so this trades a rare layout defect for a permanent, universal regression in what the
  reader sees.
- **`hscrollbar_policy = Never`** — banned outright and independently of this entry: it makes
  `GtkScrolledWindow` adopt the child's minimum width and ratchet, so the window can no longer
  shrink to fit (GTK4Rs/AP-139).

**Mitigation options**:
- Accept the limitation (chosen). A reader who hits it can resize the window a little; the
  defect is a property of the width, so any nearby width clears it.
- Revisit if the toolkit floor rises — this was checked against GTK 4.12 and the width
  computation is unchanged, so a fix would have to come from Pango's collapse logic rather than
  from GTK.
- Revisit if the clamp above stops being a symptom gate — if a way appears to distinguish a
  hanging space from a clipped glyph, the clamp becomes safe and this reopens.

## CLSD-03. No screen reader on Windows or macOS can read the app's accessible names

**Status**: Closed (inherent to GTK4's Windows and Quartz backends — the app sets the
names correctly and neither platform publishes a tree that can carry them)

⚠ The `Platform` cell reads `Windows, Mac` rather than one of the four single values the
header defines: the consequence is identical on both and the causes are backend-specific,
so filing it twice would be the header's own "one defect filed twice" trap, and `Any` would
be false — Linux/AT-SPI reads these names correctly.

MEASURED on both seats while ratifying the status bar (2026-09-15). **Windows** (GTK 4.22.4
gvsbuild, Win10 19045): UI Automation returns the toplevel (class `gdkSurfaceToplevel`)
with **zero descendants**, against a positive control of Notepad returning two. **macOS**
(GTK 4.22.4/Quartz): the window exposes 4 chrome elements and its "entire contents" is
**empty**, with the reader validated in the same run (the native menu bar enumerates, and
Save/Save As report their real enabled states). So no screen reader on either platform
reaches any control — not the toolbar, not the sidebars, not the status-bar indicators.

The consequence for verification is the part that misleads: the accessibility rubrics
(TDD 16.5, 16.7, 16.17) are **unrunnable** on Windows rather than failing. Their names
ARE set — `a11y.rs` is the single choke point and `clippy.toml` bans the bare tooltip
setter — and they are read correctly by AT-SPI on Linux, so a Windows run that reports
these checks as "not observed" is reporting this gap, not a defect in the code under test.

**Mitigation options**:

- **Accept it** — the position taken. Every exit is walled outside this project: the
  provider tree is GTK's to publish, there is no application-side API to attach one, and
  writing a UIA provider for another toolkit's widgets is a different project.
- **Verify these rubrics on Linux and macOS only**, and record the Windows limb as a
  platform gap in the run rather than as missing coverage.
- **Re-check on a future GTK** — if the Windows backend ever gains a UIA bridge this
  reopens at Low/Medium/High, since the names are already in place to be read.


## CLSD-04. In fullscreen on macOS, a click during the transition animation is never delivered

**Status**: Closed (no repair of ours is reachable). Kept so the
investigation is not run a second time.

**The report**: on the macOS build in fullscreen, clicking a toolbar button did nothing and
a second click was needed; the menu bar behaved the same way, and a toolbar button appeared
to activate on the wasted click.

**All three parts are accounted for, and none is a defect in this project or in GTK.**

- **The wasted toolbar click is AppKit's animation blocking its own input.** Measured as a
  timing sweep from the zoom button's mouse-up to the test click, real input throughout, no
  synthetic pointer placement: 0 ms, 100 ms and 250 ms all fail **silently** — the click
  never reaches GTK at all, invisible even to window-level instrumentation — while 500 ms,
  1 s and 2 s all work. The boundary matches an ordinary `NSWindow` fullscreen transition's
  duration. Every Cocoa application has this; a reader who hits the green button and reaches
  straight for a toolbar command is clicking inside that window.
- **The menu bar needing two clicks is macOS auto-hiding it in fullscreen**: a cold click at
  the top edge is spent on the reveal.
- **The phantom activation was a keyboard focus ring**, which in a screenshot is
  indistinguishable from a button that was just pressed. Confirmed by Escape moving the ring
  with no click involved. Assert on whether an action fires, never on what a screenshot
  looks like.

**A genuine toolkit defect was found on the way and is NOT this.** After a fullscreen
transition has fully settled, placing the pointer with `CGWarpMouseCursorPosition` — which
moves the cursor without posting a motion event — and then clicking makes the first click
skip the picked widget's own controllers entirely, while an ancestor's capture-phase gesture
still sees it and `pick()` resolves correctly. Reproducible with zero application code on
GTK 4.22.4 / macOS 27.0; `probes/macos-fullscreen-first-click.c` holds the measurement. No
mouse or trackpad gesture teleports a cursor, so no user meets it — it is recorded as an
upstream curiosity, not as this entry's subject.

**Dead ends, each closed by measurement, and not to be revisited**: a coordinate-space or
title-bar-origin offset; the macOS behaviour where the click that activates an inactive
window is not delivered; lost or mis-picked input; a stale implicit grab; this project's own
macOS pointer-crossing seam; a disabled action or insensitive widget; and any
ours-versus-upstream asymmetry — both binaries agree once entry and pointer arrival are held
constant. Returning to a fullscreen Space from another application is also clean.

**Linux and Windows have not been checked**, so the `Mac` narrowing is provisional — the
register's rule is that behaviour seen on one platform is not platform-specific until a
peer seat looks.
