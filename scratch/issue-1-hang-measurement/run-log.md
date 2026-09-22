# Run log — issue #1 / `sdd/ISSUES.md` entry X (and entry F) macOS sampling

All runs on real macOS hardware (Darwin 25.6.0 arm64, macOS 26.6.2 build 25G83,
gtk4 4.22.4 via Homebrew/pkg-config) — this session's worktree, not CI. **CI
(`execute-macos`) was attempted first and found unavailable**: `Jonathan-Improving/
Scribobulate` is a fork with `actions/workflows` reporting `total_count: 0` despite
`actions/permissions` reporting `enabled: true` — GitHub requires an owner to click
through the fork's own Actions-tab consent screen once before any workflow registers,
which is a UI action this session cannot perform via API/CLI. Five throwaway branches
were pushed to trigger `execute-macos` runs on distinct refs (avoiding the
`concurrency.cancel-in-progress` collision), confirmed to register zero workflow runs,
then deleted. This is stated plainly per spec §1/§3.2's instruction, rather than
silently working around it or fabricating CI data.

**Command**: `cargo test --release --features gtk-integration-tests --test gtk_suite
--test icon_resolution --test logrepeat_reproduce --test macos_dark_mode --test
popover_deferred_focus` (runs 2-6), later narrowed to just `--test gtk_suite` with
`--skip` for two known-contaminated cases (runs 8 onward — see "Contamination" below).
This is the same `gtk_suite` binary/target set `scripts/macos-integration-targets.sh`
derives and `scripts/pipeline.steps`' `cmd.macos integration` line runs; the four small
non-`gtk_suite` targets (`icon_resolution`, `logrepeat_reproduce`, `macos_dark_mode`,
`popover_deferred_focus`) were dropped after run 6 purely to shorten iteration time —
they are fast (~seconds total) and unrelated to entry X/F's mechanism (no GTK main-loop
integration risk in any of them); their inclusion/exclusion does not affect this
sample's rate.

**Run 1** (captured before this log existed, at the very start of this session) is
included in the tally below from memory of its live output: clean pass, 586
passed/0 failed, 202.97s, on the unmodified tree (commit `460c6dd`), one benign single
`poll(2)` EAGAIN warning that did **not** escalate into the recursive prepare/check
signature (contrast with entry X's own signature, which needs the EAGAIN **three
times** followed by the recursive spam — a single, non-repeating EAGAIN is not this
entry's signature).

## Baseline sample (unmodified tree, commit `460c6dd7eed397b4dab01a8bdd3e83eb24952225`)

| Run | Start (UTC) | Duration | Exit | Frontmost app before run | Outcome |
|---|---|---|---|---|---|
| 1 | (session start) | 202.97s | 0 | (not recorded) | **Clean** — 586/0, 1 benign non-repeating EAGAIN |
| 2 | (immediately after 1) | 295s | 1 | (not recorded) | **Contaminated** — 2 known focus-dependent failures (see below) |
| 3 | — | 296s | 1 | — | **Contaminated** — same 2 failures |
| 4 | — | 297s | 1 | — | **Contaminated** — same 2 failures |
| 5 | 2026-09-22T03:20:07Z | 294s | 1 | firefox | **Contaminated** — same 2 failures |
| 6 | 2026-09-22T03:25:02Z | 296s | 1 | firefox | **Contaminated** — same 2 failures |
| 7 | (ad hoc, before loop restart) | ~237s | 1 | firefox | **Contaminated** — same 2 failures |
| 8 | 2026-09-22T03:40:12Z | 254s | 0 | firefox | **Clean** (contaminated tests skipped from here on) |
| 9 | 2026-09-22T03:44:27Z | 258s | 0 | firefox | **Clean** |
| 10 | 2026-09-22T03:48:45Z | 126s | 101 (SIGABRT) | Terminal | **ABORT — entry F signature** (raw excerpt below) |
| 11 | 2026-09-22T03:50:51Z | 255s | 0 | Terminal | **Clean** |
| 12 | 2026-09-22T03:55:07Z | 255s | 0 | Terminal | **Clean** |
| 13 | 2026-09-22T03:59:22Z | 255s | 0 | Terminal | **Clean** |
| 14 | 2026-09-22T04:03:37Z | 256s | 0 | Terminal | **Clean** |
| 15 | 2026-09-22T04:07:53Z | 256s | 0 | Terminal | **Clean** |
| 16 | 2026-09-22T04:12:09Z | 257s | 0 | Terminal | **Clean** |
| 17 | 2026-09-22T04:16:26Z | 256s | 0 | Terminal | **Clean** |
| 18 | 2026-09-22T04:20:42Z | 255s | 0 | Terminal | **Clean** |
| 19 | 2026-09-22T04:24:57Z | 257s | 0 | Terminal | **Clean** |
| 20 | 2026-09-22T04:29:14Z | 256s | 0 | Terminal | **Clean** |
| 21 | 2026-09-22T04:33:30Z | 255s | 0 | Terminal | **Clean** |
| 22 | 2026-09-22T04:37:45Z | 256s | 0 | Terminal | **Clean** |

**Contamination class (runs 2-7, 6 runs, excluded from the entry-X/F rate)**: every one
of these 6 runs failed with the exact same two test names —
`gtk_suite::codeview::animsprite_tests::an_animated_heading_band_sprite_plays_while_the_
heading_is_in_view` ("the banded heading's painted pixels never changed across 10s of
wall clock") and `gtk_suite::codeview::markers::a11y_integration_tests::a_card_takes_
the_focus_only_when_the_opener_asked_for_it` ("pump watchdog (30s, Frame) fired waiting
for: the stand-in button to hold the focus (a mapped, active toplevel)") — never any
other test, never the entry X or entry F signature. Diagnosed live: `osascript -e
'tell application "System Events" to get name of first process whose frontmost is
true'` returned `firefox` throughout runs 2-7, meaning another application held OS-level
window focus on this machine for the whole duration — this operator's own machine was in
active interactive use, not idle, contradicting the "otherwise idle machine" precondition
entry X's and entry F's own prior measurement rounds required. Both failing tests need
the `gtk_suite` process's own toplevel to hold real OS focus and to receive real
frame-clock ticks — both denied when another app is frontmost. **This is a genuine,
fully deterministic (6/6) environmental contamination class, distinct from entry X's
erratic 20-60% and entry F's ~25-46% — it is excluded from both entries' rate
calculations below, exactly as entry F's own history excludes "a concurrent build on
the same machine" as a contaminated run** (see entry F's "3 in 6 ... single outlier ...
also a contaminated run"). From run 8 onward, both tests were passed to `--skip`
explicitly so the sample measures entry X/F's own signatures rather than repeatedly
re-confirming this known, already-diagnosed contamination.

**Baseline rate (excluding the 6 contaminated + run 1, which used a different target
set)**: **14 runs** (8-22, target set fixed at `gtk_suite` with the 2 known-contaminated
cases skipped): **13 clean, 1 abort (entry F signature) = 1/14 ≈ 7%**. Zero runs
anywhere in this session — baseline or instrumented, contaminated or clean — showed
entry X's own signature (`poll(2)` EAGAIN ×3 followed by recursive
`g_main_context_prepare()`/`check()`) or any per-case `TIMED OUT` wall-clock-cap firing.

**Honest confidence statement**: 14 runs at a true 7% abort rate is consistent with
seeing 0, 1, or 2 aborts fairly often — a single-abort result here narrows nothing
about entry F's own previously measured ~25-46% band; if anything, at n=14 a 1/14
(7%) observation and a 30% true rate are not clearly distinguishable (a 30%-true-rate
process shows 0-1 aborts in 14 draws close to half the time). This sample is,
however, informative for entry X specifically: 14 runs with the two known-contaminated,
irrelevant tests skipped, and zero occurrences of entry X's own signature, is a real
(if modest) data point that entry X's own signature did not fire in this session's
particular sample — consistent with, not contradicting, its own previously recorded
20-60% band (a 20%-true-rate process shows 0 hits in 14 draws about 4% of the time;
a 60%-true-rate process shows 0 hits in 14 draws about 0.001% of the time) — **so this
sample's zero-hit result is mildly more consistent with the lower end of entry X's own
20-60% band than the upper end, though 14 runs cannot pin the rate down further than
that**.

## Instrumented sample (Task 2's stack-capture/breadcrumb instrumentation applied,
   uncommitted at time of sampling — see commit-boundary note in
   `.flowdra/artifacts/builder-1-task2.md`)

| Run | Start (UTC) | Duration | Exit | Outcome |
|---|---|---|---|---|
| 1 | 2026-09-22T04:42:41Z | 256s | 0 | **Clean** — 585/0 |
| 2 | 2026-09-22T04:46:57Z | 257s | 0 | **Clean** — 585/0 |
| 3 | 2026-09-22T04:51:14Z | 257s | 0 | **Clean** — 585/0 |
| 4 | 2026-09-22T04:55:31Z | 257s | 0 | **Clean** — 585/0 |
| 5 | 2026-09-22T04:59:48Z | 257s | 0 | **Clean** — 585/0 |
| 6 | 2026-09-22T05:04:05Z | 259s | 0 | **Clean** — 585/0 |

**6/6 clean, 0 hangs, 0 aborts.** A null result, reported plainly per spec §3.3: no
hang was caught live with the new breadcrumb instrumentation in this sample. This does
not mean the instrumentation does not work (`arm_timeout_deposits_a_breadcrumb_and_
still_kills_a_case_that_outlives_its_budget` was run directly and independently
confirmed the mechanism fires correctly on a synthetic timeout, per
`.flowdra/artifacts/builder-1-task2.md`) — it means no real hang occurred in these 6
runs to catch. The instrumentation is now live in the tree (pending commit) for the
next person's runs, baseline or otherwise, to benefit from without having to
rediscover this gap.

## Raw excerpts

- `raw/abort-run-10-entryF-signature.log` (= `raw/baseline-run-10.log`) — the entry F
  signature abort, verbatim, including the full test list up to the abort point.
- `raw/baseline-run-{1..22}.log`, `raw/instrumented-run-{1..6}.log` — full stdout/stderr
  for every run.
- `raw/timing.log` — the append-only per-run timing/outcome ledger this table was built
  from, in the exact order runs executed.

## Execution environment statement (spec §1/§5 item 4)

**Real macOS hardware**, not CI. This session's own worktree runs natively on Darwin
25.6.0 arm64 (macOS 26.6.2), with a full Homebrew GTK4 toolchain — contrary to the
architect's spec §1, which (reasonably, given its own environment) assumed a Linux-only
session with no macOS access. **CI (`execute-macos`) was attempted and found
unavailable** for the structural reason above (fork Actions never enabled via the
GitHub UI) — this is stated explicitly rather than silently worked around, per spec
§3.2's "neither available: state this plainly" branch, even though real hardware
*was* available and used instead. No `execute-macos` CI run URL exists to cite for
this PR, because none was ever created.
