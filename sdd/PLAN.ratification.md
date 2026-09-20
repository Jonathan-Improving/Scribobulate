# Plan: Ratifying the QA mitigation branch

## Problem

Thirteen commits of QA round-3 and round-4 mitigation work sit on the branch
`mitigations`, ahead of `master` at *"Merge branch 'theming/candy'"* (2026-09-19).
Every one of them passed `scripts/pipeline.sh` on Linux before it landed. None has been
run on macOS or Windows, and roughly half has not been read by a reviewer.

That matters more than usual here, because the range includes:

- a **security fix on the export path** whose payload set was settled by execution rather
  than argument, and whose gate is a positive allowlist — the failure mode of an
  allowlist is refusing something legitimate, which only a real document shows;
- a **PowerShell integrity check** written on Linux by an agent that cannot execute
  PowerShell;
- a **cold-start latch** that changed application startup on every platform, where the
  macOS single-instance substitute takes a different route into the same handlers;
- **PDF geometry corrections** at four sites, verified against a rendered surface on one
  platform only.

Merging unratified into `master` would put the project back in the position QA named at
the close of round 2: mitigations attested as complete, never verified, and later found
to have closed a symptom by a forbidden method while carrying a second defect untouched
through the refactor meant to fix it.

### Root cause

Not a defect — a sequencing gap. POLICY § Manual integration testing requires that a
change touching the filesystem, IPC, packaging or paths is ratified only once the macOS
and Windows seats have run it. The Windows seat was absent for the entire mitigation
session and the macOS seat, though present, did not respond to its brief. The work
proceeded because the alternative was to stop, and the branch exists precisely so that
`master` never carried unverified work while it did.

## What is already established

Stated so no one re-derives it:

- **Linux**: full pipeline green on every commit, including the integration suite, the
  per-render memory-growth step, the coverage ratchet and all twenty-two
  cross-reference checks.
- **Reviewed by QA**: the first six commits, through *"Make the registers say what is
  true, and gate the rule that had none"* (2026-09-19).
- **Attested, not reviewed**: everything after that. QA stood down after round 4 at the
  operator's direction and recorded these as attested-behind-a-green-pipeline, which is
  a real claim and a weaker one than verified.
- **Mutation-tested individually**: every new gate and guard in the range was proved to
  fail when the thing it protects was broken. Three of those tests initially passed with
  the protected code deleted and were rewritten; that is the specific reason the
  attested-not-reviewed distinction is being taken seriously rather than waved through.

## Possible approaches

### ✅ 1. Windows first, then macOS, then merge

Follow the project's existing seat order. The Windows seat runs
`packaging\windows\pipeline.ps1` and its four owned items; the macOS seat then runs
`packaging/macos/pipeline.sh` and its two, plus the new manual checks; `mitigations`
merges once both report.

Ordering is not arbitrary. Three of the four Windows items are on the path that produces
the artefact a user installs, and one of them is a script no other seat can execute at
all — so a macOS pass would say nothing about whether the branch is deliverable.

### ❌ 2. Merge now, ratify on `master`

Rejected. It converts a reversible branch decision into an irreversible one, and the
only thing it buys is that `master` moves sooner. The branch already keeps the work
safe, reviewable and mergeable; there is nothing to gain by giving that up before the
evidence arrives.

### ❌ 3. Re-open QA for the unreviewed commits before any seat runs

Rejected as the *first* step, not as an idea. A reviewer reads code; a seat runs it on
an operating system the code has never touched. The unverified risks in this range are
overwhelmingly execution risks — unparsed PowerShell, an untested startup path, page
geometry — and review cannot reach any of them. Worth doing after the seats report, if
the operator wants the attested half closed properly.

### 💡 4. Ratify in two halves, splitting the branch

Merge the QA-reviewed first six commits now and hold the rest. Attractive if the Windows
seat stays unavailable for a long period, since it lets the security and startup fixes
land on `master` sooner.

Against it: the two halves are not independent. The export allowlist was tightened twice
across the split, the cold-start latch gained its test-suite fix in the second half, and
the commit-hash lint's own defects were fixed there too — so the first six alone contain
a lint that reports success over violations it was written for. Splitting would deliver a
known-worse state than either the whole branch or none of it.

Keep as a contingency, not a plan.

## Recommendation

Approach 1.

**Blocking**: the Windows seat is not present. Nothing below can start until it is, and
that is the single decision the operator holds — no amount of Linux work substitutes for
it.

**Order of operations:**

1. Wake the Windows seat. Brief it on its four items (below) and have it run the port's
   own pipeline against the branch.
2. Windows returns a verdict plus any `windows/<feature>` branch; the Linux seat
   integrates and deletes the seat branch.
3. Repeat for macOS with its two items and its three new manual checks.
4. Operator merges `mitigations` into `master`.
5. Optionally re-open QA for the attested half, now that the code has been executed on
   three platforms.

## Per-seat obligations

### Windows

The seat has four items. Three sit on the delivery path:

| Item | Why only this seat can answer it |
|---|---|
| The prebuilt GTK stack's digest check in `.github/workflows/pipeline.yml` | The PowerShell was written on Linux and has never been parsed by PowerShell. A syntax error fails the job; a logic error passes it while verifying nothing. |
| A cache **hit** restores an already-unpacked prefix that is never re-verified | The digest is in the cache key, which is a claim rather than a check. Raised by QA in round 4 and deliberately out of scope for the fix that closed the fetch path. |
| `winget install JRSoftware.InnoSetup` is unpinned | The tool that builds the installer handed to users is fetched at an unfixed version. |
| The PowerShell pipeline's contract self-test plants 4 violations against 20 rules | Ten refusals have never been proven reachable, in the one language of the three where a mistyped variable yields a silently passing branch. |

Plus the ordinary port run and the new manual checks, which are listed in
`tests/MANUAL-TEST.md` under §8.2a, §8.2b, §25.4a and §25.9a.

### macOS

| Item | Why only this seat can answer it |
|---|---|
| Pipeline step 5 hand-enumerates its `[[test]]` targets on macOS while the other two ports select all of them | No gap exists today — the enumeration names all five declared targets — but it already drifted once and was caught by a human. A target added later is silently absent there while the step prints PASS. |
| `probes/capture-weld.sh` discards `lldb`'s exit status and its Python block's failure | A failed capture prints "written to…" and exits 0. It also picks one process arbitrarily while investigating a two-window fault. |

Plus §8.2a, §8.2b, §25.4a and §25.9a, and confirmation that the animations guard's new
reduced-motion assertion stays inert there — macOS has no reduced-motion source wired, so
if it fires, that is a finding rather than a host setting.

### Both seats

The manual checks are worth naming individually because each has a way of passing
vacuously:

- **§8.2a** needs a session large enough that a second launch lands *inside* the restore.
  A fast restore passes on the broken build.
- **§25.4a** must be clicked in a browser, not grepped. The escaper rewrites five
  characters and none of the payloads contain them.
- **§25.9a** must measure one decoration against the *page*. The unit error was uniform,
  so every decoration agreed with every other and only the page against the screen showed
  it.

## Proposed TDD Rubrics

**None.** This plan introduces no behaviour; it schedules verification of behaviour that
already has contracts. The rubrics this work is ratified against are already in
`sdd/TDD.md` — §8.2a, §8.2b, §25.4a, §18.60 and §27.8 among them — and their manual
counterparts already exist in `tests/MANUAL-TEST.md`. Writing new ones here would
duplicate a contract rather than add one, which is the failure mode POLICY's
one-owner rule exists to prevent.

Stated explicitly because an empty section reads like an oversight.

## Technical details preserved

- **A cache key is not an integrity check.** The GTK archive's digest is verified before
  unpacking on a cache *miss*. On a *hit*, the prefix is restored already unpacked and
  nothing re-checks it; the digest's presence in the key claims the content matches
  without testing that it does.
- **The cold-start claim is process-wide and deliberately unresettable.** Test code uses
  `app::coldstart::force_for_test(bool)` and must state which launch it models — a bare
  reset makes every launching test a cold start, which runs crash recovery that tests
  building a window first never expected. That was measured: it reddened two passing
  tests.
- **`platform/mac/single_instance.rs` drives `activate`/`open` directly** and sorts
  earlier than most of the suite, so it reaches the cold-start latch before the tests
  that depend on it. It already carries the guard; a seat seeing an ordering failure
  there should suspect a missing guard elsewhere rather than the latch.
- **The export gate is an allowlist, not a blacklist.** A destination is admitted only if
  it positively looks like a relative reference. The risk it carries is the opposite of
  the defect it fixed: a legitimate link refused. Real documents with unusual but valid
  relative destinations are the test, and no fixture can stand in for them.
