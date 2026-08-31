#!/usr/bin/env bash
set -euo pipefail

# Tests for scripts/validate-constitution.sh's orchestration behavior:
# it must locate the 7 check scripts regardless of the caller's cwd, and
# it must distinguish "check script unavailable" from "check ran and failed".
#
# Uses a throwaway CHECKS_DIR of fast fake scripts instead of the real
# (slow: tarpaulin, cargo test/clippy) checks, so this stays a fast,
# deterministic unit test of the orchestrator's own logic.

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET="$ROOT/scripts/validate-constitution.sh"
FAILURES=0

pass() { echo "  PASS: $1"; }
fail() { echo "  FAIL: $1"; FAILURES=$((FAILURES + 1)); }

FAKE_DIR="$(mktemp -d)"
trap 'rm -rf "$FAKE_DIR"' EXIT

for name in detect-violations detect-missing-docs detect-test-smells \
            detect-mock-only-tests verify-coverage verify-constitution-mapping \
            detect-integration-tests; do
    printf '#!/usr/bin/env bash\nexit 0\n' >"$FAKE_DIR/$name.sh"
    chmod +x "$FAKE_DIR/$name.sh"
done

echo "test_runs_correctly_from_a_different_cwd"
set +e
OUTPUT="$(cd /tmp && CHECKS_DIR="$FAKE_DIR" "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -ne 0 ]; then
    fail "orchestrator failed from a different cwd: $OUTPUT"
else
    pass "orchestrator runs successfully regardless of caller cwd"
fi

echo "test_missing_check_script_is_reported_as_unavailable_not_as_a_violation"
rm -f "$FAKE_DIR/detect-violations.sh"

set +e
OUTPUT="$(CHECKS_DIR="$FAKE_DIR" "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e

if [ "$EXIT_CODE" -eq 0 ]; then
    fail "orchestrator exited 0 while a check script was unavailable"
elif ! echo "$OUTPUT" | grep -qi "unavailable"; then
    fail "orchestrator did not report the unavailable check separately from a real violation"
else
    pass "orchestrator reports the unavailable check distinctly and fails closed"
fi

echo ""
if [ "$FAILURES" -eq 0 ]; then
    echo "OK: all validate-constitution.sh orchestration tests passed"
    exit 0
else
    echo "FAILED: $FAILURES orchestration test(s) failed"
    exit 1
fi
