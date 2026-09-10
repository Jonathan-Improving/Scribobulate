#!/usr/bin/env bash
#
# Build-pipeline step 5b (Linux): the per-render memory-growth class (TDD 6.6–6.8).
#
# A helper rather than a contract one-liner for the same reason as step 5: the
# bodies need a throwaway GTK session, and that session lives in `scripts/gtk-run.sh`.
# This file names the command and the wedge budget; it does not restate the session.
set -uo pipefail

# Runtime is tens of milliseconds once compiled; the budget is the release
# rebuild of gtk_suite on a cold target/.
BUDGET="${SCRIB_MEMORY_GATES_BUDGET:-600}"

exec "$(dirname "$0")/gtk-run.sh" memory "$BUDGET" \
    cargo test --release --features memory-gates --test gtk_suite memgate
