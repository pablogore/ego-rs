#!/usr/bin/env bash
set -euo pipefail

# Tests for scripts/verify-constitution-mapping.sh: it must actually fail the
# gate when cargo test/clippy/fmt fail or when AGENTS.md is missing or has
# incomplete/pending entries — not downgrade any of that to a WARN and exit 0
# regardless, as the pre-fix script did.
#
# Uses CARGO_TEST_CMD/CARGO_CLIPPY_CMD/CARGO_FMT_CMD to inject fast fakes
# instead of running the real (slow) cargo commands, and AGENTS_MD_PATH to
# point at throwaway fixtures instead of the repo's real AGENTS.md.

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET="$ROOT/scripts/verify-constitution-mapping.sh"
FAILURES=0

pass() { echo "  PASS: $1"; }
fail() { echo "  FAIL: $1"; FAILURES=$((FAILURES + 1)); }

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

CLEAN_AGENTS_MD="$FIXTURE_DIR/AGENTS-clean.md"
printf 'evidence: none required\n' >"$CLEAN_AGENTS_MD"

PENDING_AGENTS_MD="$FIXTURE_DIR/AGENTS-pending.md"
printf 'task: still incomplete\n' >"$PENDING_AGENTS_MD"

echo "test_all_checks_pass_on_a_clean_tree"
set +e
OUTPUT="$(CARGO_TEST_CMD=true CARGO_CLIPPY_CMD=true CARGO_FMT_CMD=true \
    AGENTS_MD_PATH="$CLEAN_AGENTS_MD" "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -ne 0 ]; then
    fail "gate failed on a clean tree: $OUTPUT"
else
    pass "gate exits 0 when cargo checks pass and AGENTS.md has no incomplete entries"
fi

echo "test_cargo_test_failure_fails_the_gate"
set +e
OUTPUT="$(CARGO_TEST_CMD=false CARGO_CLIPPY_CMD=true CARGO_FMT_CMD=true \
    AGENTS_MD_PATH="$CLEAN_AGENTS_MD" "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ]; then
    fail "gate exited 0 despite cargo test failing: $OUTPUT"
else
    pass "gate fails when cargo test fails"
fi

echo "test_missing_agents_md_fails_closed"
set +e
OUTPUT="$(CARGO_TEST_CMD=true CARGO_CLIPPY_CMD=true CARGO_FMT_CMD=true \
    AGENTS_MD_PATH="$FIXTURE_DIR/does-not-exist.md" "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ]; then
    fail "gate exited 0 despite AGENTS.md being missing: $OUTPUT"
else
    pass "gate fails closed when AGENTS.md does not exist"
fi

echo "test_incomplete_task_entry_fails_the_gate"
set +e
OUTPUT="$(CARGO_TEST_CMD=true CARGO_CLIPPY_CMD=true CARGO_FMT_CMD=true \
    AGENTS_MD_PATH="$PENDING_AGENTS_MD" "$TARGET" 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ]; then
    fail "gate exited 0 despite an incomplete task entry in AGENTS.md: $OUTPUT"
else
    pass "gate fails when AGENTS.md has an incomplete/pending task entry"
fi

echo ""
if [ "$FAILURES" -eq 0 ]; then
    echo "OK: all verify-constitution-mapping.sh tests passed"
    exit 0
else
    echo "FAILED: $FAILURES constitution-mapping gate test(s) failed"
    exit 1
fi
