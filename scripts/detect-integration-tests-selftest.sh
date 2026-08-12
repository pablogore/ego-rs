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

# --- 3. The membership check, driven with controlled metadata --------------
#
# Check 4b asks cargo whether `integration-tests/` resolves as a member of the
# root workspace. Running it against the real tree can only ever demonstrate the
# passing case — and a check shown only to pass is exactly the failure this
# self-test exists to prevent.
#
# So the evaluator is driven directly, through the guard's `--eval-membership`
# seam, with metadata this script writes. That covers the membership regression
# without mutating the real workspace: mutating it for real is not equivalent,
# because a glob plus the carve-out's own `[workspace]` table makes cargo refuse
# outright, so the interesting case — silent membership — cannot be reached by
# editing the root manifest alone.
FAKE_ROOT='/fake/repo'

# 3a. Metadata in which the carve-out IS a member. The evaluator must reject it.
#
# This is the regression the old text search missed entirely: the member is
# resolved through a glob, so the name never appears in the root manifest.
member_metadata='{"packages":[
  {"name":"ego-domain","manifest_path":"/fake/repo/crates/domain/Cargo.toml"},
  {"name":"ego-integration-tests","manifest_path":"/fake/repo/integration-tests/Cargo.toml"}
]}'
if printf '%s' "$member_metadata" | "$GUARD" --eval-membership "$FAKE_ROOT" >/dev/null 2>&1; then
    echo "FAIL[selftest]: the membership check PASSED on metadata that lists"
    echo "  /fake/repo/integration-tests/Cargo.toml as a workspace member."
    echo "  A glob in the root's members list would make the carve-out part of"
    echo "  'cargo test --workspace' without ever spelling its name."
    failures=1
else
    echo "ok[selftest]: membership check rejects a carve-out resolved as a member"
fi

# 3b. Metadata in which it is not. The evaluator must accept it — otherwise 3a
#     passes for the trivial reason that the check rejects everything.
clean_metadata='{"packages":[
  {"name":"ego-domain","manifest_path":"/fake/repo/crates/domain/Cargo.toml"},
  {"name":"ego-persistence","manifest_path":"/fake/repo/crates/persistence/Cargo.toml"}
]}'
if printf '%s' "$clean_metadata" | "$GUARD" --eval-membership "$FAKE_ROOT" >/dev/null 2>&1; then
    echo "ok[selftest]: membership check accepts a workspace without the carve-out"
else
    echo "FAIL[selftest]: the membership check rejected clean metadata."
    echo "  A check that rejects everything proves nothing about the case above."
    failures=1
fi

# 3c. A sibling whose name merely starts with the carve-out's is NOT inside it.
#     Guards against a prefix match standing in for a path match.
sibling_metadata='{"packages":[
  {"name":"helpers","manifest_path":"/fake/repo/integration-tests-helpers/Cargo.toml"}
]}'
if printf '%s' "$sibling_metadata" | "$GUARD" --eval-membership "$FAKE_ROOT" >/dev/null 2>&1; then
    echo "ok[selftest]: membership check does not confuse a sibling for a child"
else
    echo "FAIL[selftest]: 'integration-tests-helpers' was treated as living"
    echo "  inside 'integration-tests/'. The prefix test needs its trailing slash."
    failures=1
fi

# 3d. Empty input finds no violations, and must not crash trying.
#
# Emptiness is what a failed `cargo metadata` produces, so the evaluator has to
# survive it predictably. It reports no violations — which is why the guard
# treats a metadata failure as a FAIL *before* the evaluator ever sees the
# output. That ordering is the real defence; this asserts the evaluator does not
# instead abort under `pipefail` and get misread as a violation.
if printf '' | "$GUARD" --eval-membership "$FAKE_ROOT" >/dev/null 2>&1; then
    echo "ok[selftest]: empty metadata is handled without crashing"
else
    echo "FAIL[selftest]: the evaluator errored on empty input. A failed"
    echo "  'cargo metadata' would then be reported as a membership violation,"
    echo "  or worse, abort the guard mid-run."
    failures=1
fi

# 3e. And the real tree must satisfy the check it ships with.
real_metadata=$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 2>/dev/null || true)
if [ -z "$real_metadata" ]; then
    echo "FAIL[selftest]: could not read real root metadata, so the shipped"
    echo "  membership check is unverified on this tree."
    failures=1
elif printf '%s' "$real_metadata" | "$GUARD" --eval-membership "$ROOT" >/dev/null 2>&1; then
    echo "ok[selftest]: real root metadata places the carve-out outside the workspace"
else
    echo "FAIL[selftest]: cargo resolves integration-tests/ as a member of the"
    echo "  root workspace. That is the isolation every E2E here depends on."
    failures=1
fi

# --- 4. The tree is back as it was ----------------------------------------
if [ -e "$PLANTED" ]; then
    echo "FAIL[selftest]: the planted file survived cleanup."
    failures=1
fi

if [ "$failures" -eq 0 ]; then
    echo "PASS[selftest]: the guard examines files, resolves real membership, and"
    echo "  rejects what it must."
    exit 0
fi

echo "FAIL[selftest]: the guard cannot be trusted."
exit 1
