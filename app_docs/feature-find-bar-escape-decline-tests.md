# Test coverage for the find bar's Escape-decline mitigation

## Overview

Issue #5 reported that Escape stopped closing the find bar, permanently, once, in a
single unreproduced `mac`-seat run (tracked as `sdd/ISSUES.md` entry **W**, still open,
still High severity). Before this work, the shipped mitigation for that wedge's blast
radius — the window-level bubble-phase Escape handler in `wire_find_bar` declining
rather than swallowing Escape when the find bar fails to close — had **no automated test
coverage at all**. This change does not fix or diagnose the wedge. It makes the existing
mitigation's decision logic testable and adds tests for it.

**What this is:** a behavior-preserving refactor plus new tests around already-shipped
code.
**What this is not:** a fix for the Escape wedge, a root-cause diagnosis, or a change to
any observable behavior.

## What Was Built

1. **Extracted the decision logic into a named, pure function.** The bubble-phase
   Escape handler in `wire_find_bar` used to decide inline, after calling
   `close_find_bar()`, whether to claim the key (`Propagation::Stop`) or let it pass
   through (`Propagation::Proceed`). That inline `if` is now
   `findbar::decide_after_close_attempt(reveals_child, is_child_revealed)`, a plain
   function over two booleans with no GTK dependency. The closure is now a thin
   wrapper: it reads the two revealer properties after calling `close_find_bar()`,
   calls the function, and logs the same `WARN` on the same decline arm, exactly as
   before.
2. **Pinned the function's full truth table with unit tests.** Four `#[test]` cases in
   `findbar.rs` cover all four `(reveals_child, is_child_revealed)` combinations, each
   naming the specific hazard it guards against (fully closed / target still open /
   drawn state still open — animation or desync / both still open).
3. **Added a live-window regression test for the mitigation's happy path.** A new
   `#[gtktest::test]` in `bartests.rs` opens the find bar, closes it, confirms a second
   Escape-bound surface (the history popover) still opens and closes normally
   afterward, and confirms the bar can be reopened — guarding against a regression to
   the pre-hardening "Escape dies forever" shape.
4. **Updated the citation in `sdd/ISSUES.md` entry W** to name
   `findbar::decide_after_close_attempt` directly, in place of prose-only description.
   The diagnosis, "Not reproduced" section, and High severity are all unchanged, and
   the entry remains in the OPEN table.

## Technical Implementation

### Files modified

- **`src/window/findbar.rs`**
  - Added `pub(super) fn decide_after_close_attempt(reveals_child: bool, is_child_revealed: bool) -> glib::Propagation`,
    extracted from the inline check in the bubble-phase `connect_key_pressed` closure
    inside `wire_find_bar`. Returns `Propagation::Stop` only when both properties agree
    the bar is closed; `Propagation::Proceed` in every other combination.
  - The closure itself is unchanged in observable behavior: it still calls
    `close_find_bar()` first, still reads `fr.reveals_child()` and
    `fr.is_child_revealed()` afterward, still logs the same `log::warn!` message on the
    decline arm, sited at the same call site (logging deliberately stayed with the GTK
    call site, not inside the pure function).
  - Added `#[cfg(test)] mod tests` (end of file, after `clippy::items_after_test_module`)
    with four cases:
    - `a_bar_that_is_fully_closed_yields_stop` — `(false, false)` → `Stop`.
    - `the_target_still_says_open_after_close_was_called` — `(true, false)` →
      `Proceed`.
    - `the_drawn_state_still_shows_it_after_the_target_flipped` — `(false, true)` →
      `Proceed`.
    - `both_properties_still_say_open` — `(true, true)` → `Proceed`.
  - These are plain `#[test]`s (no `gtktest::test`, no GTK init needed) because the
    function takes only booleans — this is the codebase's established pattern for a
    key-handling decision with no synthetic-event path, precedented by
    `src/codeview/navkeys.rs`'s `redirect_navigation_key`.

- **`src/window/find/bartests.rs`**
  - Added `#[gtktest::test] fn escape_closes_the_bar_and_a_later_history_popover_still_works()`.
  - Its doc comment states plainly what it can and cannot exercise: a real
    `GtkRevealer`'s `set_reveal_child(false)` updates `reveals_child()` synchronously,
    so **the decline arm's precondition (the bar failing to close) cannot be
    reproduced through a live widget at all** — the four `decide_after_close_attempt`
    unit tests in `findbar.rs` are the sole coverage of that arm, not a supplement to
    a GTK-driven one. Cross-widget bubble-phase routing (whether a different consumer
    takes Escape first) also has no synthetic-key-event path anywhere in this test
    suite; that half is covered only by the manual script at
    `tests/MANUAL-TEST.md` §11.5a.
  - What the test does exercise live: opening the bar via `win.find`, closing it via
    the same `close_find_bar` path the Escape handler uses (driven through
    `find_entry`'s own `stop-search` signal), confirming the revealer is genuinely no
    longer revealed, confirming a second Escape-bound surface (the history
    `GtkMenuButton` popover) still opens and closes normally with the bar down, and
    confirming the bar can be reopened afterward without being left in a
    once-only state.

- **`sdd/ISSUES.md`**
  - Entry **W** ("Escape stopped closing the find bar, permanently, once") now cites
    `findbar::decide_after_close_attempt` by name in its "What the hardening does and
    does not do" paragraph, in place of describing the decision only in prose.
  - No other change: diagnosis, "Not reproduced" evidence, severity (High), and open
    status are all exactly as before this work.

### No dependencies added

This work introduced no new crates or external dependencies — only new `#[test]` /
`#[gtktest::test]` cases and a mechanical extraction of existing logic.

## Usage

This is internal test coverage, not a user-facing feature — there is nothing to
configure or invoke directly. For anyone verifying or extending this coverage:

```bash
# Unit tests, including the four decide_after_close_attempt truth-table cases
cargo test

# GTK integration tests, including the new happy-path regression test
cargo test --features gtk-integration-tests

# Lint and reference hygiene (the ISSUES.md citation, etc.)
cargo clippy --all-targets -- -D warnings
cargo xtask lint-references
```

## Configuration

None. No feature flags, environment variables, or config keys were introduced.

## What remains open

`sdd/ISSUES.md` entry **W** is unchanged in substance and remains an **open, High
severity, unreproduced** defect: a one-time report of Escape permanently ceasing to
close the find bar (and everything else it would otherwise reach) for the rest of a
process, seen once on the `mac` seat and not reproduced across three isolated legs and
three full compound passes (one faithful replay, two more against the hardened build).
The work described here does not change that status — it adds test coverage for the
mitigation that already bounds the blast radius (decline rather than swallow Escape
when the bar does not visibly close), so that mitigation's own correctness is now
pinned by tests rather than resting on inspection alone. If the wedge recurs, entry W's
"Where to look on the next sighting" section (wall-clock timing and AX-bus traffic as
the two things that differed between the original hand-paced run and the scripted
replays) is still the active lead, not anything introduced by this change.
