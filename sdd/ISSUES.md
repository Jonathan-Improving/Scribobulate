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
| D | Any | Production | A large document leaves the process spinning a CPU core at ~100% while idle — a GTK/Pango relayout pass that re-shapes text every main-loop iteration and never converges | High |
| F | Mac | Upstream | A GTK4/Quartz autorelease-pool crash SIGABRTs the macOS integration suite in roughly one full run in four, at a varying site | Medium |
| I | Mac | Upstream | macOS only: every native file-chooser invocation (Open, Save, Export) grows RSS by ~1.1 MB and does not give it back. Roughly four fifths is AppKit's own price for presenting an `NSSavePanel` — reproduced with no GTK in the process — with about a fifth GTK-attributable. Caching the panel upstream would recover ~95% | Medium |
| M | Windows | Production | On a machine with no Visual C++ runtime the app installs and then fails to start; the installer's bootstrapper for it has landed but has never been verified against that condition | Medium |
| U | Any | Production | The preview is drawn horizontally scrolled (~20px, its left padding gone, a horizontal scrollbar showing) after a mode switch or an explicit Reload rebuilds it — intermittent, pre-existing, seen on Linux and Windows | Low |
| X | Mac | Test | The macOS integration suite hangs part-way through a run, at a varying site, in roughly two to four runs in five. Independent of any one feature — it survives removing the surface it was first blamed on | High |

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
**does not exist**; see GTK4Rs/AP-28 / ScrAP-135.)*

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

## D. A large document pegs a CPU core at ~100% while idle (GTK/Pango relayout loop that never converges)

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
validation completes for correctness reasons (ScrAP-260), so on a document caught in this spin
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
*no widget re-queued a resize* (ScrAP-251). Restoring that key requires a locally built,
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

## F. A GTK4/Quartz autorelease-pool crash intermittently SIGABRTs the macOS integration suite

**Re-measured 2026-09-14 by the macOS seat — consistent with the existing rate, NOT a measured
increase** (entry content theirs, edited here for format). Full `gtk_suite` runs, same machine:
0 aborts in 6 at one commit, then 3 in 6 and 3 in 7 on the next two — 6 in 13 (46%) once a test
that wrote the system clipboard had been added. That reads like an increase and is not one: at
this entry's 25–33% rate, six clean runs in a row happen 9–18% of the time, and the interval on
6 in 13 spans roughly 19–75%, so the data cannot separate the two commits. An attribution to the
new test was made and RETRACTED for exactly that reason, and because it contradicts the 93
filtered runs below — the fault needs suite depth, not a particular body. The aborting body
wandered across four tests, including this entry's dominant site.

What IS new is the stderr text, where this entry records only the `.ips` termination — two
libobjc messages of one fault class:
`Invalid or prematurely-freed autorelease pool 0x… Invalid autorelease pools are a fatal error`,
and `autorelease pool page 0x… corrupted / magic 0x0f9fdca3 … should be 0xa1a1a1a1 … / pthread
0x1f6b22180 should be 0x1f6b22180`. In the page-corruption case the pthread values MATCH: the page
was damaged in place on its own thread, not popped from a different one. That argues against a
cross-thread pop and for an accumulating in-place imbalance, which agrees with the depth finding.

**Removing that test's clipboard traffic did not make the suite complete.** On the next commit,
where the test asserts the paste hand-off without touching the clipboard, two consecutive full
runs still aborted — at an annotation-card body and a find body, neither of them clipboard work —
while the clipboard-free test itself passed; the pthread values matched again. Two runs are not
a rate and no change in rate is claimed either way; what they settle is that the abort is not
this test's, so do not read the clipboard-free test as having addressed this entry.

**Re-measured 2026-09-13 by the macOS seat (entry content theirs, edited here for format),
and two claims further down were too narrow.** Same defect: the same OBJC termination
(`namespace OBJC, flags 646, code 1`), `objc_autoreleasePoolPop` →
`AutoreleasePoolPage::busted_die`, the nested `CFRunLoop` resolving `NSPasteboard` promised
data, HIToolbox input-method session activation, and
`+[NSTextInputContext currentInputContext_withFirstResponderSync:]` → `discard_preedit`; the
same dominant site (`select_all_stands_down_for_every_text_entry_and_recovers_for_the_editor`)
and rate (one in four full pipeline runs). But this report reaches `discard_preedit` through a
**focus crossing**, not a mark-set — `gtk_text_view_mark_set_handler` appears nowhere in it:

```
gtk_widget_grab_focus_self
  -> gtk_window_root_set_focus -> synthesize_focus_change_events
  -> gtk_widget_handle_crossing -> gtk_event_controller_handle_crossing
  -> gtk_event_controller_focus_handle_crossing -> g_signal_emit
  -> discard_preedit -> (AppKit / HIToolbox / libobjc, as below)
```

And a Scribobulate frame IS on that thread, as the caller: `window::findbar::wire_find_bar`'s
closure calling `WidgetExt::grab_focus` from a `SimpleAction` activation. Nothing of ours is
faulting — every faulting frame is libobjc, AppKit or HIToolbox — so the entry stays
`Upstream`; the sentences below are corrected to the claims that survive. Evidence:
`~/Library/Logs/DiagnosticReports/gtk_suite-a0aba6a9f8498e0e-2026-09-13-115631.ips`, macOS
26.6.2 (25G83), GTK 4.22.4.

**Re-measured 2026-08-31 by the macOS seat, and the DISCRIMINATOR is now sharp.** 5 aborts
in 15 full `gtk_suite` runs — **33%, one in three**, spread across two trees (3 on one, 2 on
the other), so it is unmoved by the work that happened to be under test. Abort case indices
234, 234, 235, 16, and one inside a pipeline run: clustered, with one far outlier. And the
finding that narrows it most: **0 aborts in 93 FILTERED runs** of two cases per process. It
needs suite DEPTH, not any particular body — consistent with an accumulating pool imbalance
rather than one bad test, and it means a bisect-by-test cannot reach it.

**Re-measured 2026-08-27, and the rate and shape are both narrower than first recorded.**
Roughly **one abort in four FULL pipeline runs**, not two in three, and the abort site varies
rather than concentrating on the focus-churning test — the observed one was a find-cursor
test. The discriminator: `gtk_suite` run standalone, three times consecutively, passed clean
every time (323 passed). So this is a property of the FULL run rather than of any one test,
which is what a fix would have to account for and what a bisect-by-test would never find.

**Severity**: Medium (the macOS GTK suite cannot be trusted to complete; no data at risk,
and no Scribobulate code is faulting — but a red run there means nothing until re-run)

`cargo test --features gtk-integration-tests --test gtk_suite` intermittently aborts the
whole test process on macOS. Not one specific test — whichever happens to trigger a
focus crossing or text-view mark-set at the wrong moment relative to macOS's input-method state.

**Measured** via four independent **Apple crash reports** — the system crash reporter, not
a Rust panic (`termination: {namespace: OBJC, flags: 646, code: 1}`) — across four separate
runs. That distinction is the one that matters diagnostically: the Rust harness reports
only that the process died, which is equally consistent with a defective test, and the
discriminating evidence exists only at OS level. All four stacks are identical in
signature:

```
gtk_text_view_mark_set_handler                                   (libgtk-4.1.dylib)
  -> discard_preedit                                             (libgtk-4.1.dylib)
  -> +[NSTextInputContext currentInputContext_withFirstResponderSync:]   (AppKit)
  -> TSM input-method session (de)activation                     (HIToolbox)
  -> nested CFRunLoop pump for NSPasteboard promised-data resolution
  -> objc_autoreleasePoolPop -> AutoreleasePoolPage::busted_die() (libobjc.A.dylib)
```

**No Scribobulate frame is faulting.** Our code appears, when it appears at all, only as the
caller that reaches the toolkit path (2026-09-13: the find bar's `grab_focus`). The cause is
GTK4's Quartz backend firing `discard_preedit` on a `GtkTextView` focus crossing or mark-set —
so on a focus change as well as on any caret or selection change — which activates/deactivates the macOS input-method bridge, which
pumps a nested run loop for pasteboard-promise resolution and corrupts the autorelease
pool stack. Not reachable from application code.

**Rate and distribution** (n=6 full-suite runs, isolated): **4 crashed, 2 clean — about
two in three.** Of the 4 crashes, **3 were on the same test**,
`select_all_stands_down_for_every_text_entry_and_recovers_for_the_editor`; the single
outlier was the first observation, which was also a contaminated run (a concurrent build
on the same machine). All 4 crash reports carry a byte-identical stack signature.

**Why this is still not filed against that test**, even though it is the dominant trigger
— the argument is the stack, not the distribution. No application frame faults in it, so
nothing in that test's *code* is faulting; what the test does is arrive at the toolkit
path more often. It exists to verify select-all standing down across *every* text entry,
so its body is mostly rapid focus-switching between entries — which is precisely what
drives `discard_preedit` / `NSTextInputContext` activation churn. A test that exercises
the mechanism hardest crashing most is consistent with the mechanism, not evidence of a
defect in the test.

> **An earlier version of this entry claimed the opposite and was wrong.** On n=3 it read
> "2 crashed on *different* tests, so a defective test would fail on the same one every
> time" — and offered that distribution as the load-bearing proof. A larger sample
> inverted it: the spread was an artefact of a small n whose one cross-test data point
> came from the contaminated run. The mechanism argument survived unchanged because it
> never rested on the distribution; the distribution argument did not. Recorded because
> the retracted reasoning is more instructive than the correction — a frequency pattern
> read off three samples is a hypothesis, and it was stated here as evidence.

**Unverified**: whether it reproduces on other macOS or GTK versions. Linux and Windows
have no equivalent Quartz/AppKit/TSM path, so it is plausibly macOS-only *by
construction* — but that is an argument, not a test result.

**Mitigation options**
- **Re-run and treat a single abort as inconclusive** — what the macOS seat does today.
  Cheap, but it means the suite's silence is weaker evidence there than on Linux.
- **Raise it upstream** with the two crash reports. The stack is specific enough to be
  actionable and nothing about it is project-specific.
- **Re-test on a newer GTK** when one is available; this is the kind of interaction an
  upstream fix moves without anyone here doing anything.

Measured on macOS 26.6.1 (25G76), GTK 4.22.4 (Homebrew), by the macOS seat. Primary
evidence is machine-local and not transferable:
`~/Library/Logs/DiagnosticReports/gtk_suite-9047c36e3af692e9-2026-08-07-232935.ips` and
`…-2026-08-08-015722.ips`.

---

## I. Every native file chooser invocation grows RSS on macOS

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
leaves the preview stuck blank until a manual resize (ScrAP-22, ScrAP-23).

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
  shrink to fit (ScrAP-23a).

**Mitigation options**:
- Accept the limitation (chosen). A reader who hits it can resize the window a little; the
  defect is a property of the width, so any nearby width clears it.
- Revisit if the toolkit floor rises — this was checked against GTK 4.12 and the width
  computation is unchanged, so a fix would have to come from Pango's collapse logic rather than
  from GTK.
- Revisit if the clamp above stops being a symptom gate — if a way appears to distinguish a
  hanging space from a clipped glyph, the clamp becomes safe and this reopens.

## M. The Windows installer's Visual C++ runtime bootstrapper is unverified

**Severity**: Medium (a first-run failure on a clean machine, and the last thing the
installer does is launch the app — so it reads as "it would not install")

`scribobulate.exe` and the staged GTK tree import `VCRUNTIME140.dll` (plus
`VCRUNTIME140_1.dll`, via `cairo-2.dll`). Windows does not ship it; the
`api-ms-win-crt-*` imports beside it are the UCRT, which it does. The installer neither
installs it nor checks for it, so on a machine that has never had a Visual C++
redistributable the app cannot start. `scribobulate.iss`'s `[Run]` section launches the
app post-install, so the failure is the last thing the user sees.

**MEASURED** by the Windows seat (`dumpbin /DEPENDENTS`, VS2022 14.44.35207): 33 staged
modules import `VCRUNTIME140.dll`; zero CRT DLLs are staged; the gvsbuild prefix has none
to stage. **A standing gap, never a regression** — `git log` on `scribobulate.iss` shows
one commit in its whole history, and `git log -S vcruntime -- packaging/windows/stage.ps1`
is empty, so this line has never shipped it.

⛔ **Do NOT fix this by staging `vcruntime140.dll` into `stage.ps1`.** That is the obvious
move and it is the one the remedy below explicitly reversed: copying the DLLs in makes the
project a redistributor of Microsoft's Distributable Code, whose terms require an
end-user click-through that no file vendored into this repository can present. The licence
problem arrives with the DLLs.

**THE REMEDY IS NOW IN THIS TREE**, landed by the `ci` merge: `scribobulate.iss` carries a
`PrepareToInstall` `[Code]` block that runs Microsoft's own `vc_redist.x64.exe` when a
registry probe finds the runtime absent or below the embedded redist's version, its
`dontcopy` source entry, and the redist discovery in `package.ps1`. Running Microsoft's
installer is what satisfies the click-through, which is why that shape was chosen — the
project never becomes a redistributor. The `stage.ps1` half of "Stop redistributing Microsoft's C runtime" (2026-08-14) is *removal* of an
app-local copy, and it merged as such: nothing in the staged tree copies a CRT DLL.

**A FIELD DISCRIMINATOR, so a future report can be placed without a clean image.** The
bootstrapper is EMBEDDED, so it shows up in the artefact's size: a `ci`-line installer
measures ~37.7 MB (39,509,846 bytes, measured by the Windows seat, 2026-08-30) against
~15.7 MB for a master-line build with no bootstrapper. Cite the SIZE CLASS, never the
constant — the same build shape already moved from 38,595,643 bytes at that change. That is
also independent evidence the bootstrapper is WIRED rather than merely present in the
`.iss`, which the `.iss` alone cannot show.

**INDEPENDENTLY CONFIRMED FROM CI, which closes the weaker half.** The hosted Windows
runner's artefact measures 39,146,361 B (run 33357529291), squarely the bootstrapper size
class — so the artefact CI publishes demonstrably carries `vc_redist.x64.exe`, established
from a machine that is not the Windows seat's own. That answers "does the shipped artefact
contain the remedy at all" and leaves untouched the question below.

**WHY THIS ENTRY IS STILL OPEN, and it is the part to read before closing it: the remedy
has never been verified against the condition it exists for.** Every machine able to build
this project already has the CRT, so a staged launch on a build box proves nothing either
way. The observation that means something is a PAIR — runtime absent with the bootstrapper
disabled must fail to start, and the bootstrapper must then make it start — and that needs
a clean Windows image no seat currently has. An unverified remedy in the tree is not a
smaller problem than one on a branch; it is the same problem wearing a green tick, which is
why the severity is unchanged. Two live possibilities also remain open: the original report
may have come from a `ci` build, in which case the fault is *in* the bootstrapper (a 32-bit
Setup reading a redirected registry view, a declined elevation, a redist below the compiled
floor) rather than in its absence.

**The attribution half is discharged.** `THIRD-PARTY-LICENSES.md` is now generated from
`notices/*.md` at build time, and `notices/20-msvc.md` covers the embedded
`vc_redist.x64.exe`. That was the other obligation this entry was carrying; only the
verification remains.

---

## U. The preview is drawn horizontally scrolled after a mode switch or an explicit Reload rebuilds it

**Severity**: Low (cosmetic: the content is shifted about 20px left — the pane's left padding —
and a horizontal scrollbar shows; nothing is lost, and a width change corrects it)

First reported by the Windows seat (GTK 4.22.4 gvsbuild, release build, 2026-09-13) on the
first entry into Split after launch. **Now measured on Linux and Windows, and pre-existing**:
the Windows seat reproduced it identically across two successive builds (2026-09-14), and
Linux (Xvfb, the later of the two) shows both triggers below. **macOS not yet checked.**

**Measured**:

- **The first entry into Split after launch.** A plain document, `# Swap check` and one
  sentence — no inline code, no line near the wrap width. Fresh launch with the file as an
  argument, Preview, then Split: the preview's heading is drawn flush at the pane's left edge,
  about 20px left of the unscrolled layout, with a horizontal scrollbar showing. Windows
  (PrintWindow, 1336x759) and Linux (Xvfb). The built-in welcome document after a session
  restore did the same on Windows, with its code block's box shifted by the same 20px.
- **The toolbar's Reload** (`win.reload`) in Preview, even over an unchanged file, and **the
  toolbar's Edit-then-Preview toggle**. Frequent but **not deterministic per gesture**: the first
  Reload after launch shifted it 8 of 8 in earlier Windows sessions, yet a later session's
  alternating Reload/toggle runs went normal five times, shifted four, then normal; either
  gesture can cause it and either can clear it. Linux shifted on its one Reload. A second
  Reload never corrected it on Windows; an auto-reload from an external write corrected it 5
  of 5; widening the window corrects it and restoring the width does not bring it back.
  Parking the pointer over the button without clicking does not cause it, so it is not a
  hover-revealed scrollbar.
- **Windows' Split in a later session stayed normal** 4 of 4, including three Reloads, so the
  first-entry trigger above is not reliable either.
- Windows used two detectors per capture, each validated on known-normal and known-shifted
  frames: the scrollbar row's pixel, and the heading's leftmost ink x (≈275 normal, ≈255
  shifted, in a pane starting at ≈253). **Not seen** on Windows: the same document after Swap
  Panes in the same process.

**Not issue J**, on all three of J's axes: no paragraph mixes fonts, no line sits at a wrap
point, and the symptom is a nonzero horizontal adjustment VALUE (content displaced by about
the left padding) with the pane drawn, not blanked.

**Inferred, not probed**: every gesture that can shift it rebuilds the preview through the
view-mode handler — a mode switch, and the explicit Reload, which re-issues the current mode —
while both things that reliably correct it do not: the auto-reload builds a fresh preview and
installs it with its reading line restored, and a width change forces a fresh allocation. So
the likely shape is a race in the view-mode rebuild's first allocation, where the horizontal
adjustment's `upper` briefly exceeds `page_size`, `value` lands near the padding width, and
nothing clamps it back when `upper` shrinks.

**Mitigation options**:

- **Treat the adjustment's settling as a dark pattern**: how and when GTK clamps an
  adjustment's value when `upper` shrinks during a first allocation is not documented, so the
  researcher should establish that before a fix is written, rather than resetting `value` by
  guesswork. Alternating toolbar Reload and Edit-then-Preview about ten times in Preview gives
  a reproduction within a session.
- **Compare the rebuild paths**: the auto-reload's rebuild has not been seen to shift, so
  building the preview the same way from the view-mode handler might remove it without
  touching adjustments — to be established by that research, not assumed.
- **Accept it** while it stays cosmetic.

---

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


## X. The macOS integration suite hangs intermittently, at a varying site

**Severity**: High (it is the reason a macOS ratification cannot be read at face value. A
hung run produces no verdict, so every macOS result now costs several runs to interpret,
and a real regression introduced on that platform would be indistinguishable from this.)

A full macOS integration run stops part-way through and never finishes. The site moves
between runs — `copy_full_path_for_tab` twice in the most recent set, elsewhere before
that. Measured rates across one day of arms, five runs each: **3 in 5**, **4 in 5** and
**2 in 5** under three different tree configurations.

**It is NOT caused by the status bar, and the effort to prove otherwise is the useful part
of this entry.** The hang first appeared alongside the status bar's arrival, so the surface
was the obvious suspect. Four separate hypotheses were each armed and measured — destroy-time
timers, the pooled counter's latch, the mark-set handler's attachment, and finally the
status bar's mere *presence in the widget tree*, via a diagnostic environment flag that
builds the window without it. **With the whole strip absent from the tree, 2 of 5 runs still
hung.** No configuration tested has ever been hang-free.

⚠ **The original attribution was a small-sample artefact, and this is the trap to avoid on
the next one.** It rested on a parent commit going 3-for-3 green against a child going
3-for-4 red. Against a background rate that varies between 40% and 80%, **neither result
carried information** — a 3-run green streak is unremarkable when the true pass rate is
one in two, and the whole investigation that followed was chasing a difference that was
never measured to exist. Before attributing an intermittent to a change, establish the
background rate FIRST, on enough runs to tell two rates apart; a clean baseline of three is
not a baseline.

**Possibly the same defect as the macOS autorelease-pool crash recorded elsewhere in this
register** — both are macOS-only, both fire at a varying site, and both land in a similar
fraction of runs. One kills the process and one stops it, which is a real difference, but
the register's own warning about one defect filed twice applies: check them together before
treating either as understood.

**Mitigation options**:

- **Establish the background rate properly** — a run of ten on an untouched tree, which is
  the measurement every arm so far has been missing.
- **Capture a stack from a hung run** rather than recording where the output stopped; the
  site is the one thing that has moved every time and it is being read as a clue.
- **Accept slower macOS ratification** in the meantime: read a macOS result only from
  several runs, never from one, and never treat a hang as a verdict about the change.

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

