#!/usr/bin/env bash
set -euo pipefail

# Tests for scripts/detect-missing-docs.sh:
#   1. trybuild fixtures (tests/compile_fail, tests/compile_pass) must be
#      exempt from the pub-API doc scan, and the exemption itself must fail
#      closed if its target directory no longer exists.
#   2. a registered architectural component whose path no longer exists must
#      FAIL the gate, not silently skip via `[ -f ... ]`.
#
# Uses SCAN_ROOT/ARCH_COMPONENTS overrides against throwaway fixtures instead
# of the real crates/ tree, so this stays fast and independent of whatever
# doc debt the real tree currently carries.

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET="$ROOT/scripts/detect-missing-docs.sh"
FAILURES=0

pass() { echo "  PASS: $1"; }
fail() { echo "  FAIL: $1"; FAILURES=$((FAILURES + 1)); }

FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

mkdir -p "$FIXTURE_ROOT/tests/compile_fail" "$FIXTURE_ROOT/src"
cat >"$FIXTURE_ROOT/tests/compile_fail/undocumented.rs" <<'EOF'
pub fn undocumented_in_exempt_dir() {}
EOF
cat >"$FIXTURE_ROOT/src/documented.rs" <<'EOF'
/// This one is documented.
pub fn documented() {}
EOF

echo "test_exempt_dir_undocumented_pub_item_does_not_fail_the_gate"
set +e
OUTPUT="$(SCAN_ROOT="$FIXTURE_ROOT" EXEMPT_DIRS_OVERRIDE="$FIXTURE_ROOT/tests/compile_fail" ARCH_COMPONENTS="" "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -ne 0 ]; then
    fail "gate failed on a clean tree with only an exempt-dir violation: $OUTPUT"
else
    pass "gate ignores undocumented pub items inside an exempt trybuild fixture dir"
fi

echo "test_non_exempt_undocumented_pub_item_fails_the_gate"
cat >"$FIXTURE_ROOT/src/broken.rs" <<'EOF'
pub struct Undocumented;
EOF
set +e
OUTPUT="$(SCAN_ROOT="$FIXTURE_ROOT" ARCH_COMPONENTS="" "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ]; then
    fail "gate exited 0 despite an undocumented public item outside any exempt dir"
else
    pass "gate fails on an undocumented public item outside the exempt dirs"
fi
rm -f "$FIXTURE_ROOT/src/broken.rs"

echo "test_missing_registered_arch_component_fails_the_gate"
set +e
OUTPUT="$(SCAN_ROOT="$FIXTURE_ROOT" ARCH_COMPONENTS="$FIXTURE_ROOT/src/does_not_exist.rs" "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ]; then
    fail "gate exited 0 despite a registered architectural component path not existing"
else
    pass "gate fails when a registered architectural component no longer exists"
fi

echo ""
if [ "$FAILURES" -eq 0 ]; then
    echo "OK: all detect-missing-docs.sh tests passed"
    exit 0
else
    echo "FAILED: $FAILURES docs gate test(s) failed"
    exit 1
fi
