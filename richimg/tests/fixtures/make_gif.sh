#!/usr/bin/env bash
# Generates every richimg GIF test fixture and its reference frames, so
# `cargo test -p richimg` needs no image tool at run time (only at fixture
# generation time, i.e. when this script is re-run after a deliberate
# change). Re-running it is idempotent: it recreates every generated file
# from scratch in a throwaway temp directory and only then copies the
# results over the committed ones. Mirrors `make.sh`'s scheme for the
# WebP fixtures, one directory over: `gif/*.gif` and `gif/refs/*.rgba`.
#
# Tools used: python3 (hand-built GIF89a + LZW encoding — gifsicle is not
# available in this environment, and no other tool here gives per-frame
# control over disposal method, transparent index and a partly-off-canvas
# frame rect), magick (independent reference decode via `-coalesce`).
set -euo pipefail

FIXTURES_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GIF_DIR="$FIXTURES_DIR/gif"
REFS_DIR="$GIF_DIR/refs"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$GIF_DIR" "$REFS_DIR"
cd "$WORK"

# ---------------------------------------------------------------------------
# Hand-built GIF89a + LZW encoder. Every fixture below needs an exact,
# individually-chosen disposal method, delay, transparent index and frame
# rect per frame — no encoder tool here (`magick`, `ffmpeg`) exposes that
# combination through its CLI without also running its own frame-diffing
# optimizer, which would fight the deliberately exact rects these tests
# need. The LZW encoder never looks up a multi-symbol dictionary entry (it
# always emits a fresh base code per pixel: no compression), but it MUST
# still track the dictionary-growth schedule (`next_code`/`code_size`) a
# real encoder/decoder pair would, one entry per pixel after the first
# following a Clear code — otherwise a standards-compliant decoder's code
# width desyncs from the byte stream and every pixel past the first
# boundary decodes to garbage. (This was verified against both `magick
# -coalesce` and Pillow while developing this script; an earlier, simpler
# draft without the growth bookkeeping corrupted every fixture with more
# than ~7 pixels.)
python3 - << 'PYEOF'
import struct

PALETTE = [
    (0, 0, 0),        # 0: unused/transparent placeholder
    (255, 0, 0),      # 1: red
    (0, 255, 0),      # 2: green
    (0, 0, 255),      # 3: blue
    (255, 255, 255),  # 4: white
]
MIN_CODE_SIZE = max(2, (len(PALETTE) - 1).bit_length())
CLEAR_CODE = 1 << MIN_CODE_SIZE


def lzw_encode(indices, min_code_size):
    clear_code = 1 << min_code_size
    end_code = clear_code + 1
    max_code_value = 4095
    code_size = min_code_size + 1
    next_code = end_code + 1
    bits = []

    def emit(code, size):
        for i in range(size):
            bits.append((code >> i) & 1)

    emit(clear_code, code_size)
    prev = None
    for idx in indices:
        emit(idx, code_size)
        if prev is not None:
            if next_code <= max_code_value:
                next_code += 1
                if next_code == (1 << code_size) and code_size < 12:
                    code_size += 1
            else:
                emit(clear_code, code_size)
                code_size = min_code_size + 1
                next_code = end_code + 1
        prev = idx
    emit(end_code, code_size)

    while len(bits) % 8:
        bits.append(0)
    out = bytearray()
    for i in range(0, len(bits), 8):
        byte = 0
        for b in range(8):
            byte |= bits[i + b] << b
        out.append(byte)
    return bytes(out)


def chunk_subblocks(data):
    out = bytearray()
    for i in range(0, len(data), 255):
        chunk = data[i:i + 255]
        out.append(len(chunk))
        out.extend(chunk)
    out.append(0)
    return bytes(out)


def build_gif(canvas_w, canvas_h, bg_index, frames, loop_raw=None):
    """frames: dicts with left/top/width/height/indices/dispose/delay, and
    optionally `transparent` (a palette index). `loop_raw` is the raw
    NETSCAPE2.0 count (None omits the block entirely; 0 means infinite)."""
    n = max((len(PALETTE) - 1).bit_length(), 1)
    gct_size = 1 << n
    padded_palette = PALETTE + [(0, 0, 0)] * (gct_size - len(PALETTE))

    out = bytearray()
    out += b"GIF89a"
    out += struct.pack("<HH", canvas_w, canvas_h)
    out.append(0x80 | (0x7 << 4) | (n - 1))
    out.append(bg_index)
    out.append(0)
    for (r, g, b) in padded_palette:
        out += bytes([r, g, b])

    if loop_raw is not None:
        out += b"\x21\xff\x0b" + b"NETSCAPE2.0" + b"\x03\x01" + struct.pack("<H", loop_raw) + b"\x00"

    for f in frames:
        transparent = f.get("transparent")
        packed_gce = (f["dispose"] & 0x7) << 2
        if transparent is not None:
            packed_gce |= 0x01
        out += b"\x21\xf9\x04"
        out.append(packed_gce)
        out += struct.pack("<H", f["delay"])
        out.append(transparent if transparent is not None else 0)
        out.append(0)

        out += b"\x2c"
        out += struct.pack("<HHHH", f["left"], f["top"], f["width"], f["height"])
        out.append(0x00)
        out.append(MIN_CODE_SIZE)
        indices = f["indices"]
        assert len(indices) == f["width"] * f["height"], (len(indices), f["width"], f["height"])
        assert all(i < CLEAR_CODE for i in indices)
        out += chunk_subblocks(lzw_encode(indices, MIN_CODE_SIZE))

    out += b"\x3b"
    return bytes(out)


def solid(color_index, width, height):
    return [color_index] * (width * height)


CANVAS = 8
RED, GREEN, BLUE, WHITE = 1, 2, 3, 4
DELAY_0MS, DELAY_10MS, DELAY_20MS = 0, 1, 2
ANY, KEEP, BACKGROUND, PREVIOUS = 0, 1, 2, 3

# ---------------------------------------------------------------------------
# 1: dispose Any (0) then Keep (1). Both are the "leave the frame" no-op in
# richimg's compositing, so one fixture exercises both raw codes: frame 0's
# disposal (Any) governs the gap before frame 1 draws, frame 1's (Keep) the
# gap before frame 2. Base covers the full canvas so there is no
# never-drawn region for `magick -coalesce` to disagree about.
# ---------------------------------------------------------------------------
open("dispose_any_and_keep.gif", "wb").write(build_gif(
    CANVAS, CANVAS, bg_index=0, loop_raw=0,
    frames=[
        {"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(RED, 8, 8), "dispose": ANY, "delay": DELAY_10MS},
        {"left": 0, "top": 0, "width": 4, "height": 4, "indices": solid(GREEN, 4, 4), "dispose": KEEP, "delay": DELAY_10MS},
        {"left": 4, "top": 4, "width": 4, "height": 4, "indices": solid(BLUE, 4, 4), "dispose": KEEP, "delay": DELAY_10MS},
    ],
))

# ---------------------------------------------------------------------------
# 2: dispose Background (2). Frame 0 is the full opaque canvas so disposing
# it clears the WHOLE canvas to transparent before frame 1 draws its own
# small square — proving the clear is to transparent, not to the (deliberate,
# distinct-channel) logical-screen background colour index 1 (red).
# ---------------------------------------------------------------------------
open("dispose_background.gif", "wb").write(build_gif(
    CANVAS, CANVAS, bg_index=RED, loop_raw=0,
    frames=[
        {"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(RED, 8, 8), "dispose": BACKGROUND, "delay": DELAY_10MS},
        {"left": 0, "top": 0, "width": 4, "height": 4, "indices": solid(GREEN, 4, 4), "dispose": KEEP, "delay": DELAY_10MS},
    ],
))

# ---------------------------------------------------------------------------
# 3: dispose Previous (3), mid-animation. Frame 0 (Keep) is the persistent
# full-canvas red backdrop; frame 1 (Previous) draws a green square that
# must vanish — reverting to the backdrop, not staying green — before
# frame 2 draws its own, disjoint square.
# ---------------------------------------------------------------------------
open("dispose_previous.gif", "wb").write(build_gif(
    CANVAS, CANVAS, bg_index=0, loop_raw=0,
    frames=[
        {"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(RED, 8, 8), "dispose": KEEP, "delay": DELAY_10MS},
        {"left": 0, "top": 0, "width": 4, "height": 4, "indices": solid(GREEN, 4, 4), "dispose": PREVIOUS, "delay": DELAY_10MS},
        {"left": 4, "top": 4, "width": 4, "height": 4, "indices": solid(BLUE, 4, 4), "dispose": KEEP, "delay": DELAY_10MS},
    ],
))

# ---------------------------------------------------------------------------
# 4: dispose Previous on FRAME 0 SPECIFICALLY — the pinned "invalid; treat as
# clear-to-transparent" case (no state exists before the very first frame).
# Frame 0 covers the full canvas so there is no never-drawn region. Frame 0
# declares (but never itself uses) transparent index 0: `magick -coalesce`
# only serialises a cleared/reverted region as alpha 0 in its output when
# the source file declares transparency SOMEWHERE (verified empirically
# while developing this script — unlike `Background` disposal, which
# `magick` always renders as transparent regardless); without this the
# reference for frame 1's wiped region comes out opaque black instead of
# transparent, which would be `magick` failing to express the state rather
# than richimg disagreeing with it.
# ---------------------------------------------------------------------------
open("dispose_previous_frame0.gif", "wb").write(build_gif(
    CANVAS, CANVAS, bg_index=0, loop_raw=0,
    frames=[
        {"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(RED, 8, 8), "dispose": PREVIOUS, "delay": DELAY_10MS, "transparent": 0},
        {"left": 0, "top": 0, "width": 4, "height": 4, "indices": solid(GREEN, 4, 4), "dispose": KEEP, "delay": DELAY_10MS},
    ],
))

# ---------------------------------------------------------------------------
# 5: transparency. Frame 0 is a full opaque blue canvas (dispose Keep).
# Frame 1 overlays a 4x4 checkerboard of green/transparent-index (0) at
# (2,2): the transparent squares must show the blue underneath, not the
# green and not black.
# ---------------------------------------------------------------------------
_checker = [0, GREEN, 0, GREEN, GREEN, 0, GREEN, 0, 0, GREEN, 0, GREEN, GREEN, 0, GREEN, 0]
open("transparency.gif", "wb").write(build_gif(
    CANVAS, CANVAS, bg_index=0, loop_raw=0,
    frames=[
        {"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(BLUE, 8, 8), "dispose": KEEP, "delay": DELAY_10MS},
        {"left": 2, "top": 2, "width": 4, "height": 4, "indices": _checker, "dispose": KEEP, "delay": DELAY_10MS, "transparent": 0},
    ],
))

# ---------------------------------------------------------------------------
# 6/7/8: loop count mapping, all three cases. Two full-canvas frames each;
# only the NETSCAPE2.0 block (or its absence) differs.
# ---------------------------------------------------------------------------
_loop_frames = [
    {"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(RED, 8, 8), "dispose": KEEP, "delay": DELAY_10MS},
    {"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(GREEN, 8, 8), "dispose": KEEP, "delay": DELAY_10MS},
]
open("loop_finite.gif", "wb").write(build_gif(CANVAS, CANVAS, bg_index=0, loop_raw=4, frames=_loop_frames))
open("loop_infinite.gif", "wb").write(build_gif(CANVAS, CANVAS, bg_index=0, loop_raw=0, frames=_loop_frames))
open("loop_none.gif", "wb").write(build_gif(CANVAS, CANVAS, bg_index=0, loop_raw=None, frames=_loop_frames))

# ---------------------------------------------------------------------------
# 9: the delay floor. Declared 0ms and 10ms (both under the fixed 20ms
# SHORT_DELAY_THRESHOLD) must come out as `short_delay_substitute`; a
# declared 20ms passes through unchanged.
# ---------------------------------------------------------------------------
open("delay_floor.gif", "wb").write(build_gif(
    CANVAS, CANVAS, bg_index=0, loop_raw=0,
    frames=[
        {"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(RED, 8, 8), "dispose": KEEP, "delay": DELAY_0MS},
        {"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(GREEN, 8, 8), "dispose": KEEP, "delay": DELAY_10MS},
        {"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(BLUE, 8, 8), "dispose": KEEP, "delay": DELAY_20MS},
    ],
))

# ---------------------------------------------------------------------------
# 10: a still (single-frame) GIF.
# ---------------------------------------------------------------------------
open("still.gif", "wb").write(build_gif(
    CANVAS, CANVAS, bg_index=0, loop_raw=None,
    frames=[{"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(RED, 8, 8), "dispose": KEEP, "delay": DELAY_10MS}],
))

# ---------------------------------------------------------------------------
# 11: a frame rect partly outside the logical screen. Frame 0 covers the
# whole canvas (white) so there is no never-drawn region; frame 1's rect
# (6,6,4,4) on an 8x8 canvas extends 2px past both the right and bottom
# edges and must be clipped, never panic.
# ---------------------------------------------------------------------------
open("partial_offscreen.gif", "wb").write(build_gif(
    CANVAS, CANVAS, bg_index=0, loop_raw=0,
    frames=[
        {"left": 0, "top": 0, "width": 8, "height": 8, "indices": solid(WHITE, 8, 8), "dispose": KEEP, "delay": DELAY_10MS},
        {"left": 6, "top": 6, "width": 4, "height": 4, "indices": solid(RED, 4, 4), "dispose": KEEP, "delay": DELAY_10MS},
    ],
))

# ---------------------------------------------------------------------------
# 12: an over-cap logical screen (16384x16384) with NO frame data at all —
# richimg's central pixel-cap check must refuse this before ever allocating
# a canvas, and `probe` must read the (bogus) dimensions without allocating
# anything canvas-sized. `loop_raw=0` (a NETSCAPE2.0 block) is present only
# because the `gif` 0.14.2 crate's `read_info` needs at least one byte of
# lookahead past the trailer to resolve a FRAME-less file's `HeaderEnd`
# transition — with the trailer as the literal last byte (spec-conformant,
# and what every OTHER fixture here ends with too) it returns
# `DecodingError::UnexpectedEof` even though the header itself is
# well-formed; verified empirically while developing this script. A trivial
# extension block is more natural filler than a stray trailing byte and
# supplies exactly that lookahead.
# ---------------------------------------------------------------------------
open("oversized_canvas.gif", "wb").write(build_gif(16384, 16384, bg_index=0, loop_raw=0, frames=[]))

# ---------------------------------------------------------------------------
# 13: truncated file. A normal two-frame GIF (header, GCT and NETSCAPE block
# intact) cut off partway through frame 0's LZW data — still recognisable as
# GIF by content (`sniff` needs only the 6-byte magic), but undecodable.
# ---------------------------------------------------------------------------
_full = build_gif(CANVAS, CANVAS, bg_index=0, loop_raw=0, frames=_loop_frames)
_cut = len(_full) - 10  # inside frame 0's LZW sub-blocks; well past the header
assert _cut > 40, "truncation point should still be well past the header"
open("truncated.gif", "wb").write(_full[:_cut])
PYEOF

# ---------------------------------------------------------------------------
# Reference frames: raw straight-alpha RGBA8, one file per frame, decoded by
# `magick -coalesce` (a decoder independent of the `gif` crate and of this
# script's own hand-rolled LZW encoder). Every animated fixture built above
# with real, fully-canvas-covered content gets one; `oversized_canvas.gif`
# (no frame data) and `truncated.gif` (deliberately undecodable) do not.
# ---------------------------------------------------------------------------
write_refs() {
    local gif_file="$1"
    local stem="$2"
    magick "$gif_file" -coalesce "${stem}-%d.png"
    local i=0
    while [ -f "${stem}-${i}.png" ]; do
        # -depth 8 is load-bearing: see make.sh's identical comment — without
        # it, magick may write raw samples at a smaller-than-8-bit depth for
        # these solid-color images.
        magick "${stem}-${i}.png" -depth 8 "RGBA:${REFS_DIR}/${stem}-${i}.rgba"
        i=$((i + 1))
    done
}

for name in dispose_any_and_keep dispose_background dispose_previous \
            dispose_previous_frame0 transparency loop_finite loop_infinite \
            loop_none delay_floor still partial_offscreen; do
    write_refs "$name.gif" "$name"
done

# ---------------------------------------------------------------------------
# Install the generated fixtures over the committed ones.
# ---------------------------------------------------------------------------
for name in dispose_any_and_keep dispose_background dispose_previous \
            dispose_previous_frame0 transparency loop_finite loop_infinite \
            loop_none delay_floor still partial_offscreen oversized_canvas \
            truncated; do
    cp "$WORK/$name.gif" "$GIF_DIR/$name.gif"
done

echo "make_gif.sh: fixtures and reference frames written to $GIF_DIR"
