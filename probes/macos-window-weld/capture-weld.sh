#!/bin/bash
# Capture the live AppKit window graph of a running Scribobulate.
#
# Run this the MOMENT the two-window fault is on screen — before touching
# anything else. It attaches to the running process, reads every NSWindow's
# parent/child relationships, detaches, and writes a readable table.
#
# The app keeps running; lldb pauses it for a second or two and lets it go.
#
#   ./capture-weld.sh [output-file]

set -u
OUT="${1:-weld-capture-$(date +%Y%m%d-%H%M%S).txt}"
DIR="$(cd "$(dirname "$0")" && pwd)"
PID=$(pgrep -f "Scribobulate.app/Contents/MacOS" | head -1)

if [ -z "$PID" ]; then
  echo "Scribobulate is not running (looked for the .app bundle binary)." >&2
  exit 2
fi

SCRIPT="$(mktemp)"
RAW="$(mktemp)"
cat > "$SCRIPT" <<'EOF'
expr -l objc++ -O -- (id)[[(id)NSApp windows] valueForKey:@"windowNumber"]
expr -l objc++ -O -- (id)[[(id)NSApp windows] valueForKeyPath:@"className"]
expr -l objc++ -O -- (id)[[(id)NSApp windows] valueForKey:@"title"]
expr -l objc++ -O -- (id)[[(id)NSApp windows] valueForKey:@"visible"]
expr -l objc++ -O -- (id)[[(id)NSApp windows] valueForKey:@"miniaturized"]
expr -l objc++ -O -- (id)[[(id)NSApp windows] valueForKeyPath:@"parentWindow.windowNumber"]
expr -l objc++ -O -- (id)[[(id)NSApp windows] valueForKeyPath:@"childWindows.windowNumber"]
detach
quit
EOF

echo "attaching to pid $PID ..."
lldb -p "$PID" -b -s "$SCRIPT" > "$RAW" 2>&1
rm -f "$SCRIPT"

python3 - "$RAW" "$OUT" <<'PY'
import re, sys

raw = open(sys.argv[1], errors="replace").read()

# Each `expr` echoes its command, then prints an NSArray literal. Split on the
# echoed commands so a value can never be mistaken for a command.
blocks = re.split(r'\(lldb\) expr [^\n]*\n', raw)[1:]

def flat(block):
    """Values of a flat NSArray, in order, with <null> as None."""
    body = block.split('(', 1)[-1]
    body = body.rsplit(')', 1)[0]
    out = []
    for line in body.splitlines():
        s = line.strip().rstrip(',')
        if not s or s.startswith('<__NSArray') or s == ')' or s == '(':
            continue
        out.append(None if s == '<null>' else s)
    return out

def nested(block):
    """Values of an NSArray of NSArrays: one list of child numbers per window."""
    groups, cur = [], None
    for line in block.splitlines():
        s = line.strip()
        if s.startswith('<__NSArray0') or s.startswith('<__NSSingleObjectArray') or s.startswith('<__NSArrayI') or s.startswith('<__NSArrayM'):
            if cur is not None:
                groups.append(cur)
            cur = []
            continue
        s = s.rstrip(',')
        if cur is not None and re.fullmatch(r'-?\d+', s):
            cur.append(s)
    if cur is not None:
        groups.append(cur)
    return groups[1:] if groups else []

try:
    nums    = flat(blocks[0])
    classes = flat(blocks[1])
    titles  = blocks[2].split('(', 1)[-1].rsplit(')', 1)[0].splitlines()
    titles  = [t.strip().rstrip(',') for t in titles if t.strip() != '']
    visible = flat(blocks[3])
    mini    = flat(blocks[4])
    parents = flat(blocks[5])
    kids    = nested(blocks[6])
except Exception as e:
    open(sys.argv[2], 'w').write("PARSE FAILED: %s\n\nRAW:\n%s" % (e, raw))
    print("parse failed; raw lldb output written to", sys.argv[2])
    raise SystemExit(1)

n = len(nums)
def at(seq, i, default=''):
    return seq[i] if i < len(seq) and seq[i] is not None else default
while len(titles) < n:
    titles.append('')

lines = []
lines.append("Scribobulate AppKit window graph")
lines.append("=" * 78)
lines.append("%-9s %-18s %-4s %-5s %-8s %s" % ("win#", "class", "vis", "mini", "parent", "children"))
for i in range(n):
    lines.append("%-9s %-18s %-4s %-5s %-8s %s" % (
        at(nums, i), at(classes, i), at(visible, i), at(mini, i),
        at(parents, i, 'nil'),
        ",".join(kids[i]) if i < len(kids) and kids[i] else "-"))
    t = titles[i].strip()
    if t:
        lines.append("          title: %s" % t)

lines.append("")
lines.append("READING THIS")
lines.append("-" * 78)
lines.append("Each document window should have parent=nil. A document window that is")
lines.append("listed as another document window's child, or that shares a child with")
lines.append("another document window, is the weld: AppKit keeps an ordering group")
lines.append("together, so raising one raises the other and de-miniaturizes it.")
lines.append("An untitled GdkMacosWindow is a popup (tooltip, popover, menu).")
lines.append("A popup listed under a window it is NOT parented to is the missing")
lines.append("removeChildWindow: in _gdk_macos_popup_surface_attach_to_parent.")

# Flag anything suspect outright, so a reader does not have to eyeball it.
suspect = []
byindex = {at(nums, i): i for i in range(n)}
for i in range(n):
    for k in (kids[i] if i < len(kids) else []):
        j = byindex.get(k)
        if j is not None and at(parents, j, 'nil') != at(nums, i):
            suspect.append("win#%s lists child #%s, but #%s's parent is %s"
                           % (at(nums, i), k, k, at(parents, j, 'nil')))
    if at(classes, i) == 'GdkMacosWindow' and titles[i].strip() and at(parents, i) not in ('', None):
        suspect.append("TITLED window #%s has an AppKit parent (#%s) — document windows should not"
                       % (at(nums, i), at(parents, i)))

lines.append("")
lines.append("SUSPECT FINDINGS: " + ("none" if not suspect else ""))
for s in suspect:
    lines.append("  !! " + s)

open(sys.argv[2], 'w').write("\n".join(lines) + "\n")
print("\n".join(lines))
PY

rm -f "$RAW"
echo
echo "written to $OUT"
