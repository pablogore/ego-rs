#!/usr/bin/env bash
set -euo pipefail

# detect-integration-tests-selftest.sh
#
# Proves that `detect-integration-tests.sh` actually examines files.
#
# ---------------------------------------------------------------------------
# Why this exists
# ---------------------------------------------------------------------------
#
# The guard's forbidden-pattern check scanned with the pathspec `'**/tests/'`,
# which matches zero files — a pathspec ending in `/` with no file component
# matches nothing, because pathspecs match files, not directories. Every green
# run reported a safety it had never checked.
#
# A fix to that is not verifiable by running the guard and watching it pass:
# passing is exactly what the broken version did. The only way to know the
# guard looks at anything is to plant something it must reject and watch it
# fail. That is what this does.
#
# Run it the way CI would:
#   scripts/detect-integration-tests-selftest.sh
#
# It leaves the tree exactly as it found it, including on failure.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$ROOT/scripts/detect-integration-tests.sh"

# `git grep` only sees tracked files, so the planted file has to be added to
# the index. It is removed again by the trap below, whatever happens.
PLANTED_DIR="$ROOT/crates/domain/tests"
PLANTED="$PLANTED_DIR/zz_guard_selftest_planted.rs"
CREATED_DIR=0

cleanup() {
    git -C "$ROOT" rm --cached --quiet --force "$PLANTED" 2>/dev/null || true
    rm -f "$PLANTED"
    if [ "$CREATED_DIR" -eq 1 ]; then
        rmdir "$PLANTED_DIR" 2>/dev/null || true
    fi
}
trap cleanup EXIT

failures=0

# --- 1. The guard passes on the clean tree ---------------------------------
#
# Establishes the baseline. Without it, a guard that always failed would
# satisfy the check below for the wrong reason.
if ! "$GUARD" >/dev/null 2>&1; then
    echo "FAIL[selftest]: the guard does not pass on a clean tree."
    failures=1
else
    echo "ok[selftest]: guard passes on the clean tree"
fi

# --- 2. The guard fails on a planted violation -----------------------------
#
# This is the assertion the broken pathspec could never satisfy.
if [ ! -d "$PLANTED_DIR" ]; then
    mkdir -p "$PLANTED_DIR"
    CREATED_DIR=1
fi
cat > "$PLANTED" <<'PLANTED_EOF'
// Planted by detect-integration-tests-selftest.sh. If you are reading this in
// a committed tree, the self-test did not clean up after itself.
#[test]
fn planted() {
    let _ = "testcontainers";
}
PLANTED_EOF
git -C "$ROOT" add --intent-to-add --force "$PLANTED" >/dev/null 2>&1

if "$GUARD" >/dev/null 2>&1; then
    echo "FAIL[selftest]: the guard PASSED with a forbidden pattern planted at:"
    echo "  ${PLANTED#"$ROOT/"}"
    echo "  This is the original defect: the pathspec matches no files, so the"
    echo "  forbidden-pattern list scans nothing and reports safety it never"
    echo "  checked."
    failures=1
else
    echo "ok[selftest]: guard rejects a forbidden pattern under a tests/ path"
fi

cleanup
trap - EXIT

# --- 3. The tree is back as it was ----------------------------------------
if [ -e "$PLANTED" ]; then
    echo "FAIL[selftest]: the planted file survived cleanup."
    failures=1
fi

if [ "$failures" -eq 0 ]; then
    echo "PASS[selftest]: the guard examines files and rejects what it must."
    exit 0
fi

echo "FAIL[selftest]: the guard cannot be trusted."
exit 1
