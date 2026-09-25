#!/usr/bin/env bash
#
# Print the `--test <name>` arguments naming EVERY integration-test target the crate
# declares, for step 5's macOS command body.
#
# WHY THIS EXISTS AT ALL — macOS cannot use the other two ports' selection.
#
# Linux and Windows let Cargo choose: their step 5 runs every target, the library's own
# `#[test]` bodies included. macOS must not, because the dual-harness library bodies abort
# the process off the main thread on Quartz (GTK4Rs/AP-159, measured on GTK 4.22.4), so this
# port selects the integration targets BY NAME and leaves `--lib` out. That constraint is
# real and is not what this script changes.
#
# What it changes is WHERE THE NAMES COME FROM. They were written out by hand in the
# contract, which made the macOS step the only one whose target set could fall behind the
# manifest. It already had: the list drifted once and a person, not a gate, noticed. The
# failure mode is the bad one — a target absent from the list is never built and never run,
# and the step prints PASS for the four it did run, so the evidence of the omission is the
# absence of evidence.
#
# So the names are derived from `cargo metadata`, which reads the same manifest Cargo
# builds from. Adding a `[[test]]` target now adds it to this port's step 5 by
# construction, and nothing has to remember to.
#
# `cargo metadata` PARSES the manifest rather than building it, so this stays correct —
# and stays runnable — while the crate does not compile.
#
# FAILING LOUDLY IS THE WHOLE DESIGN. This is consumed as `$(...)` inside the contract's
# command body, and a command substitution that fails is silent: the shell substitutes
# empty and carries on. Empty here would leave a bare `cargo test --features ...`, which
# is not a smaller run — it is the ALL-TARGETS run this port must never make, with the
# library bodies back in it, aborting on Quartz for a reason nobody would connect to a
# missing enumeration. So on any failure this prints a target name that does not exist,
# which Cargo rejects by name, and writes the real reason to stderr where the step's
# output carries it.
#
#   scripts/macos-integration-targets.sh
#
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# The name is chosen to be self-explaining in Cargo's own error text, which is the only
# place a reader will first meet it: "error: no test target named `...`".
POISON="--test pipeline-could-not-derive-the-macos-test-targets"

die() {
    echo "macos-integration-targets: $*" >&2
    echo "  Step 5's target list could not be derived, so this is emitting a target name" >&2
    echo "  that does not exist rather than an empty list. An empty list would silently" >&2
    echo "  widen the run to every target, library bodies included, which aborts on" >&2
    echo "  Quartz (GTK4Rs/AP-159)." >&2
    printf '%s' "$POISON"
    exit 0   # The poison IS the report; a non-zero exit here would be swallowed by $( ).
}

command -v cargo   >/dev/null 2>&1 || die "cargo is not on PATH"
command -v python3 >/dev/null 2>&1 || die "python3 is not on PATH"

META=$(cargo metadata --no-deps --format-version 1 2>/dev/null) \
    || die "cargo metadata failed"
[ -n "$META" ] || die "cargo metadata produced no output"

NAMES=$(printf '%s' "$META" | python3 -c '
import json, os, sys

# THE WORKSPACE IS NOT THE CRATE, and the first version of this script got that wrong in
# the direction that matters. `--no-deps` still reports every workspace MEMBER, so taking
# every "test" target swept in the image-decoder crate as well and produced a twenty-target
# step 5 in place of a five-target one. It was caught by running it, not by reading it.
#
# The root package is identified by its manifest being the repository root Cargo.toml,
# rather than by its name. A name here would be a second copy of a fact the manifest
# already owns, and the one this script exists to stop being copied.
root_manifest = os.path.realpath(sys.argv[1])

md = json.load(sys.stdin)
names, matched = set(), False
for pkg in md.get("packages", []):
    if os.path.realpath(pkg.get("manifest_path", "")) != root_manifest:
        continue
    matched = True
    for tgt in pkg.get("targets", []):
        # "test" is the kind Cargo gives a [[test]] target. A lib is kind "lib" and a
        # binary "bin", so neither can arrive here — which is the point: this selection
        # can never widen to --lib however the manifest grows.
        if "test" in tgt.get("kind", []):
            names.add(tgt["name"])

if not matched:
    sys.stderr.write("no workspace package is rooted at %s\n" % root_manifest)
    raise SystemExit(1)

# Sorted so the command the runner echoes is stable between runs and diffable against
# the other ports. Cargo does not care about the order.
for n in sorted(names):
    print(n)
' "$repo_root/Cargo.toml") || die "could not read cargo metadata"

[ -n "$NAMES" ] || die "the manifest declares no [[test]] targets, which cannot be right"

OUT=""
while read -r n; do
    [ -n "$n" ] || continue
    # A target name Cargo would not accept unquoted here would be mangled by the word
    # splitting this output relies on. Cargo already restricts target names, so this
    # refuses rather than tries to quote its way out.
    case "$n" in
        *[!a-zA-Z0-9_-]*) die "test target '$n' has a character this cannot pass safely" ;;
    esac
    OUT="$OUT --test $n"
done <<< "$NAMES"

# No trailing newline: this is spliced into the middle of a command line.
printf '%s' "${OUT# }"
