#!/usr/bin/env bash
# Generates every richimg WebP test fixture and its reference frames, so
# `cargo test -p richimg` needs no image tool at run time (only at fixture
# generation time, i.e. when this script is re-run after a deliberate
# change). Re-running it is idempotent: it recreates every generated file
# from scratch in a throwaway temp directory and only then copies the
# results over the committed ones.
#
# Tools used: img2webp, webpmux, cwebp, magick, python3 (hand-built chunk
# crafting), curl (fetching the pinned upstream #182 reproducer — see that
# section below for what happens when the network is unavailable).
set -euo pipefail

FIXTURES_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REFS_DIR="$FIXTURES_DIR/refs"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$REFS_DIR"
cd "$WORK"

# ---------------------------------------------------------------------------
# Base source images (small, solid or semi-transparent color blocks). Kept
# tiny per the fixture size rule (<=64x64, <=8 frames) — everything here is
# 16x16 or an 8x8/8x16 partial region.
# ---------------------------------------------------------------------------
magick -size 16x16 xc:red red.png
magick -size 16x16 xc:lime green.png
magick -size 16x16 xc:blue blue.png
magick -size 8x16 xc:lime green_left.png
magick -size 16x16 xc:red -alpha set -channel A -evaluate set 50% +channel alpha_red.png
magick -size 8x8 xc:blue full_blue.png
magick -size 16x16 xc:blue full_blue16.png
magick -size 8x8 xc:lime -alpha set -channel A -evaluate set 50% +channel green_half_alpha.png
magick -size 8x8 xc:red -alpha set -channel A -evaluate set 50% +channel red_half_alpha.png
magick -size 8x8 xc:red loop_a.png
magick -size 8x8 xc:lime loop_b.png
magick -size 16x8 xc:blue loop_c.png

# ---------------------------------------------------------------------------
# 1-3: still images (lossy, lossless, alpha)
# ---------------------------------------------------------------------------
cwebp -quiet -q 80 red.png -o lossy_still.webp
cwebp -quiet -lossless green.png -o lossless_still.webp
cwebp -quiet -lossless alpha_red.png -o alpha_still.webp

# ---------------------------------------------------------------------------
# 4: animated, full-canvas frames only (no partial regions, no disposal)
# ---------------------------------------------------------------------------
img2webp -loop 3 -d 100 red.png -d 100 green.png -d 100 blue.png -o animated_full_canvas.webp

# ---------------------------------------------------------------------------
# 4b: the delay floor. Declared delays of 0ms and 10ms (both under the fixed
# 20ms SHORT_DELAY_THRESHOLD) must come out as `short_delay_substitute`; a
# declared 20ms must pass through unchanged. Built with webpmux rather than
# img2webp: img2webp rejects a literal "-d 0" ("Invalid negative duration").
# ---------------------------------------------------------------------------
cwebp -quiet -lossless red.png -o delay0_red.webp
cwebp -quiet -lossless green.png -o delay10_green.webp
cwebp -quiet -lossless blue.png -o delay20_blue.webp
webpmux -frame delay0_red.webp +0+0+0+0-b \
        -frame delay10_green.webp +10+0+0+0-b \
        -frame delay20_blue.webp +20+0+0+0-b \
        -loop 1 -o short_delays.webp

# ---------------------------------------------------------------------------
# 5: dispose-to-background. Frame 0 is a full, opaque canvas (red);
# dispose=1 (background) means it is cleared before frame 1 draws. Frame 1
# only covers the left half, so the right half must come from the ANIM
# background color after disposal. bgcolor is deliberately A,R,G,B =
# 255,200,100,50 — R, G and B are all distinct so a channel swap (e.g.
# BGRA/RGBA confusion) is visible rather than accidentally symmetric.
# ---------------------------------------------------------------------------
cwebp -quiet -lossless red.png -o dispose_red.webp
cwebp -quiet -lossless green_left.png -o dispose_green_left.webp
webpmux -frame dispose_red.webp +100+0+0+1-b \
        -frame dispose_green_left.webp +100+0+0+0-b \
        -loop 0 -bgcolor 255,200,100,50 \
        -o dispose_to_background.webp
# Disposal is the only source of transparency here, so webpmux leaves the VP8X
# alpha flag clear — and image-webp 0.2.4 then emits RGB, dropping the cleared
# rect's alpha. An encoder that produces transparent disposal (libwebp's
# WebPAnimEncoder) sets the flag; set it (byte 20, bit 0x10) so the fixture is
# the file a real encoder writes.
python3 - dispose_to_background.webp <<'PY'
import sys
path = sys.argv[1]
data = bytearray(open(path, "rb").read())
assert data[12:16] == b"VP8X", "expected a VP8X extended header"
data[20] |= 0x10
open(path, "wb").write(bytes(data))
PY

# ---------------------------------------------------------------------------
# 6: partial transparent frames, exercising both blend=on and blend=off.
# Frame 0: full opaque blue canvas. Frame 1: an 8x8 semi-transparent green
# square at (4,4), blended over the blue (webpmux "+b"). Frame 2: the same
# region, semi-transparent red, blend OFF ("-b") — a straight overwrite that
# must keep its own straight (non-premultiplied) alpha rather than blending
# with what was underneath.
# ---------------------------------------------------------------------------
cwebp -quiet -lossless full_blue16.png -o partial_base_blue.webp
cwebp -quiet -lossless green_half_alpha.png -o partial_green_alpha.webp
cwebp -quiet -lossless red_half_alpha.png -o partial_red_alpha.webp
webpmux -frame partial_base_blue.webp +100+0+0+0-b \
        -frame partial_green_alpha.webp +100+4+4+0+b \
        -frame partial_red_alpha.webp +100+4+4+0-b \
        -loop 1 -bgcolor 255,255,255,255 \
        -o partial_transparent.webp

# ---------------------------------------------------------------------------
# 7: a loop whose frame 0 is partial, so a missing canvas clear on rewind is
# visible. Three DISJOINT partial opaque regions (top-left, top-right,
# bottom half) with dispose=none throughout and a fully transparent ANIM
# background (0,0,0,0). Decoding frame 0 a second time (after the wrap) must
# reproduce the exact same bytes as the first decode of frame 0 — under the
# image-webp 0.2.4 defect where `reset_animation` does not clear the canvas,
# the wrap only clears the LAST frame's rectangle (bottom half) rather than
# every previously-drawn region, so frame 1's top-right square would still
# be showing when frame 0 is re-decoded.
# ---------------------------------------------------------------------------
cwebp -quiet -lossless loop_a.png -o loop_a.webp
cwebp -quiet -lossless loop_b.png -o loop_b.webp
cwebp -quiet -lossless loop_c.png -o loop_c.webp
webpmux -frame loop_a.webp +100+0+0+0-b \
        -frame loop_b.webp +100+8+0+0-b \
        -frame loop_c.webp +100+0+8+0-b \
        -loop 0 -bgcolor 0,0,0,0 \
        -o partial_frame0_loop.webp

# ---------------------------------------------------------------------------
# 8: truncated file — a syntactically-recognisable WebP (RIFF/WEBP magic
# intact, so `sniff` still says WebP) whose frame data is cut off partway
# through. Cutting at HALF the file (94 of 188 bytes) turned out to still
# leave a complete, decodable frame 0 behind — not actually a decode
# failure — so this keeps just enough for the RIFF/VP8X/ANIM headers (60
# bytes) and nothing of any frame's pixel data, which measurably fails
# `probe`, `Animation::new` and `first_frame` alike with `Error::Malformed`.
# ---------------------------------------------------------------------------
TRUNCATED_LEN=60
python3 - "$WORK/animated_full_canvas.webp" "$WORK/truncated.webp" "$TRUNCATED_LEN" <<'PYEOF'
import sys
src, dst, cut = sys.argv[1], sys.argv[2], int(sys.argv[3])
data = open(src, "rb").read()
with open(dst, "wb") as f:
    f.write(data[:cut])
PYEOF

# ---------------------------------------------------------------------------
# 9: image-webp#182 crafted reproducer — a zero-sized (0x0) VP8 bitstream
# inside an ANMF chunk that also carries an ALPH chunk. The ALPH-present
# branch of `WebPDecoder::read_frame` (0.2.4) omits the width/height
# consistency check the plain-VP8 branch has, so `Frame::fill_rgba` panics
# with an out-of-bounds slice index when the VP8 frame's own decoded
# dimensions (0x0) disagree with the ANMF's declared frame size.
#
# This is the exact file attached to the upstream report
# (image-rs/image-webp#182, "Panic with zero sized vp8 data inside ANMF
# following ALPH"), fetched from its GitHub attachment URL rather than
# hand-crafted, so the reproducer is byte-identical to the one that found
# the bug. If the network is unavailable, keep whatever is already
# committed at crafted_182.webp instead of failing generation outright —
# the file rarely needs to change once pinned.
# ---------------------------------------------------------------------------
CRAFTED_182_URL="https://github.com/user-attachments/assets/89a21550-8505-4df6-a47f-8174d4950e87"
# Pinned by content: an attachment URL can be re-pointed, and a changed file
# would silently stop being the reproducer the test claims it is.
CRAFTED_182_SHA256="6163f8e987f7613874f7bcc814cc94767596388193d3c0fc21ea7284f7586e43"
if curl -fsSL -o crafted_182.webp "$CRAFTED_182_URL"; then
    if ! echo "$CRAFTED_182_SHA256  crafted_182.webp" | sha256sum -c --quiet -; then
        echo "make.sh: $CRAFTED_182_URL no longer serves the pinned reproducer (sha256 mismatch)" >&2
        exit 1
    fi
elif [ -f "$FIXTURES_DIR/crafted_182.webp" ]; then
    echo "make.sh: could not fetch $CRAFTED_182_URL, keeping the already-committed crafted_182.webp" >&2
    cp "$FIXTURES_DIR/crafted_182.webp" crafted_182.webp
else
    echo "make.sh: could not fetch $CRAFTED_182_URL and no committed crafted_182.webp exists" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# 10: VP8X header claiming a canvas above the pixel cap (16384x16384), with
# no actual pixel data — richimg's central pixel-cap check must refuse this
# before ever allocating a canvas. Hand-built byte-for-byte in Python
# because no encoder tool will deliberately emit an oversized declared
# canvas with no matching frame data.
# ---------------------------------------------------------------------------
python3 - "$WORK/oversized_canvas.webp" <<'PYEOF'
import struct
import sys

CANVAS_SIDE = 16384  # claimed canvas width/height, 4x richimg's default cap per side

def le3(value):
    """3-byte little-endian, as WebP's VP8X/ANMF header fields use."""
    return struct.pack("<I", value)[:3]

vp8x_body = bytes([0x00]) + b"\x00\x00\x00" + le3(CANVAS_SIDE - 1) + le3(CANVAS_SIDE - 1)
assert len(vp8x_body) == 10

vp8x_chunk = b"VP8X" + struct.pack("<I", len(vp8x_body)) + vp8x_body
# A VP8L chunk header with no body at all: WebPDecoder only records the
# chunk's byte range while walking VP8X's sub-chunks at construction time,
# it never reads VP8L's own bytes until a frame is actually decoded, which
# richimg's pixel-cap check must prevent from happening here.
vp8l_stub = b"VP8L" + struct.pack("<I", 0)

body = vp8x_chunk + vp8l_stub
riff = b"RIFF" + struct.pack("<I", len(body) + len(b"WEBP")) + b"WEBP" + body

with open(sys.argv[1], "wb") as f:
    f.write(riff)
PYEOF

# ---------------------------------------------------------------------------
# Reference frames: raw straight-alpha RGBA8, one file per frame, decoded by
# `magick -coalesce` (a decoder independent of image-webp). Stills get a
# single reference frame from the source PNG directly (no compositing to
# coalesce). The dispose-to-background fixture is a deliberate, documented
# exception — see richimg/src/webp.rs's dispose-to-background test module
# doc comment for why its frame 1 is only PARTLY checked against magick.
# ---------------------------------------------------------------------------
write_refs() {
    local webp_file="$1"
    local stem="$2"
    magick "$webp_file" -coalesce "${stem}-%d.png"
    local i=0
    while [ -f "${stem}-${i}.png" ]; do
        # -depth 8 is load-bearing: without it, magick writes raw samples at
        # whatever bit depth it decided the (solid-color, few-value) PNG
        # needs, which for these fixtures is 1 bit/channel rather than 8 —
        # a silently 8x-too-small file that still "succeeds".
        magick "${stem}-${i}.png" -depth 8 "RGBA:${REFS_DIR}/${stem}-${i}.rgba"
        i=$((i + 1))
    done
}

magick alpha_red.png -depth 8 "RGBA:${REFS_DIR}/alpha_still-0.rgba"
magick red.png -depth 8 "RGBA:${REFS_DIR}/lossy_still-0.rgba"
magick green.png -depth 8 "RGBA:${REFS_DIR}/lossless_still-0.rgba"

write_refs animated_full_canvas.webp animated_full_canvas
write_refs short_delays.webp short_delays
write_refs dispose_to_background.webp dispose_to_background
write_refs partial_transparent.webp partial_transparent
write_refs partial_frame0_loop.webp partial_frame0_loop

# ---------------------------------------------------------------------------
# Install the generated fixtures over the committed ones.
# ---------------------------------------------------------------------------
for name in lossy_still lossless_still alpha_still animated_full_canvas \
            short_delays dispose_to_background partial_transparent \
            partial_frame0_loop truncated crafted_182 oversized_canvas; do
    cp "$WORK/$name.webp" "$FIXTURES_DIR/$name.webp"
done

echo "make.sh: fixtures and reference frames written to $FIXTURES_DIR"
