# Plan: Per-render memory-leak gating

**Retention: this plan is kept after implementation** (operator decision), against SDD's
default of retiring a plan once its work lands. Do not offer to delete it. That does **not**
exempt it from the rest of retirement: when the fix and the gate ship, the decisions they
establish still migrate to their proper homes — the test class and its pipeline step to
POLICY, the rubrics to TDD, the runbook to `tests/MANUAL-TEST.md` — because a plan is where
a decision is *made*, never where it is *looked up*. What justifies keeping the file is the
measured evidence and the rejected approaches, which have no other home and are expensive
to re-derive.

## Problem

Scribobulate grows without bound as open documents are re-rendered. Measured on the
operator's live session: **836 MB to 1017 MB across roughly eight theme switches**
(~22 MB each), never returned. Reproduced headlessly on `master` from the same session
state: **104 MB to 2708 MB across 98 switches**, dead linear, no plateau at 2.7 GB.

A conservative RAM footprint is this project's reason to exist. A leak of this size puts
it in the company of the editors it was built to be an alternative to, so this is a
go/no-go defect in the same sense the VRAM ceiling is.

### Relationship to PLAN.profiling.md

That plan owns the **method**: the four failure classes, the tier ladder, and T3's
allocation-attribution order (RSS slope across scaled cycle counts, then a weak-ref guard,
then massif, then the `LD_PRELOAD` GType interposer), plus the comparability rules any
memory measurement must obey. **This plan does not restate any of it** — a second copy is
how the first stops matching.

This plan owns two things that plan does not: a *diagnosed, shipped defect*, and the
decision to build the **gate rung** as a standing test class with its own pipeline step.
PLAN.profiling.md observes that "every regression-gate rung is code" without committing to
one; this is that commitment, scoped to leaks only. Turn-latency and idle-CPU budgets
remain that plan's C1/C2 and are out of scope here.

**No existing gate detects it.** Every pipeline step was green throughout, and TDD section 6's
ceiling gates VRAM, not RSS over time. The defect reached the operator's desktop because
nothing in the tree has an opinion about memory growth. That gap is half of this plan;
the leak itself is the other half.

### Root cause

A local image was decoded once per render and the decode was retained.

`renderer/start.rs` used to decode a local image on every render — `Texture::from_file` at
natural size, `Pixbuf::from_file_at_scale` when zoomed. `imagecache` was URL-keyed for
**remote** images only, so no local decode was cached, bounded, or reused. That is
the shipped shape now: local keys are `local:{path}:{mtime}:{size}`, sharing the
existing LRU.

The growth is **retention, not allocator churn**: forcing aggressive glibc trimming
(`MALLOC_TRIM_THRESHOLD_`, `M_ARENA_MAX`) recovered only ~9 MB of an 80 MB step, and
growth stays linear across 98 renders. Nearly all of it sits in a single `[heap]`
mapping (680 MB of the operator's 836 MB at rest).

**Where the retention lives:** `webp-pixbuf-loader`'s animated branch.
`Pixbuf::file_info` ~2.3 MB/call, `Texture::from_file` ~12 MB/call, together
~22 MB/render. No GTK decode route is flat on a valid animated WebP
(`static_image` leaks the same and SIGSEGVs on truncated WebP). The cache is
what makes a re-render flat; a first unique decode still pays the leak.

Two properties shape the fix:

- **Animated rasters amplify it.** A 900x670 80-frame animated WebP is ~193 MB fully
  decoded at 4 bytes/pixel. Measured ~25 MB retained per theme-switch render.
- **It is render-generic.** Theme switch, zoom and live reload share the re-render path.
  Live reload matters most: it is the core use case and it fires unattended while an
  agent rewrites a file.

## Measured evidence

Release build, Cairo renderer, headless Xvfb, theme driven via `org.gtk.Actions.SetState`.
Growth is per cycle of 7 switches unless stated.

| Fixture | Growth/cycle | Reading |
|---|---|---|
| Operator session, 36 tabs / 7 windows | +157 MB, linear to 2.7 GB | the reported defect |
| 7 windows x 1 tab, zoom 1.0 | flat after cycle 1 | window count is not the trigger |
| 1 window x 7 tabs | flat after cycle 1 | tab count is not the trigger |
| 1 window x 16 distinct large docs | flat after cycle 1 | document size/count is not the trigger |
| 4 windows x 4 docs, zoom 1.25 | flat after cycle 1 | **zoom is not the trigger** |
| README.md | +178 MB | isolated to one document |
| README.md lines 1-40 | +175 MB | isolated to the `<picture>` block |
| Animated WebP alone | **+172 MB** | the trigger |
| Animated GIF alone (120 frames, same size) | +22 MB | format-specific, GIF is clean |
| No images at all | +18 MB, flat over 72 cycles | warm-up, not a leak |
| Animated WebP, 20 zoom renders | **+7.8 MB per render** | render-generic |

The ~18-22 MB baseline is first-cycle cache warm-up (GTK icon cache pulling in librsvg,
per-theme font loading). A 72-cycle run on an image-free document was flat to within
228 KB, so warm-up saturates and the leak does not.

## Possible approaches — the defect

### ✅ 1. Take our render path off the leaking route, and cache what remains

**There is a defect of ours here, and it is not the leak.** The leak belongs to a
third-party loader we cannot patch. What belongs to us is that **we call it, by a route we
chose, once per render, forever** — and a non-leaking route demonstrably exists. Three
things in this tree are wrong independently of upstream:

1. **We call a leaking entry point where a flat one exists.** `Pixbuf::file_info` leaks;
   `gdk_pixbuf_animation_new_from_file` reading width/height only is **flat**, measured on
   the same asset. That is a route choice, in our code, with a measured better option.
2. **We decode on every render, because there is no local-image cache at all.** That is a
   defect on its own terms and it is already on the register in its CPU form — a large SVG
   costs 239 ms per render on the main thread for the same reason. Upstream is irrelevant
   to that one; it would still be a bug if every loader were perfect.
3. **We decode 80 frames to display one.** The application renders images statically
   (measured — see *Diagnosis*), so entering an animated decode path at all is work we
   never use, in both memory and CPU.

Upstream supplies the hole. **We drive over it, repeatedly, at every re-render, and we
chose the road.** Fixing 1–3 is what this plan delivers on the defect side; the gate (A) is
what stops the next one.

**Pros**: entirely within this tree — no upstream dependency, no patched loader, no waiting
on a distribution. Fixes a real defect (2) that outlives the WebP question entirely.
**Cons**: route avoidance must be re-verified per loader rather than reasoned once; the
route table is measured for WebP and assumed for nothing else.

### ❌ 2. Caching alone, without changing the route

Extend the decoded-image cache to local paths (path + mtime + quantised target size, under
the existing LRU byte budget) and change nothing else.

**Rejected as a *standalone* fix — but adopted as the second half of ✅1.** Caching does not
fix a leak it cannot reach: it reduces how often we invoke an unfixable defect without
removing a single leaked byte per invocation. A reader stepping through zoom levels pays it
once per distinct size, forever. That makes it a genuine mitigation and a genuine
performance win, and **not** a cure — so it ships *with* route avoidance, never instead of
it.

The earlier objection to caching — that it would *mask* our own retention bug — is void,
because there is no retention bug of ours to mask. It was the right call on the evidence
available and is recorded here so nobody re-derives it.

### ❌ 3. Decode animated rasters to a single frame

Take frame 0 for animated formats rather than materialising every frame.

**Rejected — operator decision.** It would remove the 80x amplifier cheaply, and that is
the whole of its appeal, but it buys memory by silently dropping animation. **Degrading UX
is not an acceptable currency once the UX bar has been set**, and a reader whose animated
image stopped animating has been handed our memory problem as their rendering problem. It
also would not fix the per-render retention for large static images, so it trades a
visible feature for a partial fix.

The same reasoning rules out the softer version: **no decoded-frame budgeting.** How many
frames an image carries is the end user's business, not a number this application gets to
cap on their behalf.

### ❌ 4. Remove `assets/splash.webp` from README.md

**Rejected.** It punishes the end user for our defect — removing the asset implies the
broken thing is the *content*, when the broken thing is the code we are responsible for.
Any user document embedding an animated WebP leaks identically and no README edit reaches
them. It would also destroy the reproducer a gate needs.

### 💡 5. Decode WebP ourselves and stop using the module at all

**Status: open, and deliberately deferred — it does not block this plan** (operator
decision). Fix our own defect first; this is picked up by a future session when the
operator chooses. It stays here in full rather than being summarised, so whoever takes it
up inherits the analysis rather than re-deriving it.

Operator's proposal. Rather than routing around a broken loader, remove it from the path:
decode WebP in this tree and hand GTK a finished `GdkTexture`
(`Texture::from_bytes`, already used for remote images — ScrAP-292). Two very different
sizes of the same idea, and they should not be costed together:

**5a — still-frame decode only.** Produce frame 0 and nothing else. This is all the
application renders today (measured: paintables are `STATIC_CONTENTS`), so it is a pure
defect fix with no behavioural change.

**5b — a full animated WebP component.** 5a plus actually animating. This is **not a bug
fix, it is a new feature** — the application has never animated an image on any platform.

**Pros** (5a, and inherited by 5b):
- **We consume a fraction of the component's surface and inherit all of its failure
  modes.** The leak is in the loader's *animated* branch — a branch this application never
  uses, since it renders frame 0 and discards the rest. A general-purpose loader carries
  the whole format's generality, and its bugs, to a caller that wanted one still image.
  That mismatch is the argument for owning the narrow thing, and it is why the narrow
  thing is often *smaller* as well as more stable: it is not a smaller version of the
  component, it is a different and much shorter problem.
- **Removes the defect class, not this instance.** Route avoidance (✅1) is per-loader:
  the route table is measured for WebP and assumed for nothing else, so every future
  format is fresh whack-a-mole. Owning the decode ends that for WebP permanently, on every
  distribution, regardless of what the user's `webp-pixbuf-loader` does.
- **It is a portability gain, though a smaller one than first claimed.** Neither Windows
  nor macOS ships a WebP pixbuf loader (both measured), so WebP decodes on exactly one of
  three platforms. ⚠ **Correcting an overstatement made earlier in this plan**: that does
  *not* mean users see a broken image on the other two. TDD 2.23's `<picture>` fallback is
  designed for exactly this and skips the undecodable candidate, so the README hero renders
  its GIF there. The real gap is a **bare `<img>` pointing at a WebP**, which has no
  fallback to take and shows the broken-image placeholder on two platforms out of three.
  Our own decoder would close that and make the three behave alike.
- **A pure-Rust decoder is memory-safe**, where the thing it replaces is an unaudited C
  module parsing untrusted input on the main thread. That is a security improvement, not
  merely a lateral move.

**Cons**:
- ⚠ **"Roll our own" here means *own the decode*, not *invent the format*.** We do not
  control this input: users' documents carry standard WebP, so the bitstream (VP8/VP8L,
  plus the RIFF container and, for 5b, the ANMF/ANIM demux) must be implemented to spec.
  The tenable version of this approach is therefore *integrate a narrow decoder crate and
  own the path*, not *write a codec* — the win is scope control and dependency choice, not
  format design. Costing it as though the format were ours would badly under-estimate it.
- **A new dependency, which POLICY gates.** Note the precedent does *not* transfer: the
  `pangocairo` justification was "already linked into the process by GTK, so this adds
  bindings and no system dependency." That is **false here** — `libwebp` enters the process
  only when the loader decodes, and on Windows it is absent entirely. A C binding therefore
  adds a real system dependency to two platforms' packaging. A **pure-Rust** decoder avoids
  that and is the variant worth costing; its animated-WebP coverage needs verifying before
  anyone commits to 5b.
- **Attribution and licence obligations** — `THIRD-PARTY-LICENSES.md` and `notices/`.
- **"Bug-free" is not achievable by intention.** We would be trading a known bug for
  unknown ones. What makes this defensible now and would not have before is that this plan
  *also* builds the gate: the replacement lands under a growth-slope test that would have
  caught the very defect being replaced.

**Cost specific to 5b, and it cuts against the project's premise**: animation means
repainting continuously on the main thread under a software renderer, and holding decoded
frames resident. This is an application whose reason to exist is a small footprint, that
already carries a CPU-spin defect on the register, and that pins the Cairo renderer. An
animated image is a standing CPU and memory cost bought for decoration. **5b should be
weighed as a product decision, not folded in as part of a leak fix.**

⚠ Note ❌3 was rejected on the grounds that dropping animation degrades UX. That reasoning
does not carry over to 5b as a reason *for* it: there is no animation today to preserve, so
5b **adds** a capability rather than defending one, and it has to earn its footprint on its
own merits.

## Possible approaches — the gate

### ✅ A. A new test class with its own mandatory pipeline step

A third class alongside unit and integration tests, deterministic, excluded from the
coverage ratchet, and a mandatory pipeline step with no opt-in or opt-out.

Two kinds of assertion, catching disjoint failures:

1. **Finalization (deterministic).** Weak-ref the per-render decoded object, including
   the cache, and assert it finalizes with no main-loop pump. No thresholds, no
   sampling. Sound only under Cairo; the gate asserts `NativeExt::renderer()` is
   `gsk::CairoRenderer` (None panics) because `$GSK_RENDERER` is defeatable. See
   the GSK precondition below.
2. **Per-render growth slope.** Drive a render loop over a fixture and assert growth per
   render stays under a per-platform bound. Catches leaks nobody predicted — including
   this one, which no finalization assertion would have caught before someone knew to
   write it.

The bound is **per-render growth, not absolute footprint**. Absolute size is a design
question warranting higher-level planning, not a gate.

**Neither half needs to be Linux-only** (measured by the `mac` seat, GTK 4.22.4/M4). Half
2 is portable in *mechanism*: one sampler trait with three `cfg` bodies — `/proc` VmRSS on
Linux, `libc::proc_pid_rusage(RUSAGE_INFO_V2).ri_phys_footprint` on macOS,
`GetProcessMemoryInfo` on Windows — and no new dependency on any seat (`libc` is already in
`Cargo.lock`). Name the field **`footprint`, never `rss`**: the three numbers are not the
same quantity, and a shared name invites a shared threshold. Tolerances are **per-platform
constants**, never one shared number.

⚠ **Freed memory is not returned memory — this is a property of the class, not a
per-platform caveat.** macOS malloc keeps freed pages in the zone: a 256 MB allocation
dropped moved the footprint by nothing (257.19 MB before and after). Linux behaves the same
in kind — aggressive glibc trim returned only ~9 MB of an 80 MB step. Two consequences bind
every assertion in this class:

- **Never write a single-shot "render, free, assert the number came back".** It cannot pass
  on a correct implementation. This is the trap the whole class exists to avoid and it is
  the most natural test to reach for.
- **Slope over N with K warm-up renders discarded is the only honest shape.** A
  non-leaking loop plateaus once the allocator is warm; a leaking one climbs without bound.
  K must cover *allocator* warm-up, not merely first paint.

⚠ **The GSK precondition — resolved, and the resolution is a constraint, not a
reassurance.** Finalization is sound at the glib layer, but the question was whether GSK
retains a rendered `GdkTexture`, which would redden the assertion on healthy code.
Measured with one probe run on two seats (`probes/gsk-texture-ref-ownership.c`):

| Version / seat | `GskCairoRenderer` | `GskGLRenderer` |
|---|---|---|
| 4.22.4, `mac` | finalizes, 0 iterations | finalizes, 0 iterations |
| **4.6.9 floor**, `linux` | finalizes, 0 iterations | **never finalizes** |

So the assertion is safe **because this project pins `GSK_RENDERER=cairo` unconditionally**
(`lib.rs`) — not because GSK is generally well-behaved. At our floor, GL never releases the
texture at all, through either the caller's ref or the node tree. **The forced renderer is
load-bearing, and any proposal to let it vary re-opens this and must re-measure first.**

The contract is therefore: drop every app-side ref *including the render node tree*, then
assert the `GWeakRef` is NULL. **No main-loop pump, no forced frame count** — a
"pump N iterations" line would encode a guess about a scheduler, and it is exactly the
construct that passes on one seat and reds on another later.

Half 1 can be **mandatory**. Two caveats to carry rather than lose: the GL result is
software GL under Xvfb, a configuration this tree already knows is odd (GTK4Rs/AP-129), so
real-display GL is *unmeasured* and not claimed; and a source trace predicting both
versions would behave identically was **falsified by running it** — right for cairo, right
for 4.22.4 entirely, wrong at the floor for the one renderer nobody would run.

**Pros**: a new step can later be turned off as a unit if it proves noisy; both halves run
on all three seats, so no platform is blind to leaks. **Cons**: a third class is new surface
to maintain, and half 2's per-platform tolerances are three numbers to keep honest.

### ❌ B. Fold into pipeline step 5 (integration)

**Rejected** on the operator's call: a separate step can be disabled independently later,
where a fold cannot be undone without unpicking step 5.

### ❌ C. Absolute RSS ceiling

**Rejected.** It would fail on legitimate large documents and pass a slow leak on a small
one. It measures the wrong quantity for this purpose.

## Recommendation

Take **1** then **A**. Both are wholly inside this tree; neither waits on anyone.

**Approach 5 does not block this plan and must not delay it** (operator decision). Fix our
own defect first; the own-the-decoder question is deferred to its own plan and picked up
when the operator chooses. An implementing session that stops to research a decoder has
misread the priority.

**Implementing this plan means shipping the fix, not only the gate.** The gate is what
stops the next leak; it does nothing about this one. A session that lands the test class
and leaves the render path on the leaking route has completed none of this plan.

**And the fix is not a formality because the leak turned out to be upstream.** Diagnosis
moved the *retention* out of this tree; it did not move the *route choice*, the *missing
cache*, or the *unused animated decode* — all three are ours, and all three are why a user
sees 2.7 GB.

| # | Deliverable | Ours? |
|---|---|---|
| 1 | Stop calling the leaking entry points where a measured flat route exists | yes |
| 2 | A bounded local-image decode cache (also kills a 239 ms/render main-thread stall) | yes |
| 3 | The leak gate: growth-slope + finalization, its own mandatory pipeline step | yes |
| 4 | Patch `webp-pixbuf-loader` | **no** — not ours, not shipped by us, explicitly rejected |

Sequence:

1. Land the failing **growth-slope** test first, against the WebP fixture. It must fail on
   `master` before anything is fixed — a leak guard that has never been seen red is not
   evidence (ScrAP-209: a guard whose setup prevents the resource from existing passes with
   the fix deleted). Mutation-test it, and check the mutation fails for the reason intended
   rather than on an earlier precondition (ScrAP-183, ScrAP-254).
   ⚠ **The finalization half cannot catch this defect** — the `GdkTexture` we hold
   finalizes correctly (verified 40/40); the leak sits behind it, in a module we never
   reference. That asymmetry is the strongest argument in this plan for why the class needs
   *both* halves: a leak can be entirely real and entirely invisible to refcount assertions.
2. **Take the render path off the leaking route, and add the cache.** This is the fix, and
   it ships with this plan.
3. Wire the new pipeline step with both halves.
4. Measure the cache's effect on the SVG stall as well — the same change should show up
   there, and if it does not, the cache is not doing what it was justified on.

**Rubrics before code — the plan-kickoff stop applies.** A new test class needs TDD
rubrics saying what must be true of memory across a re-render, authored before the
harness exists; rubrics written afterwards describe whatever the harness happens to
measure. The per-render bound is one of them and cannot be chosen up front — derive it
from a measured clean baseline once the retention is fixed.

**Where this lands in the documents.** POLICY gains the new test class and its pipeline
step (and states that the class is outside the coverage ratchet, so
`scripts/coverage.scope` is untouched); `scripts/pipeline.steps` gains the step itself,
following the `4b` precedent for insertion without renumbering, with `intent` pinned and
`cmd.<platform>` per platform; TDD gains the rubrics; `tests/MANUAL-TEST.md` extends
section 1.8, which today covers only the VRAM and RSS *ceilings*. ANTI-PATTERNS gains an
entry once the retention is named — not before, since the lesson is the mechanism.

## Diagnosis

**The retaining owner is not in Scribobulate.** It is `webp-pixbuf-loader`
(0.0.5-5~22.04.1, `libpixbufloader-webp.so`, stripped), a third-party gdk-pixbuf module.
Its **animated** branch over-references `GdkPixbufWebpAnim` through a
`GdkPixbufWebpAnimIter` it creates and never releases; the anim then keeps the decoded
frame and the whole file buffer alive forever. Measured refcount on the anim is **2 where
GIF's is 1**, and the surplus is never dropped.

Two independent leaks in that one module sit on our render path:

| Our call site | Route | Leak |
|---|---|---|
| `renderer/start.rs:846` `Pixbuf::file_info` | `gdk_pixbuf_get_file_info` | 2.31 MB/call |
| `renderer/start.rs:873` `Texture::from_file` | `new_from_stream` → `GdkPixbufLoader` **incremental** path | 11.7–12.9 MB/call |

They are **super-additive in sequence** — 2.31 + ~12.2 separately, **22.0 measured
together**. The 22.0 is measured; fragmentation as the *reason* is inferred.

**Proof it is not ours:** a 20-line C program making exactly the two calls `load_texture`
makes per render — no widget, no buffer, no Scribobulate code — leaks 22.27 / 22.04 /
21.96 MB per iteration at n = 5 / 13 / 40. The application measures 22.17 and 22.01
MB/switch on a one-line `<img src="assets/splash.webp">` document. **The C probe and the
application agree to 1%.**

**Ruled out**, each excluded by the single fact that 22 MB/render reproduces with none of
them present: anchored `GtkPicture` not detached; Pango shape attribute holding a texture
ref; `Rc`/closure cycle through a controller (ScrAP-155's shape); copymap/offset-map
rebuild; `imagecache` (never entered — the path is `ImageResolution::Local`). The
`GdkTexture` **we** hold was weak-pointer-verified finalized 40/40 in every run.

**The GIF/WebP asymmetry is fully explained** and narrows further than expected: it is
**per-loader-module, not per-format-family**, and a *static* WebP of identical dimensions
is flat. The trigger is specifically the animated branch of that one module.

Route sensitivity, same asset — this is what the fix has to work with:

| Entry point | Result |
|---|---|
| `gdk_pixbuf_new_from_file` | errors outright ("Cannot create WebP decoder") |
| `new_from_stream` / `GdkPixbufLoader` | **leaks 11.7–12.9 MB/call** — what `Texture::from_file` uses |
| `gdk_pixbuf_animation_new_from_file`, read w/h only | **flat** |
| `gdk_pixbuf_get_file_info` | **leaks 2.31 MB/call** |

⚠ **A premise in this plan is measurably false, and it is load-bearing for ❌3.** The
application does **not** animate the WebP today: `gdk_paintable_get_flags` on the texture
`Texture::from_file` returns is `0x3` (`STATIC_SIZE | STATIC_CONTENTS`) for both WebP and
GIF, and `src/` contains no animation handling at all — the decode path returns a
`GdkTexture`, which is structurally a single image. All 80 frames are decoded and thrown
away on every render; only frame 0 is ever shown. ❌3 was rejected on the grounds that it
would "silently drop animation"; there is no animation to drop. **The rejection is left
standing pending the operator**, because the decision was the operator's and the premise
is theirs to re-weigh — but it must not be inherited unexamined.

### What this does to the fix

Approach ✅1 said *break the retention*. We cannot: the defect is in a distribution
package, and no `.deb` we ship changes the user's loader. So the fix becomes **avoid the
leaking routes**, and the route table above says that is possible — a non-leaking path to
dimensions already exists. **This also rehabilitates 💡2**: caching was rejected as the
fix because it would *mask* our retention bug, and that objection dies with the bug. It no
longer masks anything — but it only reduces how often we invoke a leak we cannot fix, so
it is a mitigation, not a cure, and route avoidance outranks it.

## Technical details preserved

**Reproducer.** A document containing `<img src="assets/splash.webp">` where the path
resolves, driven through repeated re-renders. ⚠ **The gate is portable; this asset is
not.** The gvsbuild prefix ships exactly one pixbuf loader (SVG), so an animated WebP does
not decode at all on Windows — it renders a broken-image icon, and the leak is
structurally unreachable there. A **shared** fixture must be a large PNG; the WebP fixture
is Linux/macOS-only and should declare itself skipped elsewhere rather than silently
passing. The asset is 7.4 MB, 900x670, 80 frames.
A committed fixture should carry its own small animated WebP rather than depending on
`assets/splash.webp`, which exists for the README and may change.

**Drive method.** `gdbus call --session -d com.extollit.scribobulate -o
/com/extollit/scribobulate -m org.gtk.Actions.SetState preview-theme "<'sepia'>" "{}"`.
Zoom uses `Activate` on `zoom-in`/`zoom-out` at `/com/extollit/scribobulate/window/1`.
Driving the GAction bypasses the toolbar and menu popovers entirely, which is what makes
the measurement deterministic: kwin-on-Xvfb will not deliver a synthetic click to a
non-autohide popover surface, so a popover-driven measurement is unreliable (ScrAP-101).

**Measurement.** `VmRSS` from `/proc/<pid>/status`; per-mapping attribution from
`/proc/<pid>/smaps`. Two readings worth keeping, neither in PLAN.profiling.md: `VmHWM`
equal to `VmRSS` means the process is at its peak and has returned nothing; and to separate
retention from allocator slack, re-run with `MALLOC_ARENA_MAX=1`,
`MALLOC_TRIM_THRESHOLD_=65536`, `MALLOC_MMAP_THRESHOLD_=65536` — if RSS barely moves, the
memory is genuinely referenced. (Scaling the cycle count is that plan's rule, not restated.)

**Harness requirements.** `xvfb-run -a ... dbus-run-session -- ...` in that nesting, plus
`GTK_USE_PORTAL=0`; a private `XDG_STATE_HOME` so the suite never touches the developer's
session file; `GSK_RENDERER=cairo`. Identify the process by the PID captured at launch,
never by name — a process name is not an identity (ScrAP-241).

**Traps already paid for, all of which produced clean-looking false negatives:**

- Deleting the `<img src="...gif">` line from README leaves `<source srcset="...webp">`
  live inside the same `<picture>` block, so the WebP still loads and the GIF looks guilty.
  Remove the whole element when isolating.
- Copying a document to `/tmp` breaks relative image paths, so no image loads at all and
  the fixture silently measures nothing. Image fixtures must sit where their paths resolve.
- Three cycles is too few to separate warm-up from a leak; the first cycle is dominated by
  GTK icon-cache and font loading. This is PLAN.profiling.md's scale-the-count rule biting
  in practice: warm-up is flat across counts, the leak grows with them.
- A single-window or single-document fixture does not reproduce it, which is what sent the
  first investigation to a false "no leak" conclusion. The trigger is the document content,
  not the window or tab count.

**Format asymmetry.** The animated GIF does not leak and the animated WebP does, through
what is nominally the same `Texture::from_file` loader chain. Whatever the fix, the
regression fixture should be a WebP, and the GIF is a useful negative control.

## Open decisions

- **Whether ❌3 should be reopened.** ✅ Operator confirmed frame 0. TDD 2.23 already
  specified it; `Texture::from_file` already returns `STATIC_CONTENTS`. Tried
  `PixbufAnimation::static_image` as the decode: same leak on a valid animated WebP,
  and **SIGSEGV** on truncated WebP (`undecodable_webp_degrades_to_one_anchored_child`).
  Raster decode stays `from_file`; the cache is what makes re-renders flat.
- **The per-render growth bound's value.** ✅ Shape, not a byte count.
  `memgate::footprint::TOLERANCE_BYTES` is 2 MiB on Linux and 4 MiB on macOS/Windows;
  warm-up is 3 samples; 10 samples after that, five per half.
- **How to avoid the leaking routes.** ✅ Dimensions via
  `gdk_pixbuf_animation_new_from_file` (flat after warm-up). No GTK decode route is
  flat on animated WebP (measured: `from_file`, `from_bytes`, one-shot `PixbufLoader`,
  `static_image` all leak). Cache (💡2) ships with the route change so a re-render
  does not invoke the loader again. SVG second load: 234 ms → 22 µs.

### Routing an anti-pattern from this

Two separable lessons, deliberately not written yet because the number-claim is the
operator's:

1. **Resident, `Disp C`, ≤6 lines** — the third-party module defect itself. It is not a
   `gtk4-rs` skill entry: the bug is in a distribution pixbuf loader, not core GTK.
2. **Separable and core-GTK-adjacent** — `GdkTexture::from_file` reaches a pixbuf module's
   **incremental** (`begin_load`/`load_increment`/`stop_load`) path, not its one-shot
   `load`. A module bug present in only one of the two is therefore reachable from the
   plainest possible GTK call, and `new_from_file` erroring while `new_from_stream` leaks
   is that asymmetry showing. Kin `GTK4Rs/AP-66`. **Raise with the operator before routing
   to the skill.**

## After-effect: PLAN.profiling.md narrows

Worth stating because it is a deliverable of this plan and not a side note. That plan's
C4 (leak) class is currently served by an *on-demand* T3 ladder — a human remembering to
run it. Once this class exists, C4 is covered by a standing, deterministic gate, and the
ladder reverts to what it is good at: attributing a leak the gate has already caught.

So on completion, PLAN.profiling.md should be narrowed rather than left as written — its
scope becomes C1/C2 (turn latency, idle CPU) plus T3-as-diagnostic. That is a real
reduction in what it still has to build, and the narrowing is the responsibility of the
session that implements this plan, not a later cleanup.
