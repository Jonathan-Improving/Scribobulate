# Plan: Per-render memory-leak gating, and owning the animated-image decode

**RETIRED 2026-09-12 — both phases shipped.** Kept, at the operator's decision, for the
MEASURED EVIDENCE below: the leak's numbers, the route-by-route table, and the C-program
corroboration are expensive to re-measure and are cited from code comments across
`src/imagedecode/`, `src/animation/`, `src/memgate/` and `richimg/`. Its durable decisions
have migrated to their homes — TECH.md (the crate rows, the `imagedecode/` and `animation/`
module entries, the concurrency model), POLICY.md (the worker's obligations to the shared
pool, image admission, richimg's place in the workspace), CAM.md (the menu-only exception),
TDD 2.23a/b, 6.6-6.10 and §27 with their `tests/MANUAL-TEST.md` checks, and ScrAP-351 /
ScrAP-352. **This file is no longer a work plan**: the work-package briefs and wave schedule
it carried during implementation are history and have been pruned; git holds them.

| Phase | Scope | State |
|---|---|---|
| 1 | Take the render path off the leaking route, cache local decodes, build the leak gate | **Shipped** |
| 2 | Decode WebP ourselves (`richimg`), animate WebP, GIF **and** APNG at full fidelity, never animate what is not visible | **Shipped** |

## What implementation changed about this plan

Three of the decisions below were WRONG as written, and were corrected against measurement
rather than followed. They are left in place with their corrections so the record shows what
was believed and what was found:

- **Dispose-to-background does NOT fill the ANIM background colour.** Every reference
  renderer clears to transparent (libwebp's `ZeroFillFrameRect`, Blink, Gecko, WebKit,
  `magick -coalesce`), and the `background_color_hint()` this plan said to pass is also in
  on-disk BGRA order, so following it would have painted red and blue swapped. Corrected in
  the "Decoder" section below.
- **`has_tick_callback()` does not exist** at this project's GTK floor — not in 4.6.9's
  headers, not in gtk4-rs under any feature — so the visibility gate's oracle is the
  type's own bookkeeping PLUS the functional fact that the painted pixels stop changing.
  Weaker than this plan assumed; said plainly where it is relied on.
- **A collapsed `<details>` does not park its child at (−w,−h)** in this renderer: the fold
  deletes the buffer range and `GtkTextBuffer::delete` unparents anchored children outright.
  The visibility table's row for it is therefore satisfied by a different mechanism.

One scope gap remains and is tracked in `sdd/ISSUES.md`: an animated sprite plays in the two
BAND decorations and shows its first frame in the other four sprite slots.

## Problem

Scribobulate grew without bound as open documents were re-rendered: **836 MB to 1017 MB
across ~8 theme switches** on the operator's session, and **104 MB to 2708 MB across 98
switches** headlessly, dead linear. A conservative RAM footprint is this project's reason
to exist, so this is a go/no-go defect in the same sense the VRAM ceiling is. No gate saw
it: every pipeline step was green, and TDD §6 gated VRAM, not growth.

Phase 1 made re-renders flat. What remains is that **the decoder itself still leaks**, and
phase 1 only stops us calling it twice for the same file:

- Every **first** decode of a distinct animated WebP still leaks ~12 MB, forever.
- A decode the cache has **evicted** (32 MiB budget) or whose file **changed** leaks again.
  Inferred, not measured: a document whose images exceed the budget re-leaks on every
  re-render, and 6.6 cannot see it because it measures the cached path.
- WebP does not decode at all on Windows or macOS (no loader ships), so a bare `<img>` of a
  WebP is a broken-image placeholder on two platforms of three.
- Remote images, theme sprites and PDF export decode through the same leaking loader.

### Relationship to PLAN.profiling.md

That plan owns the **method** (failure classes, tier ladder, T3 attribution order,
comparability rules); this plan does not restate it. Turn-latency and idle-CPU budgets are
that plan's C1/C2 — but phase 2's animation **creates** a standing CPU cost, so its
"never animate what is not visible" gate sits here, as a deterministic assertion rather
than a CPU budget.

### Root cause

The retaining owner is **not in Scribobulate**. It is `webp-pixbuf-loader`
(0.0.5-5~22.04.1, `libpixbufloader-webp.so`), a third-party gdk-pixbuf module. Its
**animated** branch over-references `GdkPixbufWebpAnim` through a `GdkPixbufWebpAnimIter`
it never releases; the anim keeps the decoded frame and the whole file buffer alive.
Refcount on the anim is **2 where GIF's is 1**. Per-loader-module, not per-format: a
*static* WebP of identical size is flat, and GIF is clean.

What was ours: we chose a leaking entry point where a flat one existed, decoded on every
render because local images had no cache, and decoded 80 frames to show one.

## Measured evidence

Release build, Cairo renderer, headless Xvfb, theme driven via `org.gtk.Actions.SetState`.
Growth per cycle of 7 switches unless stated.

| Fixture | Growth/cycle | Reading |
|---|---|---|
| Operator session, 36 tabs / 7 windows | +157 MB, linear to 2.7 GB | the reported defect |
| 7 windows × 1 tab / 1 window × 7 tabs / 16 large docs / zoom 1.25 | flat after cycle 1 | none of these is the trigger |
| README.md lines 1–40 | +175 MB | isolated to the `<picture>` block |
| Animated WebP alone | **+172 MB** | the trigger |
| Animated GIF alone (120 frames, same size) | +22 MB, then flat | warm-up; GIF is clean |
| No images, 72 cycles | +18 MB, then flat to 228 KB | warm-up saturates, a leak does not |
| Animated WebP, 20 zoom renders | **+7.8 MB per render** | render-generic |

**Proof it is not ours:** a 20-line C program making only the two calls `load_texture`
made per render leaks 22.27 / 22.04 / 21.96 MB per iteration at n = 5 / 13 / 40; the
application measured 22.17 and 22.01 MB/switch. They agree to 1%. The `GdkTexture` we hold
was weak-ref-verified finalized 40/40 — **a leak can be entirely real and invisible to
refcount assertions**, which is why the gate has a slope half.

Route sensitivity on the same animated WebP:

| Entry point | Result |
|---|---|
| `gdk_pixbuf_new_from_file` | errors outright ("Cannot create WebP decoder") |
| `new_from_stream` / `GdkPixbufLoader` (what `Texture::from_file` and `from_bytes` use) | **leaks 11.7–12.9 MB/call** |
| `gdk_pixbuf_get_file_info` | **leaks 2.31 MB/call** |
| `gdk_pixbuf_animation_new_from_file`, read w/h only | flat |
| `PixbufAnimation::static_image` | leaks the same, and **SIGSEGVs on truncated WebP** |

The two leaking calls are super-additive in sequence: 2.31 + ~12.2 alone, 22.0 together.

## Possible approaches — the defect

### ✅ 1. Take our render path off the leaking route, and cache what remains (shipped)

`renderer/start.rs` probes SVG dimensions with `gdk_pixbuf_animation_new_from_file`
instead of `Pixbuf::file_info`, and `imagecache` now caches local decodes under
`local:{path}:{mtime}:{size}` in the shared LRU. Re-renders are flat; the large-SVG second
load went from 234 ms to 22 µs. It could not remove the per-decode leak — no GTK decode route
is flat on a valid animated WebP — which is what phase 2 is for.

### ❌ 2. Caching alone, without changing the route

Reduces how often an unfixable leak is invoked without removing a byte of it. Adopted as
half of ✅1, rejected as a standalone fix.

### ❌ 3. Show frame 0 only, permanently

Rejected by the operator: the target is **full animation at full fidelity**. (The
application does show frame 0 today — `Texture::from_file` returns `STATIC_CONTENTS` —
and TDD 2.23 still says animation is out of scope; phase 2 changes both.) The softer
version, capping how many frames an image may carry, is rejected on the same ground.

### ❌ 4. Remove `assets/splash.webp` from README.md

Punishes the content for our code, reaches no user document, and destroys the reproducer.

### ✅ 5. Own the decode — `richimg` — and animate WebP, GIF and APNG

**Decided (operator):** build it now, as phase 2 of this plan rather than a separate plan,
and animate rather than stop at frame 0. The reasoning that carried it:

- **It removes the defect class rather than routing around one instance.** Route avoidance
  is per-loader and was measured for WebP only; owning the decode ends it for WebP on every
  distribution, whatever the user's loader does.
- **Portability:** WebP renders on all three platforms instead of one.
- **Memory safety:** a pure-Rust decoder replaces an unaudited C module parsing untrusted
  input on the main thread.
- **It lands under a gate** that would have caught the defect it replaces.

Every phase 2 decision below is settled (✅) or rejected (❌).

## Phase 2 — decisions

### ✅ `richimg`: a workspace crate, loosely coupled

Scribobulate's animated-raster decoder, for WebP, GIF and APNG. A workspace member beside
`gtktest` and `xtask`, in `default-members` so steps 1, 2 and 4 format, lint and test it.
**No GTK dependency**: bytes in, dimensions and composited RGBA8 frames (with per-frame
durations and loop count) out. It never sees the application, so moving it
to an external dependency later is a `Cargo.toml` change, not a refactor. The decoder crate
it wraps is an implementation detail behind its API.

### ✅ Decoder: `image-webp` 0.2.4, pinned

Pure Rust, `#![forbid(unsafe_code)]`, MIT OR Apache-2.0 (compatible with Apache-2.0), MSRV
1.80.1, two tiny dependencies. Backend of the `image` crate, GNOME glycin, resvg, Servo.
Implements VP8, VP8L, ALPH, VP8X and ANIM/ANMF. Researcher-measured on `splash.webp`:
frame 0 **bit-exact** against `dwebp` (0 / 603,000 pixels differ); a lossy 900×670 frame
decodes in ~9 ms (~1.6–2.4× libwebp); truncated input returns `Err`, never panics.

Carried obligations:

- **Known panics on crafted input** — image-webp#182 (zero-sized VP8 in ANMF after ALPH;
  fix is PR #185, unreleased) and #118 (lossless Huffman overflow, still open). So the
  decode runs under `catch_unwind`, and a panic degrades exactly like a decode `Err`. A
  panic on the GTK main thread would take the process down.
- **No release since 2025-08-27** with fixes waiting on `main` (issue #183). A yellow
  flag, not a show-stopper (operator). Pin 0.2.4 and watch for 0.2.5.
- **Cap before allocating.** `WebPDecoder::new` + `dimensions()` allocates no pixels; check
  `limits::image_pixels_within_cap` before any buffer. `set_memory_limit` is also set,
  but its docs admit gaps, so it is defence in depth and never the cap.
- **Straight (non-premultiplied) alpha**; RGB8 when the file has no alpha. The GTK side
  builds a `MemoryTexture` (`R8g8b8a8`), **never** `Texture::from_bytes`, which hands
  encoded bytes back to the leaking loader.
- **Animation compositing has two known gaps, both in `image-webp` itself** (researcher,
  source-read 0.2.4; the `image` wrapper adds no second compositor, so image#2913's locus is
  here):
  - **Dispose-to-background is a no-op by default.** `background_color` defaults to `None`,
    so the clear does nothing. Every reference renderer clears to **transparent** and treats
    ANIM `bgcolor` as an ignorable hint (libwebp `WebPAnimDecoder`'s `ZeroFillFrameRect`,
    Blink, Gecko, WebKit, `magick -coalesce`; researcher, corrected from an earlier "browsers
    fill `bgcolor`"). So `richimg` calls `set_background_color([0, 0, 0, 0])` —
    **never** `background_color_hint()`, which is also in on-disk BGRA order. Re-check on any
    bump: image-webp `main` already zeroes a disposed rect when the frame has alpha.
  - **`reset_animation` does not clear the canvas** (it only rewinds the ANMF pointer). A
    loop whose frame 0 is partial and transparent composites over the last frame, so
    `richimg` clears the canvas itself on every loop.
- **Fidelity evidence so far is weak for animation.** On `splash.webp` frame 0 is
  bit-exact, and frames 1–4 differ from `magick -coalesce` by at most **1 LSB** on 80–91% of
  pixels (YUV/blend rounding). But every `splash.webp` frame is `dispose=none`, so it
  cannot exhibit either gap: `richimg` needs its own fixtures for dispose-to-background and
  partial transparent frames.
- **Sequential `read_frame`** costs a median 9.0 ms per 900×670 frame (6.5–13.4); a full
  80-frame pass is 715 ms. `reset_animation` is free.

### ❌ Other decoders

- **zenwebp** — faster and bit-exact, but AGPL-3.0-only or commercial.
- **libwebp bindings** (`libwebp-sys`, `webp`, `webp-animation`, `webpx`) — add a C
  dependency on Windows and macOS and give up memory safety; libwebp's CVE-2023-4863
  (exploited heap overflow) is the class being left.
- **The `image` crate** — extra surface, and its WebP wrapper drops `set_memory_limit`
  (image#3077).
- **Young crates** (`webp-rust`, `gamut-webp`, `oxideav-webp`, …) — small user bases, no
  comparable fuzzing; `gamut-webp` has no animation.

### ✅ Route by content, through one choke point

WebP is recognised by its `RIFF`…`WEBP` magic, GIF by `GIF87a`/`GIF89a`, and APNG by the PNG
signature plus an `acTL` chunk before the first `IDAT` — never by file extension. **One**
application-side module decodes every encoded image and routes WebP, GIF and APNG to
`richimg`, everything else (a still PNG included) to GTK. It serves the four sites that decode today — preview local images
(`renderer/start.rs`), remote images (same file), theme sprites (`sprite.rs`, including its
`probe_pixel_size`) and PDF export (`export/pdf`) — and `export/doc.rs`'s existing WebP
sniff becomes `richimg`'s. **Enforcement:** a `clippy.toml` `disallowed-methods` ban on
GTK's encoded-image decode entry points (`Texture::from_bytes` / `from_file` /
`from_filename`, `Pixbuf::from_stream`, `PixbufLoader::new`), sanctioned only in the choke
point. The true-positive rate is high — every current caller is decoding untrusted content.

### ✅ A local image file is admitted like a document

Sniffing by content means the choke point reads the file itself, where GTK used to. So a
local image read gets the same two-part test documents get (POLICY § Input limits): **a
regular file** — a FIFO named `x.gif` would otherwise block the main thread forever — and
**within a byte limit**. Local images have no byte limit today; only remote ones do
(`limits::MAX_REMOTE_IMAGE_BYTES`). The limit is **configurable in `config.toml`**
(operator), with its default in `limits.rs`: **16 MiB**, as a sibling constant
`MAX_LOCAL_IMAGE_BYTES` with its own justification (researcher; not an alias of the remote
one). One limit for every format.

- **Evidence:** `splash.webp` is 7.0 MiB, so 16 MiB is 2.2× the project's own hero. GitHub
  caps pasted images and GIFs at 10 MB; X at 15 MB, Discord at 8 MB, Reddit at 20 MB.
  Browsers, `image`, gdk-pixbuf and glycin cap *decoded* pixels rather than file bytes,
  because they stream; we need a file cap only because we read the whole file to sniff it.
- **Why not higher:** 64 MiB is the document cap, and an image that large is a mis-attached
  video; nothing in the measured corpus needs 32.
- **Why not split still vs animated:** the pixel cap already bounds the canvas, and a
  decompression bomb is caught by `MAX_IMAGE_PIXELS`, not by this. This cap bounds the read
  and the compressed bytes that stay resident while an animation plays.

### ✅ GIF animates too

Animating WebP and leaving GIF static would be inconsistent (operator). Route: see the
`gif` crate decision below.

### ✅ Theme sprites animate too, and so do remote images

An animated sprite always plays (operator): whether a theme uses one is the theme
designer's call, and not the engine's to police. Sprites are painted by the preview's paint
plan (`decorplan.rs`), not by a picture widget, so their visibility comes from its existing
viewport gates, and they still obey the toggle and reduce-animations. Remote images animate
like local ones. PDF export always takes the first frame.

### ✅ Decode off the main thread; present on the frame clock

Operator decision. A frame decode is ~9 ms for a 900×670 WebP and grows with pixel count,
so a large, fast animation could take longer than a frame on the main thread. `richimg` has
no GTK in it, so its decoder can run on a worker and hand back **owned RGBA bytes**. That
is the shape POLICY § Architecture rules prefers (plain owned data crosses, no GTK object
does), and the texture is built and swapped on the main thread. **This is the application's
first worker of its own**, so TECH.md's concurrency model and that POLICY rule change with
it.

**Backpressure: fall behind by skipping, never by queueing.** At most one decode per
animation is in flight. When the clock has passed a frame's presentation time, that frame is
not shown late — playback jumps to the frame that is due now. ⚠ **Skipping a frame's
presentation does not skip its decode**: WebP, GIF and APNG frames composite onto the previous
canvas, so every intermediate frame must still be decoded to reach a later one, except
where a frame replaces the whole canvas. What backpressure saves is painting and texture
uploads, not decode work. The frame-delay floor below is what bounds that.

### ✅ Frame timing: a delay under 20 ms means 50 ms

A frame that declares a delay **under 20 ms** — 0 or 10 ms, since GIF counts in 10 ms
steps — is shown for **50 ms**, configurable in `config.toml` (operator). Browsers apply the
same kind of floor (≤10 ms becomes 100 ms). Without it, a zero-delay GIF redraws as fast as
the display allows, and given the decode note above a 10 ms GIF decodes 100 frames a
second whatever the display rate. An animation that has played its declared loop count
stops, and its tick callback is removed.

### ✅ Never animate what is not visible

**A hard requirement** (operator). Animation costs CPU on every frame — the decode on a
worker, and the texture swap and repaint on the main thread under the Cairo renderer — and
an application built for a small footprint must not spend it on pixels nobody can see. Not visible includes, at least: scrolled out of the preview viewport, a
background tab, a hidden preview pane (edit mode), a collapsed `<details>` body, and a
minimized or hidden window. A paused animation costs **zero** CPU — no timer, no tick
callback — not merely no repaint.

**How, per the researcher's GTK 4.6.9 source read** (a dark pattern; each claim is verified
by a test before it is relied on). **GTK pauses nothing for us**: `GtkPicture` only snapshots
its paintable, `GdkPixbufAnimationIter` is pull-based, and gtk-demo's own animated paintable
runs a `g_timeout_add` forever.

- **The tick callback is installed or it is not.** An installed callback that returns early
  still holds `gdk_frame_clock_begin_updating`, which keeps the whole toplevel's clock
  running at display rate — one forgotten off-screen GIF costs the window 60 fps of
  update/layout/paint. Pausing means `TickCallbackId::remove()`, and
  `has_tick_callback() == false` is the gate's oracle.
- **Never `glib::timeout_add`.** A timeout ignores mapping, viewport and frame-clock freeze.
- **Decode on schedule, not per vsync.** Request the next frame only as the clock's
  `frame_time` approaches its presentation time (`splash.webp`'s frame 0 lasts 2250 ms).

| Not visible because… | What GTK does | Signal / predicate |
|---|---|---|
| Scrolled out of view | Anchored children **stay mapped** and are still snapshotted; only the allocation moves | Intersect the child's `allocation()` with the view's (both widget space — not `visible_rect()`, which is buffer space and lags paint, GTK4Rs/AP-142). Re-test on h/v adjustment `value-changed` and `changed`, coalesced on one idle |
| Collapsed `<details>` | An invisible tag over U+FFFC does **not** unmap the child; it is parked at (−w,−h) (GTK4Rs/AP-166) | The same allocation test — a parked child fails it. `is_mapped()` is wrong here |
| Inside a `<details>` body (`widgets/disclosure.rs`) | The picture is nested in another widget, so its own allocation is relative to that parent, not the view | `compute_bounds(picture, view)`, then the same intersection; the parent's allocation changes re-run it |
| Background tab | Page gets `child-visible = false` → unmapped, still realized | `map` / `unmap`; `is_mapped()` |
| Hidden pane | `set_visible(false)` → unmapped | `map` / `unmap`; `is_mapped()` |
| Minimized window | `GdkToplevelState::MINIMIZED` on all three. Win32 also freezes the clock; X11 with a modern WM and Quartz do **not** | `notify::state` on the toplevel surface — never rely on the freeze |
| Hidden window | Unmapped | `map` / `unmap` |
| Covered by another window | No state and no signal at 4.6 | **Out of scope** — not observable |

The pause decision is the conjunction of all of them plus reduce-animations (below), re-run
from every one of those signals; the collapse lever is the `<details>` fold model.

### ✅ Control: a "Play Animations" toggle in the View menu

Operator decision. A stateful toggle beside "Show Unsafe Images" in the View menu, and
**not on the toolbar**, because animations in Markdown documents are rare. That is an
operator-granted exception to the command-surface CAM, recorded in CAM.md when it lands.
**No keyboard interaction**: no accelerator and no key on the animation itself. It is
**process-wide** (operator decision), unlike its per-tab sibling: one stateful `app.*`
`GAction` (POLICY § Architecture rules) that pauses and resumes every animation in every
window, and every window's View menu mirrors its state.

**On by default**, and **"reduce animations" wins over it**. Its state **survives a restart
in the session file** (UI state, beside the other window state; `config.toml` holds only the
hand-edited numbers). ⚠ The saved state is the *reader's choice*, never the effective
state: a new process re-reads `gtk-enable-animations` and re-applies the override, so a
system setting changed between runs takes effect, and the override is never saved as
though the reader had chosen it (operator).

### ❌ The media Play/Pause key

The operator's first choice, dropped because **the focused app does not receive it on any
shipping desktop**. Delivery, per the researcher (GTK 4.6.9 source + a measurement on a
private Xvfb):

| Desktop | Reaches the focused GTK window? | Why |
|---|---|---|
| X11 KDE, default shortcuts (the operator's) | **No** | `kglobalaccel` `XGrabKey`s Media Play on the root (`kglobalshortcutsrc` `[mediacontrol]`) and forwards it to an MPRIS player. Unconditional, whether or not a player is running |
| X11 GNOME | **No** | `gsd-media-keys` grabs it, including a hard-coded binding |
| Wayland GNOME / KDE | **No** | The compositor owns the key |
| X11 with Media Play unbound | **Yes** | Measured: `keyval=0x1008ff14` (`AudioPlay`), keycode 172 |
| macOS Quartz 4.22.4 | **No** | GDK does not translate `NSEventTypeSystemDefined` / `NX_KEYTYPE_PLAY` |
| Windows gvsbuild | **No** | GDK drops `WM_APPCOMMAND`, and `VK_MEDIA_PLAY_PAUSE` maps to `VoidSymbol` |

The accelerator spelling would be `AudioPlay` (hardware play/pause is `KEY_PLAYPAUSE` →
`XF86AudioPlay`); there is no `AudioPlayPause` keysym, and `AudioMedia` is the launch-player
key. Not bound even as a latent extra, since the operator ruled out keyboard interaction.

### ❌ Claiming the media key system-wide

MPRIS on Linux, `MPRemoteCommandCenter` on macOS, SMTC or `RegisterHotKey` on Windows. Each
makes Scribobulate the session's "now playing" target and takes Play/Pause from the user's
music player; `RegisterHotKey` is global even when unfocused. That would be a "we are a
media player" product decision, not a way to deliver a key.

### ❌ Other keyboard routes

Space/Enter on a focused animation, or a menu accelerator (the researcher's
recommendation). The operator ruled out keyboard interaction; the menu toggle is enough
for a rare feature.

### ✅ Default state: autoplay

Operator decision. An animation plays by itself whenever it is visible, the View-menu
toggle is on and "reduce animations" is off. The visibility rule is what bounds the cost.

### ❌ Start paused behind a "Play" overlay

Costs nothing until asked, but the operator chose autoplay.

### ✅ What a paused animation looks like: a corner "paused" badge

Operator: a paused animation carries **a typical pause overlay**, to the canonical idiom
the researcher found:

- **A state badge, not a play button.** A centered play triangle is the click-to-play
  idiom (GitHub's reduced-motion GIFs, YouTube, `GtkVideo`), and on an image that does not
  respond to a click it promises something it cannot do. So the glyph is
  `media-playback-pause-symbolic` (the state), not `media-playback-start-symbolic` (the
  action). Both ship in GTK's own icon set; verify by render, not `has_icon`
  (GTK4Rs/AP-48).
- **Bottom-end corner**, a 16 px symbolic icon in a small circular `.osd` well (~32 px), no
  scrim. It does not scale with the image, and is omitted when the image is under 48 px on a
  side, where it would cover the picture.
- **Paint-only**: drawn in the snapshot as an `IconPaintable`, not a child widget or button,
  and with no click handler. The View-menu toggle stays the only control.
- **Frame:** freeze on the current frame when paused mid-play (resetting to 0 would be a
  stop, and the glyph would lie); frame 0 if it never started.
- **Reduce-animations shows the same badge on frame 0**, so a reader can always tell a
  paused animation from a still image. Still images never carry it, and a playing
  animation shows nothing.

### ✅ Animation state: per picture, bounded by what is on screen

Operator decision. The image cache keeps holding finished `GdkTexture`s under its 32 MiB
budget, and a playing animation is not a cache entry:

- **Each on-screen animation owns its decoder and one working canvas.** The compressed file
  bytes are shared by reference among every picture showing the same file. Decoder state
  cannot be shared, because each copy plays at its own position.
- **Not visible means everything but the shared bytes is dropped**, and playback restarts
  from frame 0 when the picture is visible again. Resuming mid-loop would mean re-decoding
  every delta frame since the last full-canvas one, and full playback fidelity is beyond the
  scope of a Markdown editor and viewer (operator). ⚠ This is distinct from a **pause**,
  which freezes the current frame (the overlay decision above): leaving the screen restarts,
  the toggle freezes.
- **No separate budget.** Resident animation memory is bounded by the animations on screen,
  each by the file cap (16 MiB) plus one canvas (under `MAX_IMAGE_PIXELS`).
- **Lifetime:** re-render, live reload and tab close drop every animation's state, and
  6.7's finalization half extends to it.

### ❌ Resume where it left off

Needs the decoder kept alive, or a re-decode of every intermediate delta frame, for a
fidelity a Markdown viewer does not need.

### ✅ Honour "reduce animations"

Operator decision. When GTK's `gtk-enable-animations` is false, animations show their first
frame, have no tick callback installed, and stay paused until the setting changes.

### ✅ GIF decode route: the pure-Rust `gif` crate

Operator decision. `gif` (image-rs, 0.14.x, MIT OR Apache-2.0, fuzzed through `image`),
inside `richimg` behind the same bytes-in / RGBA8-frames-out API as WebP. The crate emits
raw frames without compositing, so the four disposal modes (Keep, Background, Previous,
Any) come from `gif-dispose` (kornelski, MIT/Apache, ~160 SLoC) or are folded into
`richimg`; the `image` crate's `gif.rs` is a readable spec.

### ✅ APNG animates too, through the pure-Rust `png` crate

Operator decision. An animated PNG is an ordinary PNG with extra chunks (`acTL`, `fcTL`,
`fdAT`), so anything that ignores them shows the default image — which is what GTK's PNG
decode does today. Every major browser and GitHub play it. `png` (image-rs, MIT OR
Apache-2.0) decodes the frames. As with GIF, compositing is ours: three dispose ops (none,
background, previous) and two blend ops (source, over). Two APNG-specific rules:

- **The default image may not be a frame.** When no `fcTL` precedes `IDAT`, the default image
  is the fallback for non-APNG viewers and is not part of the animation.
- **Delays are fractions** (`delay_num` / `delay_den`, where a zero denominator means 1/100 s),
  and the same under-20 ms floor applies.

Claims about `png`'s APNG coverage and fuzzing come from general knowledge, not a probe, so
they are verified with fixtures before the crate is relied on.

**❌ `GdkPixbufAnimation`** for GIF, on the researcher's findings:

- **GIF is a loader module, not built in** (meson's default built-ins are PNG and JPEG).
  Ubuntu ships `libpixbufloader-gif.so` and Homebrew stages it, but the Windows gvsbuild
  prefix was measured SVG-only — so GIF would play on two platforms of three, the WebP
  portability hole again.
- It pauses nothing; it is C parsing untrusted input on the main thread; and its
  iterator's leak behaviour is unmeasured.

### ✅ Frame residency: decode as it plays

Operator decision. Keep the decoder, the compressed bytes and one composited canvas, and
decode the next frame when it is due. Holding every frame resident would cost ~193 MB for
`splash.webp`, the size of the leak this plan exists to remove. Measured cost is ~9 ms per
900×670 frame, once per frame duration rather than per vsync.

### ❌ Hold every frame decoded

Fewer decodes, but memory scales with frame count — the ~193 MB above.

## Possible approaches — the gate

### ✅ A. A new test class with its own mandatory pipeline step (shipped)

Step 5b, TDD 6.6–6.8, POLICY § Per-render memory-growth class. Two halves catching disjoint
failures: **growth slope** after discarded warm-up (per-platform tolerances in
`memgate::footprint`, field named `footprint`, never `rss`) and **finalization** of the
decoded picture with no main-loop pump. Freed memory is not returned memory on any platform
we ship, so a single-shot "render, free, assert it came back" cannot pass on correct code;
slope over N with K warm-up renders discarded is the only honest shape.

The finalization half is sound only under Cairo. Measured (`probes/gsk-texture-ref-ownership.c`):

| Version / seat | `GskCairoRenderer` | `GskGLRenderer` |
|---|---|---|
| 4.22.4, `mac` | finalizes | finalizes |
| **4.6.9 floor**, `linux` | finalizes | **never finalizes** |

So the gate asserts the realized native's renderer is `GskCairoRenderer` — `$GSK_RENDERER`
is defeatable, and without that check GL gives a false red at the floor and a false green on
macOS. **Any proposal to let the renderer vary re-opens this and must re-measure first.**

### ❌ B. Fold into pipeline step 5

Operator's call: a separate step can be disabled independently.

### ❌ C. Absolute RSS ceiling

Fails legitimate large documents and passes a slow leak on a small one.

### Phase 2 gate additions (shipped)

- **Uncached decode slope** (TDD 6.9): decode the animated WebP repeatedly with the image
  cache emptied each time. It was written RED — ~1.05 MB per iteration on the committed
  fixture, ~12 MB on `splash.webp` — and turned green when the decode moved to `richimg`.
  Mutation-tested: routing the format back to GTK reddens it by ~10 MB per decode while a
  cold-cache PNG control stays flat.
- **Playback slope and scroll-cycle slope** (TDD 6.10), and 6.7's finalization half extended
  to the animation state.
- **The WebP skips are gone**: every host decodes WebP now, so a decoder-absent skip would
  be dead code hiding a failure.

## Technical details preserved

**Reproducer.** A document containing `<img src="assets/splash.webp">` where the path
resolves, driven through repeated re-renders. `assets/splash.webp` is 7.4 MB, 900×670, 80
frames, VP8X animation+transparency, ANIM `bgcolor` 0xFFFFFFFF, loop 0; frame 1 is
full-canvas, opaque, lossy, at (0,0), blend off, 2250 ms. Committed fixtures carry their own
small animated WebP (`tests/fixtures/anim.webp`) rather than depending on the README asset.

**Drive method.** `gdbus call --session -d com.extollit.scribobulate -o
/com/extollit/scribobulate -m org.gtk.Actions.SetState preview-theme "<'sepia'>" "{}"`.
Zoom uses `Activate` on `zoom-in`/`zoom-out` at `/com/extollit/scribobulate/window/1`.
Driving the GAction bypasses popovers, which kwin-on-Xvfb will not deliver a synthetic
click to (ScrAP-101).

**Measurement.** `VmRSS` from `/proc/<pid>/status`; per-mapping attribution from
`/proc/<pid>/smaps`. `VmHWM` equal to `VmRSS` means the process is at its peak and has
returned nothing. To separate retention from allocator slack, re-run with
`MALLOC_ARENA_MAX=1`, `MALLOC_TRIM_THRESHOLD_=65536`, `MALLOC_MMAP_THRESHOLD_=65536` — if
RSS barely moves, the memory is genuinely referenced.

**Harness.** `xvfb-run -a … dbus-run-session -- …` in that nesting, `GTK_USE_PORTAL=0`, a
private `XDG_STATE_HOME`, `GSK_RENDERER=cairo`. Identify the process by the PID captured at
launch, never by name (ScrAP-241).

**Traps already paid for, each a clean-looking false negative:**

- **`tests/fixtures/anim.webp` only changes in rows 96–236 of its 480×270 canvas.** A band
  decoration tiles the sprite from the document's own grid, so a band near the top of the
  document displays the sprite's STATIC top rows and looks frozen while it is animating
  perfectly. This produced a confident false FAIL of TDD 27.9 in a driven run, contradicted
  by a second run that happened to place the band over the changing rows. **Before believing
  a driven capture that shows no movement, prove the FIXTURE varies in the region being
  measured** — here, `magick anim.webp -coalesce -crop 480x140+0+96` makes a sprite whose
  whole area animates, and the same check then shows 7–24k pixels changing per capture.

- Deleting the `<img src="...gif">` line from README leaves `<source srcset="...webp">` live
  in the same `<picture>`, so the WebP still loads and the GIF looks guilty. Remove the whole
  element when isolating.
- Copying a document to `/tmp` breaks relative image paths, so nothing loads and the fixture
  measures nothing. Image fixtures must sit where their paths resolve.
- Three cycles cannot separate warm-up from a leak; the first is dominated by icon-cache and
  font loading.
- A single-window or single-document fixture without the image does not reproduce it, which
  sent the first investigation to a false "no leak".
