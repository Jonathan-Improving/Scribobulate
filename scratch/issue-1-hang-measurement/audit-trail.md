# Audit trail — Task 4 (`sdd/ISSUES.md` entry X narrowing update)

Issue #1, spec `.flowdra/specs/issue-1-macos-integration-hang-measurement.md` §4/§5/§8
Task 4. Companion cross-reference document per spec §5 item 3: locates the exact existing
entry X (and entry F) sentences the 2026-09-22 measurement confirms or narrows, so a
validator does not have to diff the whole entry to find what changed in substance.

## What was edited

`sdd/ISSUES.md`:
- Entry **X** ("The macOS integration suite hangs intermittently, at a varying site") gained
  one appended section, heading:
  **`**Re-measured 2026-09-22, on real macOS hardware, by this session's builder seat.**`**
  — placed immediately after the entry's existing Mitigation options list (which was left
  intact, with a one-line "superseded in part" pointer added to it rather than deleted, per
  the append-only rule), and before the CLSD-04 entry that follows X in the file.
- Entry **F** ("A GTK4/Quartz autorelease-pool crash intermittently SIGABRTs the macOS
  integration suite") gained one appended section, heading:
  **`**Re-measured 2026-09-22, on real macOS hardware, by this session's builder seat, as a
  byproduct of a baseline sample taken FOR entry X...**`**
  — placed after the entry's existing "Measured on macOS 26.6.1..." closing paragraph and
  Mitigation options, before entry I.
- Nothing else in `sdd/ISSUES.md` was touched. No history was deleted or rewritten in either
  entry. Entry X was **not** moved to Closed issues and was **not** marked resolved — it
  remains an open, High-severity row in the first table; this is a narrowing, not a fix.

## Which existing sentences this measurement bears on

**Entry X, existing text, confirmed/extended by this round:**

- *"Background rate across runs is erratic, roughly 20–60%..."* — this round's 14 (baseline)
  + 6 (instrumented) clean-on-signature runs, zero hits, is **consistent with** this band
  (mildly more consistent with its lower half per the binomial sanity check in the new
  section), **not** a narrowing of its width. Stated explicitly in the new section rather
  than left to be inferred.
- *"It has a measured signature: poll(2) failed... three times, then
  g_main_context_prepare()/g_main_context_check()... 14,877,756 repetitions..."* — this
  round adds a **null result**: the signature did not appear at all in 22 runs. Read
  together with the existing sentence, this is new evidence about the signature's own
  frequency (not about whether it exists — the 2026-09-19 round already captured it once at
  full volume), reported honestly as a null result rather than silently omitted.
- *"A prior attribution attempt was RETRACTED as a small-sample artefact..."* — this round's
  own framing ("14 runs cannot pin the true rate down any further than that... exactly the
  sample size this entry's own retraction lesson warns against over-reading") explicitly
  applies that same discipline to itself, rather than treating a clean streak as a result.
- *"Possibly the same defect family as entry F... check them together before either is
  treated as understood."* — directly answered in the new section's dedicated paragraph:
  this round's one entry-F abort fired during a run with **no** entry-X signature present,
  offered as one hedged, co-occurrence-free data point, explicitly not treated as resolving
  the question.
- **Mitigation options list** — the existing three items are carried forward verbatim in the
  new section's own updated list (marked as "still unmet" or "unchanged" as appropriate);
  two new items are added (idle-machine verification; a follow-up sample targeting
  `window::editor_annotate`).

**Entry F, existing text, confirmed/extended by this round:**

- *"Same rough rate class (measured at various points as ~25–46%..."* — this round's 1/14 ≈
  7% is stated explicitly as **not distinguishable from, and not narrower than** that band
  at this sample size — matching the entry's own established honesty convention (its
  2026-09-14 section models exactly this same non-claim: "consistent with the existing rate,
  NOT a measured increase").
- *"3 in 6 and 3 in 7 on the next two — 6 in 13 (46%)... An attribution to the new test was
  made and RETRACTED..."* and *"the aborting body wandered across four tests, including
  this entry's dominant site"* — this round's new site
  (`window::editor_annotate::gtk_integration_tests::the_prepopulated_comment_opens_
  unselected_with_the_caret_at_the_end`) is a **fifth** site, added to the record as
  genuinely new (checked against every previously named site in the entry's full text before
  writing the update — see below) rather than assumed new.
- *"the pthread values MATCH: the page was damaged in place on its own thread, not popped
  from a different one. That argues against a cross-thread pop and for an accumulating
  in-place imbalance..."* — this round's captured excerpt shows the same match
  (`pthread 0x1f9b3e180 should be 0x1f9b3e180`), stated in the new section as **consistent
  with**, not a departure from, this existing claim.
- *"It needs suite DEPTH, not a particular body."* — the new site is a further instance of
  "not the dominant site, not a repeat of a previously named one," consistent with this
  claim; the new section says so explicitly.

## Confirming the new abort site is genuinely new to entry F

Before writing the update, entry F's full existing text was searched for every test name
it already records, to verify `window::editor_annotate::...the_prepopulated_comment_opens_
unselected_with_the_caret_at_the_end` was not already on file under a different round:

- `select_all_stands_down_for_every_text_entry_and_recovers_for_the_editor` — named
  explicitly, dominant site, 2026-08-07/08 and 2026-08-31 rounds.
- "a find-cursor test" — named descriptively (not by full test path), 2026-08-27 round.
- "an annotation-card body" and "a find body" — named descriptively (not by full test
  path), 2026-09-14 round (the clipboard-free-test paragraph).
- Abort case indices 234, 234, 235, 16 — named by index only, 2026-08-31 round, no test
  names given.

None of these match or overlap with the 2026-09-22 site's specific test name.
`grep -n "editor_annotate\|prepopulated_comment" sdd/ISSUES.md` against the pre-edit file
returned no hits — confirmed mechanically, not just by reading, before the update was
written.

## Execution environment (spec §5 item 4)

**Real macOS hardware**, not CI, for this entire round (Task 3, whose data this update
cites). See `run-log.md` in this same directory for the full per-run ledger and the
CI-unavailability finding (fork Actions never enabled via the GitHub UI; verified via
`gh api` and five now-deleted throwaway branches). No `execute-macos` CI run URL exists for
this round because none was ever created — stated plainly in both `run-log.md` and the new
`sdd/ISSUES.md` section, per the spec's explicit "state this plainly" instruction rather
than silently working around the gap.

## Instrumentation diff (spec §5 item 2)

Task 2's change (not authored in this task; cited here for completeness per spec §4 item 6):
`src/forensics/mod.rs` (`BREADCRUMBS` widened to `pub(crate)`) and `src/gtk_suite.rs`
(`arm_timeout` deposits a breadcrumb before arming the alarm; `on_alarm` unchanged). Full
reasoning and verification: `.flowdra/artifacts/builder-1-task2.md`. Uncommitted at the time
Task 3's instrumented sample was taken (stated in both `run-log.md` and the new `sdd/
ISSUES.md` section); expected to land as its own commit per Task 2's own coordination note.

## `cargo xtask lint-references` result

Run after the `sdd/ISSUES.md` edit: 21 of 22 checks PASS; check 11 (register byte-size soft
limit) reports its pre-existing WARN, unrelated to and unchanged in cause by this edit (the
register grows by design as an append-only history — this is the expected shape of that
growth, not a new problem). Check 20 (git commit hashes cited) initially FAILed on a bare
SHA (`460c6dd7eed397b4dab01a8bdd3e83eb24952225`) written into the first draft of the new
section; fixed by citing the commit's subject ("Merge branch 'pipeline'") and date
(2026-09-21) instead, per POLICY's citation rule — re-run confirmed PASS after the fix.
