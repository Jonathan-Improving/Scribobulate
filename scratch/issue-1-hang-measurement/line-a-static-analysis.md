# Line A — static/code-level isolation of the main-loop re-entrancy risk

**Status: HYPOTHESIS, NOT MEASUREMENT.** Every finding below is a static-analysis
inference about what the source *could* do, not a confirmed contributor to entry X's
hang. This document produces no rate, no stack, and no confirmation. Per `sdd/ISSUES.md`'s
own header, four previously recorded root causes in this project were later measured
and found wrong; every claim below is written with that discipline in mind and is
explicitly a **narrowing of search space for the next investigation**, never a
diagnosis.

This is Task 1 of the plan in
`.flowdra/specs/issue-1-macos-integration-hang-measurement.md` §2.1/§8. It reads
`sdd/ISSUES.md` entry X ("The macOS integration suite hangs...") and entry F ("A
GTK4/Quartz autorelease-pool crash...") as already-established fact and does not
restate their findings as new — only cites them where relevant.

## 0. What this document is answering

Entry X's own text already names the leading code-level hypothesis for its
`g_main_context_prepare()`/`check()` "called recursively" signature: a **nested**
main-loop pump running while GDK's Quartz backend already holds a `prepare`/`check`
frame. This document's job, per spec §2.1, is to make that hypothesis concrete
against this codebase's actual source — naming every call site reachable from a
`#[gtktest::test]`/`gtk_suite` body that is *theoretically* capable of causing such a
nested pump — not to prove any one of them is the cause.

## 1. Method

1. Enumerated every call in `src/` to `glib::MainContext::iteration()` /
   `MainContext::block_on()` / `gtk::main_iteration()`-shaped constructs, and every
   blocking wait on a condition serviced by the main loop (`Child::wait`,
   `JoinHandle::join`, anything that could stall the thread GDK's Quartz backend
   drives), via:
   ```
   grep -rn "main_iteration\|MainContext::iteration\|\.iteration(\|block_on\|\.recv()\|\.recv_timeout(\|Condvar\|thread::park\|\.join()\|\.wait()" src/
   ```
2. For every hit reachable from a `#[gtktest::test]` body (the only bodies `gtk_suite`
   — the binary macOS's `execute-macos` step actually runs, per
   `scripts/macos-integration-targets.sh` — executes on the process main thread), read
   the surrounding function to classify it as either:
   - **top-level in the test body** (called directly in the test's own control flow,
     not from inside a GTK signal-handler closure that a live dispatch would already
     have on the stack), or
   - **nested inside a signal-handler/callback closure** (called from code that GDK's
     dispatch machinery itself would already be running when it fires) — this second
     shape is the one that matches entry X's "recursively from within a source's
     check()/prepare() member" signature, because it is the shape that would put a
     *second* main-loop pump on the stack underneath an *already-running* GDK
     dispatch frame.
3. Explicitly checked the four areas the spec calls out by name: `src/saferizer/`,
   `src/platform/mac*`, `src/clipboard.rs`, and native-dialog/file-chooser code
   (`src/saferizer/native_dialog.rs` and its callers in `src/app/appactions.rs`,
   `src/window/export.rs`, `src/window/save.rs`, `src/window/editbar/dialog.rs`).
4. Cross-referenced every finding against entry F's mechanism (§5 below), per spec
   §0.2/§4 item 5.

`gtk_suite`'s own module list (`src/gtk_suite.rs`'s `mod` block) is the authoritative
membership test for "reachable from a `#[gtktest::test]` body" — a module absent from
that list drops out of the macOS run silently (the file's own header names this and
`cargo xtask lint-references` check 4 gates it), so every finding below was checked
against that list, not just against `grep`'s hit in `src/`.

## 2. Finding classification: every hit found, and why none of them are the
   "nested-inside-a-live-dispatch" shape entry X's signature would need

**Headline finding: every reachable call to a blocking main-loop pump in this
codebase is issued from the top level of a test body's own control flow — never from
inside a GTK signal-handler closure that a live GDK dispatch would already have on
the stack.** That is the one structural fact this document can state with some
confidence from source alone; it is stated as a fact about *what the code looks like*,
not as a claim about what does or does not cause entry X's hang (§6 explains why that
gap cannot be closed by reading source).

### 2.1 `glib::MainContext::iteration()` call sites (test-body pumps)

All of the following are **top-level in the test body** — called directly from the
test function's own `fn`, in a loop the test itself drives, not from inside a
`connect_*` closure:

- `src/farscroll.rs:559,767,813,873,887,909` — scroll-animation settle loops in
  `#[gtktest::test]` bodies (`cold_editor`, `a_plain_write_truncates_a_scroll_animation`
  and siblings). Top-level.
- `src/app/menubar.rs:936,953,976,987` — menu-relabel settle loops in
  `live_menu_tests` (`the_format_relabel_lands_on_idle_and_not_in_the_callers_turn`,
  `a_there_and_back_relabel_in_one_turn_settles_on_what_is_shown`). Top-level.
- `src/saferizer/scrollpos.rs:131,147,156` — scroll-position guard tests. Top-level.
- `src/saferizer/file_monitor.rs:162` — `while ctx.iteration(false) {}` inside a
  `with_thread_default` closure, but that closure is the **test body's own driver**,
  not a GTK signal callback — it runs on a **private** `MainContext::new()`, not the
  default context GDK's Quartz backend integrates with. This is a real distinction:
  even if this pump *did* re-enter something, it would not be re-entering the same
  context GDK's `prepare`/`check` frame owns. Top-level, and off the default context.
- `src/window/tabs/dnd.rs:550,658` and `src/window/tabs/contextmenu.rs:584,641` —
  drain loops waiting for a *deferred idle callback* (tab-arrival resync, an async
  clipboard read) to land. Top-level: the loop is the test polling for the idle's
  side effect, not the idle itself calling back into a pump.
- `src/window/actions.rs:821,876,925` — draining a scheduled dismissal idle after a
  menu-action test. Top-level.
- `src/window/swap.rs:947` — one `iteration(false)` call in a swap-write test.
  Top-level.
- `src/window/rename.rs:320` (`iteration(true)`, blocking form) — `pump_for`'s wall-clock
  drain, called from rename test bodies. Top-level (the function is test-only
  scaffolding, not called from `src/window/rename.rs`'s production rename path).
- `src/window/editor_annotate.rs:304,314` — pump-to-mapped and settle loops around the
  annotation card. Top-level.
- `src/window/linknav.rs:282`, `src/preview/scroll.rs:590,725`, `src/preview/render.rs:680`,
  `src/preview/build.rs:1841`, `src/preview/annotate/overlay.rs:1016,1033,1165,1185`,
  `src/codeview/mod.rs:2309,2361,2378,2469`, `src/codeview/navkeys.rs:153,271`,
  `src/codeview/geometry.rs:487,566`, `src/widgets/tab/bar.rs:558` — all of the same
  shape: a `#[gtktest::test]`-local (or its private helper's) settle/drain loop, called
  from the top of the test body or a helper the test body calls directly, never from a
  live `connect_*` closure.
- `src/testpump.rs:137` (inside `pump`, the shared mechanism `testpump::until`/
  `until_for`/`until_stable`/`drain_for` all route through) — the blocking
  `ctx.iteration(true)` at the centre of the crate's own shared test-pump helper
  (module doc: "before it, ~24 hand-rolled copies... across 19 files"). Every one of
  its callers (`testpump::until`, `until_for`, `drain_for`, `until_stable`) is, in
  turn, called from a test body's top level in every site checked in this pass
  (e.g. `src/clipboard.rs`'s `pump()` wrapper, §2.4 below). **This is the single
  highest-traffic call site in the whole inventory** — nearly every `#[gtktest::test]`
  body that waits on anything routes through it, which means if a nested-pump
  re-entrancy risk exists anywhere in test infrastructure, this is the one place a
  fix or an instrumentation hook would have the most leverage. Still top-level by the
  classification in §1.2: it is the test's own wait, not a pump invoked *from inside*
  a GDK dispatch.
- `src/memgate/gtk.rs:38,41` — memory-gate settle loops (`#[cfg(test)]`,
  `memory-gates` feature). Top-level.
- `src/window/mod.rs:1350-1354`, `src/window/navhistory/traverse.rs:249`,
  `src/window/swaprecovery.rs:703,747,784,842,898,940,976,1014,1086,1124,1185,1233` —
  **not `iteration()` loops but `MainContext::default().block_on(future)`**, called
  directly at test-body top level to drive `restore_session`/`recover_after_restore`
  to completion. Same classification (top-level), but flagged separately in §2.2
  because `block_on` is a stronger form: it does not just iterate the context, it
  parks the calling thread's stack frame *inside* `MainContext::block_on`'s own
  private loop until the future resolves, which is a more complete "nested pump"
  shape than a `for` loop calling `iteration()` a bounded number of times.
- `src/animation/worker.rs:400` — `MainContext::default().block_on(fut)`, called
  directly inside a `#[gtktest::test]` body (`decode_runs_off_the_main_thread_and_returns_on_it`
  and its siblings in that `mod tests`). Same classification.
- `src/docio/mod.rs:588` — `MainContext::new().block_on(fut)` — a **private** context,
  not `default()` (module comment explains why: a fresh context is needed because the
  default one may already be thread-guarded by the harness). Same
  off-default-context caveat as `file_monitor.rs` above.

### 2.2 `block_on` specifically — the strongest re-entrancy *shape*, still only found
   at test-body top level

`MainContext::default().block_on(future)` is architecturally the closest thing in this
codebase to "pump the main loop until a condition holds, from inside a call that
looks synchronous" — it is exactly the shape entry X's signature describes in the
abstract (a call that, from the perspective of anything already on the stack beneath
it, re-enters the context). Every site found (`window/mod.rs:1354`,
`window/navhistory/traverse.rs:249`, `window/swaprecovery.rs` ×12,
`animation/worker.rs:400`) is called **directly from a `#[gtktest::test]` function's
own body**, before any GDK dispatch for that test iteration is in progress — not from
inside a `connect_*` closure, an idle callback, or any other code path a live
`prepare`/`check` frame would already have underneath it. `window/mod.rs`'s own
comment at line ~1350 states this explicitly for its site: *"Restore reads every tab's
document off the main thread, so it is a future now. `block_on` drives it on this
same default main context — iterating the loop until it completes — which is what the
running application does too, just without a synchronous caller waiting."* That
comment is itself worth flagging: it says the **production** code (`main.rs`'s own
startup path, not just the test) also drives `restore_session`/`recover_after_restore`
through `block_on` on the default context — meaning this shape is not test-only
infrastructure, it exists in the shipped binary's own startup sequence. That widens
this finding's relevance beyond "only exercised by the test suite" (see §4).

### 2.3 `src/saferizer/`, `src/platform/mac*`, native-dialog/clipboard — explicitly checked

Per spec item 2, these four areas were read in full:

- **`src/saferizer/`** (`popover_anchor.rs`, `click_activation.rs`, `scrollpos.rs`,
  `file_monitor.rs`, `native_dialog.rs`, `buffer_mark.rs`, `buffer_text.rs`,
  `qdata_key.rs`, `persistent_popover.rs`, `viewport.rs`): the only reachable
  main-loop-pump call sites are `scrollpos.rs`'s and `file_monitor.rs`'s test-body
  pumps already covered in §2.1. **No production code in this directory calls
  `iteration()`, `block_on`, or anything that blocks on the main loop.** `native_dialog.rs`
  (`NativeDialogHolder::show`) is purely event-driven: it calls `dialog.connect_response(...)`
  and `dialog.show()` and returns immediately — it installs a callback, it does not
  pump anything, and it blocks on nothing. This is a meaningful *negative* finding:
  the module the spec specifically flagged as a candidate (given entry F's TSM/pasteboard
  precedent) does not itself contain a synchronous main-loop wait. Its role, if any, in
  a re-entrancy scenario would be indirect — via what `FileChooserNative`/AppKit does
  *underneath* `.show()`, which is opaque to this codebase (§6).
- **`src/platform/mac/`** (`single_instance.rs`, `process.rs`, `fullscreen.rs`,
  `appearance.rs`, `bundle.rs`, `pointercrossing.rs`, `menubar.rs`): the only blocking
  wait found is `process.rs:64,77` (`child.wait()`, reaping a spawned child process
  for single-instance handoff) — this blocks the calling thread on a **child process**,
  not on the main loop, and is not reachable from any `#[gtktest::test]` body (this
  module is not in `gtk_suite.rs`'s module list at all — `platform::mac::process` is
  used only by the real single-instance handoff, not exercised by the integration
  suite). **No `iteration()`/`block_on` call anywhere in `src/platform/mac/`.**
  `fullscreen.rs`'s Cocoa FFI (reclassifying an `NSWindow` as auxiliary,
  per `sdd/TECH.md`) and `pointercrossing.rs`'s surface-realize hook both call into
  AppKit/GDK-Quartz internals this codebase does not source-control, so whether *they*
  pump a nested run loop cannot be answered from this repository's source at all
  (§6) — but neither one calls back into GLib's main context from Rust code in this
  tree.
- **`src/clipboard.rs`**: no `iteration()`/`block_on` call anywhere in the module
  itself. `wire_middle_click_paste`'s production path calls
  `primary.read_text_async(...)` — asynchronous, callback-based, no blocking wait.
  The module's own `#[gtktest::test]` bodies (in `mod gtk_integration_tests`) pump via
  `crate::testpump::drain_for`/`until_for`, i.e. route through the shared `testpump`
  mechanism already covered in §2.1 — top-level in the test body in every case (e.g.
  `a_same_application_paste_arrives_as_a_single_emission`,
  `a_preview_selection_pastes_into_the_editor_as_one_plain_text_emission`). No
  clipboard-adjacent call in this file blocks synchronously waiting on the main loop
  from inside a signal-handler closure.
- **Native-dialog/file-chooser code** (`src/app/appactions.rs:50-75`,
  `src/window/export.rs:97-118`, `src/window/save.rs:523-544`,
  `src/window/editbar/dialog.rs:253-286`): every call site constructs a
  `FileChooserNative`, then calls `NativeDialogHolder::show(&dialog, move |d, resp| {...})`
  and returns. All four are event-driven, matching `native_dialog.rs`'s own
  non-blocking design (above). **No production or test call in this codebase blocks
  on a native dialog's response synchronously** — every response is handled in the
  `connect_response` callback, asynchronously, on a later main-loop turn. None of
  these four call sites is exercised by any `#[gtktest::test]` body found in this
  pass (a real `FileChooserNative` cannot be driven headlessly/synthetically without
  a native OS dialog appearing, which is why entry I's own investigation used a
  separate manual/driven rig rather than the `gtk_suite` harness) — so while this area
  was checked per the spec's explicit instruction, **it is not currently reachable
  from any automated `#[gtktest::test]`/`gtk_suite` body at all**, which narrows its
  relevance to entry X specifically (a hang inside the *integration test suite*)
  even before considering mechanism.

### 2.4 The one place a wait genuinely spans real wall-clock time inside a test

Several sites above (`window/rename.rs:pump_for`, `saferizer/file_monitor.rs`,
`preview/annotate/overlay.rs`'s selection-debounce wait) combine `iteration()` with
`std::thread::sleep` or a real timer, to wait for GLib's own worker thread (inotify,
a spawned decode) or a debounce timer to fire. These are still top-level test-body
waits, not nested pumps — flagged here only because a wait that spans real wall-clock
time, on the same thread that would also need to service any pending GDK/Quartz
source, is a *slower* top-level pump than a tight `for` loop, and a slower pump held
open for longer is marginally more likely to overlap in time with *whatever* triggers
entry X's signature — a timing-coincidence hypothesis, explicitly weaker than a
structural one, and named as such rather than left implicit.

## 3. What was NOT found

- **No call in this codebase invokes `iteration()`, `block_on`, or an equivalent
  blocking wait from inside a `connect_*` closure, a `glib::idle_add`/`timeout_add`
  callback, or any other code that a live GDK dispatch (a `prepare`/`check`/`dispatch`
  callback already executing) would have on the stack beneath it.** Every hit
  classifies as "top-level in a test body" per §1's method. This is the central
  negative finding of this pass, and it means the search for an *application-owned*
  culprit for entry X's specific "prepare/check called recursively" signature — a
  Rust closure calling back into the main loop while GDK is already dispatching —
  comes up empty within the boundary of source this repository controls.
- No test-reachable code in `src/saferizer/`, `src/platform/mac/`, `src/clipboard.rs`,
  or the native-dialog call sites contains a synchronous main-loop wait at all (§2.3).

## 4. What this narrows, and what it does not

**What it narrows**: if entry X's mechanism is "an application Rust closure re-enters
the main loop while GDK's Quartz backend already holds a `prepare`/`check` frame", no
call site in this codebase's own source is a candidate for the *closure* half of that
sentence — every reachable blocking pump this pass found is a test's own top-level
driver, called before or after a GDK dispatch, not nested inside one. That is a real
narrowing: it means the next investigation does not need to keep searching this tree
for a "hidden nested pump inside a signal handler", because none was found across a
full `grep` of every candidate primitive.

**What it does NOT narrow, and this is the load-bearing caveat**: static analysis of
this repository's source cannot see what happens *inside* GTK's own C internals, GDK's
Quartz backend, or AppKit/HIToolbox — all three of which are exactly where entry F's
own mechanism (`gtk_text_view_mark_set_handler -> discard_preedit -> ...`, or the later
focus-crossing variant) already lives, entirely outside this codebase, per that
entry's own text ("No Scribobulate frame is faulting... The cause is GTK4's Quartz
backend firing `discard_preedit`... Not reachable from application code"). §2.3 already
notes that `platform/mac/fullscreen.rs` and `pointercrossing.rs`'s Cocoa FFI calls into
opaque AppKit/GDK-Quartz territory this analysis cannot see past. **A nested
`CFRunLoop` pump inside GDK's own Quartz backend integration code — which is
upstream, not this project's — remains fully consistent with everything found here**,
because this pass can only rule out an application-owned closure as the *direct*
re-entrant caller; it cannot rule in or out what GTK's/GDK's own C code does in
response to an event this application merely triggers (a focus change, a mark-set, a
`FileChooserNative::show()`, a clipboard read). Every one of those four *triggers* IS
reachable from application code and IS exercised by `gtk_suite`; what happens
underneath them, inside GTK/GDK/AppKit, is not.

## 5. Cross-reference to entry F, explicitly (spec §0.2 / §4 item 5)

**Are the same call sites Line A finds plausibly implicated in entry F's
`discard_preedit`/`CFRunLoop`/autorelease-pool mechanism too?**

Partially, and only at the level of *triggers*, not mechanism — stated explicitly
because the spec requires an answer even where it is qualified:

- Entry F's own text identifies its trigger surfaces as a `GtkTextView` **focus
  crossing** (`gtk_widget_grab_focus_self -> ... -> discard_preedit`) or a **mark-set**
  (`gtk_text_view_mark_set_handler -> discard_preedit`) — i.e., any code that moves
  focus between text entries or moves a text buffer's insertion mark/selection. This
  codebase's own `#[gtktest::test]` bodies do both constantly: `WidgetExt::grab_focus`
  calls (entry F's own 2026-09-13 remeasurement names `window::findbar::wire_find_bar`'s
  closure as the caller on its stack), `buf.select_range(...)` calls (present in
  nearly every test cited in §2.1 — `farscroll.rs`, `preview/annotate/overlay.rs`,
  `clipboard.rs`'s own tests, `window/editor_annotate.rs`), and the deliberate
  rapid-focus-switching entry F names as its dominant trigger site
  (`select_all_stands_down_for_every_text_entry_and_recovers_for_the_editor`). **So the
  same test bodies that contain the top-level main-loop pumps §2.1 found are, in many
  cases, the same bodies that also drive focus crossings and mark-sets** — not because
  the pump *causes* the crossing, but because a typical settle-and-assert test body
  does "select/focus, then pump to let it settle" as one unit. The overlap is in *which
  test bodies* are involved, not in any shared mechanism between the pump call and the
  `discard_preedit` path.
- **Whether the pump calls found here are themselves ON the `CFRunLoop`/autorelease-pool
  path entry F describes, or merely nearby in time within the same test body, CANNOT
  BE TOLD FROM STATIC ANALYSIS ALONE.** Entry F's stack is entirely inside
  libgtk/AppKit/HIToolbox/libobjc; nothing in this codebase's source shows what, if
  anything, `discard_preedit`'s nested `CFRunLoop` pump for pasteboard-promise
  resolution does with respect to GLib's main context integration, and nothing in
  this codebase's source shows whether an application-level `iteration()`/`block_on`
  call happening in the same test body (before or after a focus/mark-set event, per
  §1's top-level-only finding) could still land *during* that nested `CFRunLoop`'s
  window if GTK's Quartz backend schedules the two on the same turn. Saying "cannot
  tell from static analysis alone" is the honest answer here, not a placeholder for
  one — per spec §0.2's explicit instruction to state this rather than skip it.
- The subsystem-adjacency entry X's own text already claims (*"a nested `CFRunLoop`
  pump for pasteboard-promise resolution (F) sits next to a `poll(2)`-based
  `GMainContext` (X), and GDK's Quartz backend's run-loop integration is the thing both
  paths pass through"*) is **not contradicted or confirmed by anything found in this
  pass** — this pass adds no new evidence either way about whether X and F are the
  same defect family, because every call site it found sits entirely on the
  application side of that boundary, and the boundary itself is where the "possibly
  same defect" question actually lives.
- **Concretely, the `FileChooserNative`/native-dialog area (§2.3) is the one place
  this pass can name a plausible bridge between X's and F's territory**, on the
  strength of entry F's and entry I's own precedent (both cite pasteboard/`NSSavePanel`
  interaction pumping AppKit-side machinery) — but as already noted, no automated
  `#[gtktest::test]`/`gtk_suite` body currently drives a `FileChooserNative`, so this
  bridge, even if real, is not currently reachable from the suite entry X's hang was
  observed in. This is worth recording precisely because it is a *negative* finding
  that narrows scope: whatever is causing entry X's hangs during `gtk_suite` runs, it
  is very unlikely to be routing through the file-chooser code path, since that path
  is simply not exercised by any case in the suite.

## 6. What would move any of this from hypothesis to measurement

None of the above is proof of anything; each item names a distinct kind of evidence
that would be needed to promote it:

1. **A captured stack from an actual hung run** (Line B/Task 2's deliverable) —
   the single most direct promotion path. If a captured stack from a real hang shows
   an application-owned frame from any site in §2.1/§2.2 sitting *underneath* a GDK
   Quartz `prepare`/`check` dispatch frame, that would directly confirm (for that one
   call site, that one run) the shape this document could not find by reading source.
   Conversely, if every captured hang stack shows only GLib/GDK/AppKit frames with no
   application frame in between, that would be strong evidence *against* an
   application-owned trigger and would push entry X's mechanism fully into "upstream,
   like entry F" territory — itself a valuable, different finding.
2. **Whether a captured hang stack shows the SAME `CFRunLoop`/autorelease-pool frames
   entry F's `.ips` reports name** (per spec §0.2) — this is the direct test of §5's
   open question. Nothing here can substitute for it; it requires an actual captured
   stack, not a static read.
3. **Correlating hang occurrences with which test body was running** (already
   partially done for entry X's own history — the "site varies" finding, and its
   "may vary while the mechanism does not" caveat) against which of §2.1's pump call
   sites, if any, that body exercises, over a properly sized sample (§3.3 of the
   spec) — a single coincidence is not evidence (this project's own retracted
   3-vs-3 attribution is the standing lesson on that point), but a body that hangs
   disproportionately often, across a sample large enough to distinguish that from
   the 20-60% background rate, and that also happens to drive one of §2.2's `block_on`
   sites, would be worth a closer, targeted follow-up.
4. **Instrumenting `MainContext::block_on`'s call sites specifically** (§2.2) with a
   breadcrumb (matching Line B's proactive-ring-write design, §2.2 of the spec) that
   records "about to block_on the default context" before the call and "returned"
   after — since `block_on` is the strongest re-entrancy *shape* found and the one
   this document's static read cannot itself see inside of once GLib's own C
   implementation takes over.
5. **Building and running the two upstream-facing probes this project already has
   precedent for** (`probes/` holds prior investigations for entry F and entry I) —
   a minimal, instrumented Quartz+GLib reproduction that deliberately nests a
   `block_on` or `iteration()` call inside a `discard_preedit`-triggering focus change,
   run enough times to get a rate, would be the most direct way to test §5's open
   question experimentally rather than by inference. This is out of scope for Task 1
   itself (no macOS execution was used to produce this document — it is pure static
   reading) but is named here as the concrete next step that would need macOS
   execution (Line C, Task 3) to carry out.

## 7. Summary table (file:line, for the ISSUES.md update to cite)

| Site | Shape | Top-level or nested? | Default context? |
|---|---|---|---|
| `src/testpump.rs:137` | `ctx.iteration(true)` (shared pump core) | Top-level (in every caller checked) | Yes |
| `src/window/mod.rs:1354` | `MainContext::default().block_on(future)` | Top-level | Yes (prod. path too, per its own comment) |
| `src/window/navhistory/traverse.rs:249` | `block_on` | Top-level | Yes |
| `src/window/swaprecovery.rs:703,747,784,842,898,940,976,1014,1086,1124,1185,1233` | `block_on` (×12) | Top-level | Yes |
| `src/animation/worker.rs:400` | `block_on` | Top-level | Yes |
| `src/docio/mod.rs:588` | `MainContext::new().block_on` | Top-level | No (private context) |
| `src/saferizer/file_monitor.rs:162` | `while ctx.iteration(false) {}` | Top-level | No (private context) |
| `src/window/rename.rs:320` | `iteration(true)` in `pump_for` | Top-level | Yes |
| `src/window/tabs/dnd.rs:550,658` | `iteration(false)` drain | Top-level | Yes |
| `src/window/tabs/contextmenu.rs:584,641` | `iteration(false)` drain (incl. clipboard read) | Top-level | Yes |
| `src/app/menubar.rs:936,953,976,987` | `iteration(false)` settle | Top-level | Yes |
| `src/farscroll.rs:559,767,813,873,887,909` | `iteration(false)` settle | Top-level | Yes |
| `src/preview/annotate/overlay.rs:1016,1033,1165,1185` | `iteration(false)` settle | Top-level | Yes |
| `src/codeview/mod.rs:2309,2361,2378,2469`, `navkeys.rs:153,271`, `geometry.rs:487,566` | `iteration(false)` settle | Top-level | Yes |
| `src/saferizer/native_dialog.rs` (whole module) | Event-driven only — no pump | N/A (no blocking wait present) | N/A |
| `src/clipboard.rs` (whole module) | Event-driven (`read_text_async`) + test pumps via `testpump` | Top-level (tests) | Yes |
| `src/platform/mac/*.rs` | No `iteration()`/`block_on`; `process.rs` blocks on child process only, not on main loop, and not `gtk_suite`-reachable | N/A | N/A |
| `src/app/appactions.rs:75`, `window/export.rs:118`, `window/save.rs:544`, `window/editbar/dialog.rs:286` | `NativeDialogHolder::show` — event-driven, not reachable from any `#[gtktest::test]` found | N/A | N/A |

Every row above is a **hypothesis about capability, not about cause**. See §6 for what
would be needed to move any one of them further.
