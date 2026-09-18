# Development Policy

The operator's deliberate rules for working on this project. Lessons learned the hard
way belong in [ANTI-PATTERNS.md](ANTI-PATTERNS.md); procedures belong to the script or
test plan that performs them; architecture is described in [TECH.md](TECH.md). Do not add
a rule here on your own initiative.

## Build

- Rust 2021 toolchain with the GTK4 and GtkSourceView 5 development libraries. The
  oldest supported GTK is 4.6.
- Linux, macOS and Windows are all first-class. Each is built natively on its own
  platform — macOS cannot be cross-compiled. Platform setup is in
  `packaging/<os>/README.md`.
- Linux is the canonical platform for the gates. Never weaken a shared gate — a lint, a
  test, a coverage floor — to make another platform pass.
- Release builds are the reference for behaviour and footprint.

## Build pipeline

Run `scripts/pipeline.sh` (macOS: `packaging/macos/pipeline.sh`; Windows:
`packaging/windows/pipeline.ps1`) **after every change**, not at session end. A task is
not complete until every step passes. The steps:

1. Format check.
2. Clippy, zero warnings, with every feature enabled so gated test code is linted too.
   Fix a warning; an `#[allow]` needs its reason in a comment on the same line.
3. Release build.
4. Unit tests.
5. GTK integration tests, on a throwaway display and session bus.
5b. Per-render memory-growth tests.
6. Coverage gate — a no-regression ratchet, not a target. Floors are whole numbers, and
   one drops only in a change that raises the other. Fractional drift is noise: do not
   report it or adjust for it. When adding logic to a file excluded from coverage (GTK wiring), extract the
   decision into a pure module so it is measured.
7. Manual-test alignment — a change to user-visible behaviour updates
   `tests/MANUAL-TEST.md` in the same change, and the TDD rubric too if the contract
   itself changed.
8. Diagram alignment — a change to the architecture updates the system diagram in the
   same change.
9. Cross-reference lint — citations, document paths and numbered entries must resolve.
10. Installer artefact — the one opt-in step.

- The step list lives in one contract file that every runner and the CI workflow derive
  from. Change a step there — never in a runner, and never by naming steps in CI.
- Never skip a step. A step that does not apply on a platform is announced in the run
  output, never silently omitted.
- **A number has one owner.** A floor, limit, count or version is written in exactly one
  place — the code or script that uses it — and never restated in a document.

## Continuous integration

- CI invokes the platform runners whole and names no step, so adding a step never means
  editing the workflow.
- A packaging job verifies its artefact as a file — it exists, has a plausible size and
  carries the right version — not by the packaging tool's exit code.
- A workflow runs against the shared repository whichever branch it sits on, so its
  triggers are the operator's decision.

## Third-party attribution

- Every published artefact carries the licence notices its dependencies require:
  statically linked code on every platform, a bundled GTK runtime wherever one is
  bundled.
- The About dialog promises those notices are in the distribution — removing a staging
  step falsifies that claim.
- Which licence covers which binary is a determination we make, not something a gate
  can derive. Where it is unmade, say so rather than letting a green gate imply it.

## Artefact signing

No artefact is signed with a trusted identity yet; obtaining one is the operator's
decision. Until then the tool that builds an artefact announces the limitation on
success, and that announcement is removed in the same change that introduces signing.

## Testing

### Unit tests

- Pure logic gets unit tests. Prefer extracting a decision core and unit-testing it over
  writing a GTK test.
- Never `#[cfg]` a test away by platform. Skip at runtime and print
  `SKIPPED [<rubric>]: <reason>`.
- A test that installs process-global state restores it before returning.
- A new gate or guard is mutation-tested: break what it protects and watch it fail.

### GTK-object integration tests

- The exception, for behaviour that is genuinely about live widget or signal state.
- They sit in their owning module behind the integration-test feature, and helpers used
  only by them carry the same gate.
- Use `#[gtktest::test]` — never `#[gtk::test]` or a manual `gtk::init()` — so the test
  also runs where GTK is main-thread-only.
- Fail loudly when there is no display; never pass vacuously.
- A test that asserts on process-global GTK state gets its own standalone target, says
  why in its doc comment, and is added to the macOS integration step, which runs an
  enumerated list.

### Per-render memory-growth class

A separate test class and pipeline step, outside the coverage ratchet. It bounds memory
growth per render after warm-up — never absolute memory, and never a single
render-free-measure, which a correct implementation cannot pass.

### Manual integration testing

- Integration tests trace back to TDD rubrics; one that maps to no rubric is probably a
  unit test.
- **Every fix or behaviour change is proved against the running app** — per fix, by
  driving the exact scenario and reading the after-state. Passing tests are not
  sufficient. The procedure is in `tests/MANUAL-TEST.md`.
- **A regression fix needs both** an automated test and a manual-test check that was
  actually run. One without the other is incomplete.
- A new TDD section lands together with its manual-test section.
- A change touching the filesystem, IPC, packaging or paths is ratified only once the
  macOS and Windows seats have run it. Reproduce a defect on Linux before calling it
  platform-specific.

### Footprint verification

**VRAM ceiling: 50 MiB on every platform — hard limit, no exceptions.** It is the
project's reason to exist.

- Measure on a release build after any change to a rendering dependency, a rendering
  path, the process model, or how the renderer is selected. How to read the measurement
  on each platform is in TDD §6.
- Over the ceiling: stop, revert, pivot. Never keep the number down in a contrived
  measurement while it climbs in realistic use.

## Cross-platform by default

- Write every line as portable unless it lives in a platform seam.
- Never hardcode a path separator, path shape or filesystem root, **test fixtures
  included**. A fixture that encodes one platform's path grammar is selected per
  platform.
- Line endings are a property of the document, not the host. Never branch on the
  platform to decide what a line separator is.

### Platform seams

- Code that exists only because one platform lacks something lives in that platform's
  module under the platform directory, `#[cfg]`-gated at the module declaration and
  never inside shared code. Nothing else calls an OS API directly.
- A seam supplies plumbing — a source or a transport — feeding machinery all platforms
  share. It never owns application behaviour.
- Group a platform's code by the cause it answers, not by the API it happens to call.
- Prefer hand-rolled FFI over a new binding crate for a handful of calls.

## Code style

- `rustfmt` and clippy clean. Use `Result`/`?`; no `unwrap`/`expect` outside tests and
  startup invariants.
- Soft limit of 1,200 lines per file, checked before the edit that would exceed it.
- Keep functions small and shallow; hoist computation out of signal-wiring bodies.
- No magic numbers or strings. A closed set of values is an `enum`, parsed once at the
  boundary.
- Widget-owned closures capture weakly.
- Destructure tuples by name, never `.0`/`.1`.

## Dependencies

- No new dependency without justification; check the crate list in TECH.md first.
- Never add a web engine or HTML renderer.
- The in-house image decoder crate holds no GTK type and never reaches into the
  application. Its decoder dependencies are pinned exactly.

## Input limits

- Documents and images are untrusted, and cost is part of the threat. Every read of a
  user-supplied path first passes the one shared admission check — a regular file
  **and** within the size limit. Never reimplement either half.
- Decoded image size is bounded separately from file size.

## Architecture rules

- **Render Markdown into native GTK widgets only.** No web engine, no GPU-composited
  UI.
- **Force the software (Cairo) renderer before GTK initialises.** The app never holds a
  GL context. The test configuration pins the same renderer; do not remove it.
- **Native window frame on Windows.** Never add a header bar or custom titlebar on any
  platform, and keep the setting that disables client-side decorations Windows-only.
- **One source of truth for desktop light/dark.** Whatever detects a change writes GTK's
  prefer-dark setting; never re-theme a surface directly from a platform signal.
- **One process, many windows.** A platform without GTK's single-instance transport gets
  a substitute feeding the same open/activate handlers, not an exemption.
- **Never silently overwrite unsaved edits.** An external change to a dirty buffer asks
  the user; a clean buffer reloads.
- **One action per command.** Every surface showing a command — menu bar, toolbar,
  context menu, overlay — binds the same action by name, and its enabled state is driven
  from one place. Never set sensitivity or wire activation per surface. A command's
  accelerator is declared once, in its descriptor; menu models set no accelerator
  attribute of their own.
- **Extend an existing code path rather than adding a parallel one.** When a new path
  is justified, say why in the code.
- **All document I/O goes through the async document-I/O module**, never on the main
  thread. Only the app's own small state files may be read synchronously.
- **All network access goes through the one image-fetch module.** Never hand a URL to
  GIO.
- **All GTK access happens on the main thread.** The app starts no threads of its own;
  off-thread work goes to GLib's pool, carries only plain owned data, and has a stated
  concurrency bound.
- **List styling is symmetric** across bulleted, numbered and task lists at every depth.
- **No hard-coded styling.** Colour, typography and geometry come from the active
  theme, resolving selected theme → system theme → GTK probe. A theme chooses from a
  closed vocabulary of decorations; the engine holds no per-theme knowledge; an unset key
  means absent. Scale themed geometry by zoom explicitly, clamp it, and sanitise any
  string interpolated into CSS.
- **One theme key, every application path.** A surface rendered by more than one path
  takes its value from a single theme key.
- **Bundled decoration art is an original design.** Nothing copied, traced or derived
  from another work.

## Typed GTK seams

GTK's runtime contracts are enforced by nothing at compile time, so a contract that keeps
recurring is promoted into a type or choke point that makes the wrong call impossible.

- If a seam exists for what you are about to write, call it. If none exists and the
  contract is typeable, add one in that change.
- A seam lands with its enforcement — a lint ban or encapsulation — in the same change,
  and with a test proving the wrapped call, not just the type.
- A lint ban is a routing instruction: take the route it names, never `#[allow]` past
  it, and declare any new exception in the ban itself.
- Ban a raw method only when most of its hits would be real mistakes; otherwise
  encapsulate.
- A seam either returns a safe fallback or forces the caller to handle absence. Choose
  per seam.
- Link the seam from the lesson it enforces; delete the lesson once the seam says it
  all.
- Do not force contracts that are runtime ordering invariants. Those stay prose in
  ANTI-PATTERNS.md.

## Change accountability matrices (CAM)

A change in a [CAM](CAM.md) category — commands, rendering features, views derived from
the document, anything holding a position in it, document I/O — accounts for every
applicable cell of that matrix, and derives its manual-test checks from them.

## Version control

- Work that shares a mechanism, fixture or test rig lands as **one squashed commit** on
  the integration branch. Never fast-forward or cherry-pick part of it. Sharing a
  subsystem does not make a batch; work that will not fit one commit is more than one
  batch.
- Platform seats push directly into the Linux seat's clone: push with an explicit
  refspec, never a bare `git push`. Verify an integration by diffing trees.

## SDD register writes

- **ANTI-PATTERNS.md and ISSUES.md have one writer — the Linux seat.** Other seats send
  entry content; the writer allocates the ID. Do not cite an ID until the register
  carrying it has reached your clone.
- Route a lesson when it is written: about gtk4-rs → the `gtk4-rs` skill; general
  engineering discipline → the `general-engineering-principles` skill; anything else →
  ANTI-PATTERNS.md.
- Read an entry before citing it.
- **Never cite a git commit hash** — squashing orphans them. Cite the fact itself, a
  register entry, or the commit subject plus date.

## Prohibited actions

- **Never `git checkout` a file or path.** It discards uncommitted work
  unrecoverably. Copy the file aside and restore with `cp`.
- No `sudo` in any command an agent runs — it hangs on the password prompt. Ask the
  operator to install what is missing.
- Never block the GTK main thread on file I/O.
- Do not set `panic = "abort"`; the crash reporter relies on unwinding.

## Logging

- Log through the Rust `log` facade only, filtered by `RUST_LOG`. GTK's own messages
  are bridged into the same sink. Never use the glib logging macros.
- `error`: an operation failed and the app continues. `warn`: a recoverable anomaly.
  `info`: a lifecycle event. `debug`: per-operation detail. `trace`: hot paths, under an
  explicit target.
- `info` and above persist to disk and into crash reports, so every lifecycle boundary
  (open, save, reload, close, dialog, session restore) logs at `info`, once.
- **Never log document content** — not text, selections or clipboard, at any level. Log
  the path, the size, the decision.
- Repeated identical records are collapsed with a count, never capped.
- Messages are self-contained: path, event and relevant state.
- Programmer errors `panic!`; a controlled shutdown logs an `error!` and exits.
- Throwaway debug output is `eprintln!` prefixed `[🐛DEBUG]`, removed before commit.
- The test suites run with GTK criticals fatal on Linux and Windows. macOS cannot, for
  an upstream reason — do not arm it there by masking a log domain.
