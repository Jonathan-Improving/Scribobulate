#!/usr/bin/env bash
#
# Run a command against a THROWAWAY GTK session: a private X display, a private D-Bus,
# criticals fatal, output that cannot wedge a reader, and a wall-clock bound.
#
#     scripts/gtk-run.sh <label> <budget-seconds> <command> [args...]
#
# `<label>` names the caller in this script's own verdicts ("integration", "coverage");
# `<budget-seconds>` is that caller's wedge bound. Exit status is the command's own,
# except that a timeout is reported as its own verdict and passed through as 124/137.
#
# ── WHY THIS IS A SCRIPT AND NOT FOUR WORDS ON A COMMAND LINE ─────────────────────────
#
# Two pipeline steps need this session — step 5 (the GTK integration suite) and step 6's
# full-suite coverage leg — and every one of the four concerns below was measured the
# hard way. A second hand-written copy of them is a copy that will be right on the day it
# is written and wrong afterwards: the nesting order in particular FAILS FAVOURABLY, so a
# green run in the wrong order is not evidence of anything.
#
#
# 1. IT ARMS `dbus-run-session`, WHICH POLICY HAS ALWAYS PRESCRIBED AND NOTHING RAN.
#
# In a session with no accessibility bus — every agent session, and any session whose
# `at-spi-dbus-bus.service` has gone stale — GTK emits `Unable to connect to the
# accessibility bus` as a `Gtk-CRITICAL`, and `G_DEBUG=fatal-criticals` promotes it to a
# SIGTRAP before a single test runs. The failure names accessibility, is not about
# accessibility, is not about the change under test, and reproduces identically on an
# untouched tree, so it costs a control build to disbelieve every time.
#
# POLICY § Build pipeline has said to run under `dbus-run-session` since that was
# measured. The contract's command did not, so the instruction lived only in prose and
# each caller supplied it by hand or got the SIGTRAP — precisely the failure the pipeline
# contract records about `G_DEBUG=fatal-criticals`, a prescribed gate with nothing in the
# toolchain arming it.
#
#
# 2. IT KEEPS BUS-ACTIVATED DAEMONS OFF THE CALLER'S STDOUT.
#
# ⚠ Without this the pipeline HANGS A READER FOREVER ON A PASSING RUN, which is the worst
# shape a gate can fail in: a green run and a wedged one are indistinguishable from
# outside. MEASURED 2026-08-28 — `scripts/pipeline.sh | tail -45` sat for 70 minutes on a
# pipeline that had finished in about 12.
#
# The mechanism: `dbus-run-session` starts a private bus, and GTK activity activates a
# crowd of services on it — reproduced in isolation as portal.Desktop, portal.Documents,
# PermissionStore, portal-{gnome,kde,gtk}, gvfs, org.a11y.Bus, atspi.Registry and
# secrets. The private `dbus-daemon` forks each one, so every one inherits this process's
# stdout and stderr. They outlive the bus (systemd --user reaps them as subreaper), so the
# write end of the caller's pipe stays open after the pipeline exits, and any reader
# waiting for EOF waits forever. `timeout` does not help: the command already finished.
#
# The fix is to hand the daemons a FILE instead of the caller's pipe. Everything below
# runs with its output redirected to `$log`, which is emitted here once the command is
# done — so a daemon that lingers holds a descriptor on a temp file nobody is waiting on,
# and this script's own exit closes the caller's pipe normally.
#
# Do NOT "simplify" this into a direct `dbus-run-session -- <cmd>`. The redirect IS the
# fix, and its absence is invisible until someone pipes the pipeline. Chasing the daemons
# individually (`GTK_USE_PORTAL=0` and friends) is whack-a-mole: the list above is nine
# services deep and grows with the desktop, and one missed entry restores the hang.
#
#
# 3. THE NESTING ORDER IS LOAD-BEARING: `xvfb-run` OUTSIDE, `dbus-run-session` INSIDE.
#
# ⚠ Inverted — which is how this ran until it was measured — the private bus is started
# BEFORE the display exists, so it inherits the ambient `DISPLAY`. Every service it
# activates then inherits that too, and the crowd from note 2 connects to the DEVELOPER'S
# REAL X SERVER instead of the Xvfb this exists to isolate. `xvfb-run` only rewrites
# `DISPLAY` for the command it wraps, so wrapping the test command alone isolates the
# tests and nothing else.
#
# MEASURED 2026-08-30 on the reference host: inverted, one run made ~20 connections to the
# live `:0` and printed `qt.qpa.xcb: could not connect to display :0` twenty-one times
# alongside `Maximum number of clients reached`; the session's X server was at 251 of
# X.org's 256-client default, so the step aborted under `G_DEBUG=fatal-criticals` with a
# `SIGTRAP` naming the accessibility bus — note 1's failure, arriving by a different road
# and immune to note 1's fix. In this order: zero contacts with `:0`, zero refusals.
#
# IT FAILS FAVOURABLY, which is why it survived. On a desktop with client slots to spare
# the leak is invisible and every test passes; it only goes red once the developer's own
# session is full, at which point it reports a fault in the accessibility bus. So a GREEN
# RUN IN THE INVERTED ORDER WAS NEVER EVIDENCE OF ISOLATION — it was evidence that the
# machine had room. An isolation boundary has to enclose everything that inherits the
# environment, not just the process under test.
#
#
# 4. IT BOUNDS THE RUN.
#
# A wedged GTK suite is a real failure mode (GEP-31 is a whole entry about
# misdiagnosing one), and an unbounded step turns it into a run nobody can tell from a
# slow one. Budgets are the caller's, and are deliberately generous: the timeout exists to
# catch a WEDGE, not to police duration. A timeout is reported as its own distinct verdict
# rather than as a command failure, because "it said no" and "it never answered" are
# different findings and must not print the same way.
set -uo pipefail

if [ "$#" -lt 3 ]; then
    echo "usage: $0 <label> <budget-seconds> <command> [args...]" >&2
    exit 2
fi
label="$1"
budget="$2"
shift 2

# The label reaches a `mktemp` TEMPLATE and, until this line, the EXIT trap's shell.
# Neither is a place to put an argument verbatim: a label containing a quote closed the
# trap's string and ran the rest as a command, and one containing a `/` produces a
# template `mktemp` refuses outright (F-SEC-207). Reduced to the character class a label
# is actually made of.
label_safe=${label//[^A-Za-z0-9_-]/_}
log=$(mktemp -t "scrib-$label_safe.XXXXXX")
# Where the run records the private bus address and display it actually got, so teardown
# can find the daemons that bus activated. Written from INSIDE the session because
# `xvfb-run -a` and `dbus-run-session` both choose their values themselves.
session=$(mktemp -t "scrib-$label_safe-session.XXXXXX")

# ── 5. IT ENDS WHAT THE RUN LEFT BEHIND ──────────────────────────────────────────────
#
# Note 2 keeps the bus-activated daemons off the caller's stdout; it does not end them.
# Nothing here does either, so this exists to bound the run in PROCESSES the way note 4
# bounds it in time. Two separate leaks, and the measurements say opposite things about
# them — which is the point of writing both down.
#
# THE DAEMONS: measured 2026-09-16, they do NOT survive this script. A run that activates
# `portal.Desktop`, `portal.Documents`, PermissionStore, portal-gnome and gvfs leaves ZERO
# processes holding the private bus address — on the clean path AND on the timeout-kill
# path. So the sweep below is INSURANCE, not a fix for an observed leak here: it is proven
# against a deliberately detached child (`setsid`, re-parented away, found and killed), and
# it is kept because the population of services is desktop-dependent and grows, and one
# that does hold on costs a developer their whole login session to notice.
#
# ⚠ The orphan crowds that prompted this were NOT from this script. Measured the same day:
# 34 `xdg-desktop-portal`/`gvfsd` processes on displays `:71` and `:77`, none carrying an
# `xvfb-run` XAUTHORITY — hand-picked display numbers, i.e. ad-hoc `dbus-run-session`
# rigs run outside this script. Do not read this section as evidence the callers leak.
#
# THE X SERVER outlives the run BRIEFLY, and its directory outlives it for good. Measured:
# a timed-out run's `Xvfb :99` was still resident minutes after the run ended, and then
# exited by itself; meanwhile the host carried 21 stale `/tmp/xvfb-run.XXXXXX` directories
# going back a fortnight. Both come from the same skipped EXIT trap — `xvfb-run` kills its
# server AND removes that directory there, and bash defers a trap until the foreground
# child returns, so a SIGKILL to the wrapper skips both. The server dies anyway (`timeout`
# signals the whole PROCESS GROUP); the directory has nothing to clean it up, so THAT is
# the leak with a body count.
#
# So the server reap is a backstop, proven correct — it finds the server and re-verifies
# the PID — but never observed to fire, and it is kept for the run where the group signal
# does not reach. The directory removal beside it is the part that was actually leaking.
#
# BOTH are keyed on identity, never on a name: the daemons on the private bus address
# (a per-run GUID, inherited by everything the bus activates), the X server on a PID
# captured during the run and re-verified against its own `/proc` entry before the signal.
# A `pkill Xvfb` or `pkill xdg-desktop-portal` would reach the developer's `:0` session,
# which is the one outcome this must never have.
reap_stranded_xserver() {
    local display="$1" pid="$2" auth="$3" cmdline
    # THE DIRECTORY FIRST, and unconditionally. It is the leak that actually persists, and
    # it outlives the server — so hanging its removal off "is the server still alive?"
    # would clean up in exactly the case that does not need it and skip the usual one.
    # Pattern-matched rather than trusted: this is a variable from a child's environment
    # reaching `rm -rf`, and only `xvfb-run`'s own shape may pass.
    case "$auth" in
        /tmp/xvfb-run.*/Xauthority) rm -rf "${auth%/Xauthority}" ;;
    esac
    [ -n "$display" ] && [ -n "$pid" ] || return 0
    cmdline=$({ tr '\0' ' ' < "/proc/$pid/cmdline"; } 2>/dev/null) || return 0
    case "$cmdline" in
        "Xvfb $display "* | *"/Xvfb $display "*) ;;
        # The PID was captured while the run was alive, so by now it may name nothing, or
        # — after enough turnover — something else entirely. A PID is not an identity once
        # the process it named has exited, so it is re-verified, not merely signalled.
        *) return 0 ;;
    esac
    kill "$pid" 2>/dev/null
    echo "$label: reaped the stranded X server on $display (pid $pid)."
}

reap_session_daemons() {
    [ -s "$session" ] || return 0
    local display bus xvfb xauth pid environ victims=()
    { read -r display; read -r bus; read -r xvfb; read -r xauth; } < "$session" || return 0
    reap_stranded_xserver "$display" "$xvfb" "$xauth"
    # An empty address would match every process that has no such variable at all.
    [ -n "$bus" ] || return 0
    for pid in /proc/[0-9]*; do
        pid=${pid#/proc/}
        [ "$pid" = "$$" ] && continue
        # Most of /proc belongs to root or to other users, and `2>/dev/null` on the `tr`
        # does NOT silence this: the failure is the SHELL's, refused while opening the
        # redirect, so it is the shell's stderr that carries it. Unsilenced it buried a
        # measured run in 553 `Permission denied` lines. Read-test first, and redirect the
        # whole compound in case the process exits between the test and the read.
        [ -r "/proc/$pid/environ" ] || continue
        environ=$({ tr '\0' '\n' < "/proc/$pid/environ"; } 2>/dev/null) || continue
        case "$environ" in *"$bus"*) victims+=("$pid") ;; esac
    done
    [ "${#victims[@]}" -eq 0 ] && return 0
    kill "${victims[@]}" 2>/dev/null
    # A daemon that ignores SIGTERM is the case this exists for, so do not stop at asking.
    sleep 0.5
    kill -9 "${victims[@]}" 2>/dev/null
    echo "$label: reaped ${#victims[@]} bus-activated daemon(s) from this run's private session."
}
# Single-quoted, so `$log` expands when the trap FIRES rather than being pasted into the
# trap's source now. It is still in scope then, which is what makes the deferred
# expansion both safe and correct — and it is why the SC2064 suppression that used to
# sit here is gone rather than moved.
trap 'reap_session_daemons; rm -f "$log" "$session"' EXIT

# `--kill-after` so a command ignoring SIGTERM still dies rather than becoming the hang
# this script exists to prevent.
#
# Wall clock around it, because the exit status alone cannot tell a timeout from a kill
# (F-SEC-208). 137 is "died on SIGKILL" and says nothing about who sent it: `timeout`
# does after the budget, and so does the kernel's OOM killer — or `earlyoom` — after a
# few seconds. Diagnosing the second as the first sends the reader after a wedge that
# never happened, which is the one misdiagnosis this project has a written anti-pattern
# about (GTK4Rs/AP-133).
#
# The innermost shell records what section 5's teardown needs, and records it from INSIDE
# the session because that is the only place it exists: both wrappers choose their own
# values, and the X server's PID is discoverable only while the run is alive. It `exec`s
# the command, so it costs a process image, not a process.
started=$(date +%s)
timeout --kill-after=60s "$budget" \
    xvfb-run -a \
    dbus-run-session -- \
    env G_DEBUG=fatal-criticals \
    bash -c '
        d=${DISPLAY:-}; x=
        for p in /proc/[0-9]*; do
            [ -r "$p/cmdline" ] || continue
            c=$({ tr "\0" " " < "$p/cmdline"; } 2>/dev/null) || continue
            case "$c" in "Xvfb $d "*|*"/Xvfb $d "*) x=${p#/proc/}; break;; esac
        done
        printf "%s\n%s\n%s\n%s\n" "$d" "${DBUS_SESSION_BUS_ADDRESS:-}" "$x" "${XAUTHORITY:-}" > "$1"
        shift; exec "$@"' \
    gtk-run "$session" "$@" \
    >"$log" 2>&1
rc=$?
elapsed=$(( $(date +%s) - started ))

cat "$log"

# `$budget` is a `timeout` duration and may carry a suffix; strip it for the comparison,
# and treat an unparseable one as "assume the budget elapsed" — the old behaviour, and
# the conservative direction for a verdict about a hang.
budget_secs=${budget%%[!0-9]*}
if [ "$rc" -eq 124 ] || { [ "$rc" -eq 137 ] && [ -n "$budget_secs" ] && [ "$elapsed" -ge "$budget_secs" ]; }; then
    echo
    echo "$label: NO VERDICT — the command did not finish within ${budget}s and was killed."
    echo "$label: this is a WEDGE, not a failure of the thing under test; the output above"
    echo "$label: is whatever it managed to print. Do not diagnose it from a parallel run"
    echo "$label: (GEP-31)."
elif [ "$rc" -eq 137 ]; then
    echo
    echo "$label: NO VERDICT — the command was killed by SIGKILL after ${elapsed}s, BEFORE"
    echo "$label: its ${budget}s budget elapsed. This is NOT a timeout and NOT a wedge:"
    echo "$label: something outside the command killed it. Check 'dmesg | grep -i oom' and"
    echo "$label: 'pgrep -a earlyoom' (GTK4Rs/AP-133) before reading anything above as a"
    echo "$label: result — an OOM kill mid-suite leaves output that looks like a failure."
fi

exit "$rc"
