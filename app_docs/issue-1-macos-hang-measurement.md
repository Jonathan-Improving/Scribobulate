# Issue #1 — macOS integration-suite hang/crash: measurement round (2026-09-22)

**Outcome: a narrowing, not a fix.** This closes issue #1 with new measurements against
`sdd/ISSUES.md` entries **X** (the intermittent macOS integration-suite hang) and **F**
(the GTK4/Quartz autorelease-pool crash), per the spec's own explicit definition of a
complete PR (`.flowdra/specs/issue-1-macos-integration-hang-measurement.md` §0). Entry X
stays open, High severity, unmoved to Closed. No code change is claimed to fix, mitigate,
or reduce either issue's rate.

## Why this round happened

Both entries are macOS-only, intermittent, fire at a varying test site, and need full
suite depth rather than any one test body — the register's own header warns one defect
can be filed twice from two vantage points, and both spec and entry X explicitly flag
"possibly the same defect family" as unresolved. A prior attribution attempt on entry X
was already retracted once as a small-sample artefact (3-for-3 green vs. 3-for-4 red
against a 40–80% swinging background rate). This round exists to replace guesswork with a
properly sized sample and to check the two entries together rather than in isolation.

## What was done

- **Line A — static analysis** (hypothesis only, no macOS execution needed): every
  reachable `MainContext::iteration()`/`block_on()` call site in `src/` was audited for the
  re-entrancy shape entry X's signature would need. All are issued from test-body top
  level, none from inside a signal-handler closure already on a live GDK dispatch frame.
  One production-path finding: `src/window/mod.rs:1354`'s `block_on` also drives
  `restore_session` at startup, not just tests. Full document:
  `scratch/issue-1-hang-measurement/line-a-static-analysis.md`.
- **Line B — breadcrumb instrumentation**, now live for future runs: `src/gtk_suite.rs`'s
  `arm_timeout` deposits a breadcrumb (case name, timestamp, "timeout not crash" marker)
  into `src/forensics/mod.rs`'s `BREADCRUMBS` ring *before* arming the alarm. `on_alarm`
  itself is byte-for-byte unchanged — still only async-signal-safe writes and `_exit(124)`.
  Routing through the richer `forensics::signal.rs` fatal path was considered and rejected
  for this call site (verified: `gtk_suite`'s `main()` never arms that handler). Tests added
  in `src/gtk_suite.rs`; `src/logrepeat.rs`'s existing collapse mechanism was verified,
  not changed, against the alternating prepare/check shape.
- **Line C — sampling campaign**, real macOS hardware (Darwin 25.6.0 arm64), CI unavailable
  for a structural reason (fork Actions never enabled — confirmed via `gh api`, five
  throwaway branches, zero registered runs). 22 baseline runs + 6 instrumented runs = 28
  total. A genuine contamination class was found and excluded: runs 2–7 (6/6) failed
  identically on two focus/frame-clock-dependent tests because Firefox held OS-level
  frontmost focus throughout — diagnosed live via `osascript`, not this entry's signature,
  excluded from the rate rather than silently dropped. 14 clean-signature runs remain for
  the rate calculation.
- **Line D — the `sdd/ISSUES.md` update itself**: append-only "Re-measured 2026-09-22"
  sections on both entry X and entry F, preserving all existing history verbatim.

## Headline result

- **Entry X**: this entry's own signature (`poll(2)` EAGAIN ×3, then recursive
  `g_main_context_prepare()`/`check()` spam) occurred **zero times in 28 runs** (22
  baseline + 6 instrumented). This is **consistent with, not narrower than**, the entry's
  existing 20–60% band — a 20% true rate shows zero hits in 14 draws ~4% of the time, a
  60% rate shows it ~0.001% of the time, so the null result sits more comfortably against
  the band's lower half but does not narrow the band's width. No stack was captured (there
  was nothing to capture).
- **Entry F**: its signature (autorelease-pool page corruption) fired **once in 14** clean
  runs (≈7%), at a genuinely new site (`window::editor_annotate::...
  the_prepopulated_comment_opens_unselected_with_the_caret_at_the_end` — checked
  mechanically against every previously recorded site, no overlap). 1/14 is **not
  distinguishable from, and not narrower than**, the entry's existing ~25–46% band.
- **Relationship between X and F**: the one entry-F abort this round fired during a
  baseline run being sampled for entry X's signature, and entry X's signature did not
  appear anywhere in that same log — one hedged, co-occurrence-free data point, weak
  evidence against a shared failure mode *in that instance*, and explicitly not treated as
  resolving whether the two share a root cause in general.

## Where the full detail lives

- `sdd/ISSUES.md` entry **X** and entry **F**, both 2026-09-22 "Re-measured" sections —
  authoritative for every number, citation, and hedge above.
- `scratch/issue-1-hang-measurement/` — `run-log.md` (per-run ledger, all 28 rows),
  `line-a-static-analysis.md` (full Line A findings), `audit-trail.md` (cross-references
  from the new ISSUES.md text back to the exact existing sentences it bears on),
  `raw/` (per-run log excerpts for every non-clean run).
- `.flowdra/artifacts/validator-1.md` — independent verification of every claim above
  against primary evidence (raw logs, source citations, live command re-runs on real
  macOS hardware).
