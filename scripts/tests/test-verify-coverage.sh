#!/usr/bin/env bash
set -euo pipefail

# Tests for scripts/verify-coverage.sh: it must actually fail when coverage
# is below the floor, or when tarpaulin itself errors/produces unparseable
# output — not silently exit 0 regardless, as the pre-fix script did.
#
# Uses TARPAULIN_CMD to inject fake tarpaulin output instead of running the
# real (slow) cargo tarpaulin, so this stays a fast, deterministic test of
# the gate's own pass/fail logic.

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET="$ROOT/scripts/verify-coverage.sh"
FAILURES=0

pass() { echo "  PASS: $1"; }
fail() { echo "  FAIL: $1"; FAILURES=$((FAILURES + 1)); }

echo "test_coverage_above_floor_passes"
set +e
OUTPUT="$(TARPAULIN_CMD='printf "90.00%% coverage, 900/1000 lines covered\n"' "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -ne 0 ]; then
    fail "gate exited non-zero ($EXIT_CODE) for coverage above the floor: $OUTPUT"
else
    pass "gate exits 0 when coverage is above the floor"
fi

echo "test_coverage_below_floor_fails"
set +e
OUTPUT="$(TARPAULIN_CMD='printf "10.00%% coverage, 100/1000 lines covered\n"' "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ]; then
    fail "gate exited 0 despite coverage below the floor"
else
    pass "gate fails when coverage is below the floor"
fi

echo "test_tarpaulin_failure_fails_closed"
set +e
OUTPUT="$(TARPAULIN_CMD='bash -c "echo boom >&2; exit 1"' "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ]; then
    fail "gate exited 0 despite tarpaulin itself failing"
else
    pass "gate fails closed when tarpaulin errors"
fi

echo "test_unparseable_output_fails_closed"
set +e
OUTPUT="$(TARPAULIN_CMD='printf "no coverage information here\n"' "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ]; then
    fail "gate exited 0 despite unparseable tarpaulin output"
else
    pass "gate fails closed when the coverage percentage can't be parsed"
fi

echo ""
if [ "$FAILURES" -eq 0 ]; then
    echo "OK: all verify-coverage.sh tests passed"
    exit 0
else
    echo "FAILED: $FAILURES coverage gate test(s) failed"
    exit 1
fi
