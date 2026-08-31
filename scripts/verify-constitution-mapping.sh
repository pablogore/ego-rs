#!/usr/bin/env bash
set -euo pipefail

# verify-constitution-mapping.sh
#
# CI-time validation for constitution mapping and governance requirements.
#
# Actually enforced by this script:
#   1. cargo test passes
#   2. cargo clippy passes
#   3. cargo fmt --check passes
#   4. AGENTS.md exists and has no entries marked incomplete/pending
#
# NOT enforced by this script (documented here so its coverage claim
# doesn't overstate itself — tracked as pre-existing, out-of-scope debt):
#   - All tasks have required fields
#   - Evidence-complete requirement
#   - Coverage >= 85% floor (see verify-coverage.sh, which owns that check)
#   - Workflow stages not skipped
#   - Contract versions bumped on breaking changes

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
AGENTS_MD_PATH="${AGENTS_MD_PATH:-$ROOT/AGENTS.md}"
EXIT_CODE=0

echo "--- [G-R1 through G-R8] Verifying constitution mapping and governance..."

run_check() {
    local label="$1"
    local cmd="$2"
    echo "Checking $label..."
    if eval "$cmd"; then
        echo "PASS: $label"
    else
        echo "FAIL: $label"
        EXIT_CODE=1
    fi
}

CARGO_TEST_CMD="${CARGO_TEST_CMD:-cargo test --workspace --quiet}"
CARGO_CLIPPY_CMD="${CARGO_CLIPPY_CMD:-cargo clippy --workspace --quiet}"
CARGO_FMT_CMD="${CARGO_FMT_CMD:-cargo fmt --check}"

run_check "cargo test" "$CARGO_TEST_CMD"
run_check "cargo clippy" "$CARGO_CLIPPY_CMD"
run_check "cargo fmt" "$CARGO_FMT_CMD"

echo "Checking evidence and incomplete-task entries..."
if [ ! -f "$AGENTS_MD_PATH" ]; then
    echo "FAIL: $AGENTS_MD_PATH not found"
    EXIT_CODE=1
else
    evidence_count="$(grep -c "evidence:" "$AGENTS_MD_PATH" 2>/dev/null || true)"
    evidence_count="${evidence_count:-0}"
    echo "Found $evidence_count evidence entries in $AGENTS_MD_PATH"

    incomplete_tasks="$(grep -c "incomplete\|pending" "$AGENTS_MD_PATH" 2>/dev/null || true)"
    incomplete_tasks="${incomplete_tasks:-0}"
    if [ "$incomplete_tasks" -gt 0 ]; then
        echo "FAIL: found $incomplete_tasks incomplete/pending task entries"
        EXIT_CODE=1
    else
        echo "PASS: no incomplete tasks found"
    fi
fi

if [ "$EXIT_CODE" -eq 0 ]; then
    echo "PASS: Constitution mapping and governance validation completed."
else
    echo "FAIL: Constitution mapping and governance validation failed."
fi

exit "$EXIT_CODE"
