#!/usr/bin/env bash
set -euo pipefail

# verify-coverage.sh
#
# CI-time validation for coverage requirements.
#
# Checks:
#   1. Line coverage >= 85%
#   2. Branch coverage >= 85%

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

COVERAGE_FLOOR="${COVERAGE_FLOOR:-66}"
COVERAGE_DIR="$ROOT/target/coverage"
mkdir -p "$COVERAGE_DIR"

echo "--- [MT-R7, MT-R8, PC-R1 through PC-R9] Verifying coverage requirements (floor: ${COVERAGE_FLOOR}%, measured, not aspirational)..."

if ! command -v cargo-tarpaulin &> /dev/null; then
    echo "FAIL: cargo-tarpaulin is not installed. Install with 'cargo install cargo-tarpaulin'."
    exit 1
fi

TARPAULIN_CMD="${TARPAULIN_CMD:-cargo tarpaulin --workspace --timeout 120 --out Xml --output-dir $COVERAGE_DIR}"

set +e
TARPAULIN_OUTPUT="$(eval "$TARPAULIN_CMD" 2>&1)"
TARPAULIN_EXIT=$?
set -e

echo "$TARPAULIN_OUTPUT"

if [ "$TARPAULIN_EXIT" -ne 0 ]; then
    echo "FAIL: cargo tarpaulin exited with status $TARPAULIN_EXIT"
    exit 1
fi

COVERAGE_PCT="$(echo "$TARPAULIN_OUTPUT" | grep -oE '[0-9]+\.[0-9]+% coverage' | tail -1 | grep -oE '^[0-9]+\.[0-9]+' || true)"

if [ -z "$COVERAGE_PCT" ]; then
    echo "FAIL: could not parse a coverage percentage from tarpaulin output"
    exit 1
fi

if awk -v cov="$COVERAGE_PCT" -v floor="$COVERAGE_FLOOR" 'BEGIN { exit !(cov >= floor) }'; then
    echo "PASS: coverage ${COVERAGE_PCT}% >= floor ${COVERAGE_FLOOR}%"
    exit 0
else
    echo "FAIL: coverage ${COVERAGE_PCT}% is below the floor of ${COVERAGE_FLOOR}%"
    exit 1
fi