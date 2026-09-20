#!/bin/bash
# Capture the live AppKit window graph of a running Scribobulate.
#
# Run this the MOMENT the two-window fault is on screen — before touching
# anything else. It attaches to the running process, reads every NSWindow's
# parent/child relationships, detaches, and writes a readable table.
#
# The app keeps running; lldb pauses it for a second or two and lets it go.
#
#   ./capture-weld.sh [--pid N] [output-file]
#
# THIS SCRIPT REPORTS FAILURE. Earlier it ran under `set -u` alone, ignored
# lldb's exit status and ignored the Python block's, so an attach that was
# refused and a parse that threw both ended with "written to <file>" and exit 0.
# A probe used to decide whether a fault is present must never answer "captured"
# when it captured nothing: every exit below is 0 only if a table was written.

set -u
DIR="$(cd "$(dirname "$0")" && pwd)"

PID=""
OUT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --pid)
      [ $# -ge 2 ] || { echo "capture-weld: --pid needs a value" >&2; exit 2; }
      PID="$2"
      shift 2
      ;;
    --pid=*) PID="${1#--pid=}"; shift ;;
    -h|--help)
      echo "usage: $0 [--pid N] [output-file]"
      exit 0
      ;;
    -*) echo "capture-weld: unknown option '$1'" >&2; exit 2 ;;
    *)
      [ -z "$OUT" ] || { echo "capture-weld: more than one output file given" >&2; exit 2; }
      OUT="$1"
      shift
      ;;
  esac
done
OUT="${OUT:-weld-capture-$(date +%Y%m%d-%H%M%S).txt}"

# AMBIGUITY IS REFUSED, NOT RESOLVED. This used to be `pgrep ... | head -1`,
# which picks whichever process pgrep listed first. The fault being investigated
# is two WINDOWS in one process — the app is one process, many windows — so a
# second matching process is not a detail to silently discard, it is either a
# stale copy left from an earlier attempt or a single-instance failure, and
# either way attaching to the wrong one produces a clean window graph that
# reads as "the fault is absent". Name the one you mean with --pid.
if [ -z "$PID" ]; then
  ALL_PIDS=$(pgrep -f "Scribobulate.app/Contents/MacOS" || true)

  if [ -z "$ALL_PIDS" ]; then
    echo "Scribobulate is not running (looked for the .app bundle binary)." >&2
    exit 2
  fi

  COUNT=$(printf '%s\n' "$ALL_PIDS" | wc -l | tr -d ' ')
  if [ "$COUNT" -gt 1 ]; then
    echo "capture-weld: $COUNT processes match the .app bundle binary:" >&2
    while read -r p; do
      echo "    pid $p    $(ps -o lstart= -p "$p" 2>/dev/null | tr -s ' ')" >&2
    done <<< "$ALL_PIDS"
    echo "  Refusing to guess. One process is expected — more than one is itself a" >&2
    echo "  finding (a stale copy, or single-instance forwarding not taking effect)." >&2
    echo "  Re-run naming the one showing the fault: $0 --pid <N> [output-file]" >&2
    exit 2
  fi
  PID="$ALL_PIDS"
fi

if ! kill -0 "$PID" 2>/dev/null; then
  echo "capture-weld: no process $PID, or it is not ours to attach to." >&2
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
LLDB_RC=$?
rm -f "$SCRIPT"

# lldb's status is checked, and it is NOT sufficient on its own: a refused
# attach exits non-zero, but an attach that succeeded while every `expr` threw
# exits 0 with an error transcript. So the status gates here and the parse below
# gates the rest — the raw transcript is kept in both cases, because the reason
# an attach failed (a missing task port, a Developer Tools prompt, a debugger
# already attached) is only visible there.
if [ "$LLDB_RC" -ne 0 ]; then
  cp "$RAW" "$OUT"
  rm -f "$RAW"
  echo "capture-weld: lldb exited $LLDB_RC; NO window graph was captured." >&2
  echo "  Its transcript is in $OUT — read it rather than this message." >&2
  echo "  Common causes: the process is not debuggable by this user, another" >&2
  echo "  debugger holds it, or macOS has not been told to allow attaching." >&2
  exit 1
fi

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
PY_RC=$?

# "written to" IS THE CLAIM, so it is made only where the claim is true. The
# Python block already exited 1 on a parse failure and already said so; what was
# missing was anything here listening. Without this, a run that wrote
# "PARSE FAILED" into the output file went on to print "written to <file>" and
# exit 0, and a caller — or a person skimming the last line — read success.
if [ "$PY_RC" -ne 0 ]; then
  rm -f "$RAW"
  echo >&2
  echo "capture-weld: the window graph could NOT be parsed; $OUT holds the raw" >&2
  echo "  lldb transcript instead of a table. This is not a capture." >&2
  exit 1
fi

rm -f "$RAW"
echo
echo "written to $OUT"
