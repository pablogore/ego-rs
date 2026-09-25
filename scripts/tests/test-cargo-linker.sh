#!/usr/bin/env bash
set -euo pipefail

# Tests for scripts/cargo-linker.sh, the linker driver .cargo/config.toml
# points every x86_64 Linux build at.
#
# The wrapper must use mold when it is installed and fall back to the plain
# system linker when it is not, because the same .cargo/config.toml is read by
# CI runners (mold installed), contributors' machines (maybe not), and the
# official rust:<version> image Shipwright's Dagger steps run in (no clang, no
# mold). EGO_REQUIRE_MOLD=1 turns the fallback into a hard error so CI can
# never silently lose the faster linker.
#
# rustc 1.90+ links x86_64 Linux through its bundled rust-lld by passing its own
# -fuse-ld=lld to the driver, and the last -fuse-ld on the command line wins,
# so the wrapper must append -fuse-ld=mold after rustc's arguments, not before.
#
# Each case runs the wrapper with PATH restricted to a throwaway directory of
# fake cc/clang/mold executables that record how they were invoked.

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET="$ROOT/scripts/cargo-linker.sh"
FAILURES=0

pass() { echo "  PASS: $1"; }
fail() { echo "  FAIL: $1"; FAILURES=$((FAILURES + 1)); }

FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT
LOG="$FIXTURE_ROOT/invocation.log"

# fake_bin DIR NAME...: installs executables that append "NAME ARGS" to $LOG.
fake_bin() {
    local dir="$1"
    shift
    mkdir -p "$dir"
    for name in "$@"; do
        cat >"$dir/$name" <<EOF
#!/bin/sh
printf '%s' "$name" >>"$LOG"
for arg in "\$@"; do printf ' [%s]' "\$arg" >>"$LOG"; done
printf '\n' >>"$LOG"
EOF
        chmod +x "$dir/$name"
    done
}

# run_wrapper BIN_DIR [ENV=VALUE...]: runs the wrapper with only BIN_DIR on PATH.
run_wrapper() {
    local bin_dir="$1"
    shift
    : >"$LOG"
    env -i PATH="$bin_dir" "$@" "$TARGET" -fuse-ld=lld -o "out file" main.o
}

echo "test_uses_clang_with_mold_when_both_are_installed"
fake_bin "$FIXTURE_ROOT/all" cc clang mold
set +e
run_wrapper "$FIXTURE_ROOT/all" >/dev/null 2>&1
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ] && [ "$(cat "$LOG")" = "clang [-fuse-ld=lld] [-o] [out file] [main.o] [-fuse-ld=mold]" ]; then
    pass "clang drives the link, rustc's args first, -fuse-ld=mold last so it wins"
else
    fail "expected clang with -fuse-ld=mold, got exit=$EXIT_CODE log='$(cat "$LOG")'"
fi

echo "test_uses_cc_with_mold_when_clang_is_missing"
fake_bin "$FIXTURE_ROOT/no-clang" cc mold
set +e
run_wrapper "$FIXTURE_ROOT/no-clang" >/dev/null 2>&1
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ] && [ "$(cat "$LOG")" = "cc [-fuse-ld=lld] [-o] [out file] [main.o] [-fuse-ld=mold]" ]; then
    pass "cc drives the link with -fuse-ld=mold when clang is absent"
else
    fail "expected cc with -fuse-ld=mold, got exit=$EXIT_CODE log='$(cat "$LOG")'"
fi

echo "test_falls_back_to_plain_cc_when_mold_is_missing"
fake_bin "$FIXTURE_ROOT/no-mold" cc clang
set +e
run_wrapper "$FIXTURE_ROOT/no-mold" >/dev/null 2>&1
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ] && [ "$(cat "$LOG")" = "cc [-fuse-ld=lld] [-o] [out file] [main.o]" ]; then
    pass "without mold the system cc links with the original args untouched"
else
    fail "expected plain cc, got exit=$EXIT_CODE log='$(cat "$LOG")'"
fi

echo "test_require_mold_fails_loudly_when_mold_is_missing"
set +e
OUTPUT="$(run_wrapper "$FIXTURE_ROOT/no-mold" EGO_REQUIRE_MOLD=1 2>&1)"
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -ne 0 ] && [ ! -s "$LOG" ] && printf '%s' "$OUTPUT" | grep -q "mold"; then
    pass "EGO_REQUIRE_MOLD=1 refuses to link without mold and says why"
else
    fail "expected a non-zero exit naming mold and no link, got exit=$EXIT_CODE log='$(cat "$LOG")' output='$OUTPUT'"
fi

echo "test_require_mold_links_normally_when_mold_is_installed"
set +e
run_wrapper "$FIXTURE_ROOT/all" EGO_REQUIRE_MOLD=1 >/dev/null 2>&1
EXIT_CODE=$?
set -e
if [ "$EXIT_CODE" -eq 0 ] && [ "$(cat "$LOG")" = "clang [-fuse-ld=lld] [-o] [out file] [main.o] [-fuse-ld=mold]" ]; then
    pass "EGO_REQUIRE_MOLD=1 is a no-op when mold is present"
else
    fail "expected clang with -fuse-ld=mold, got exit=$EXIT_CODE log='$(cat "$LOG")'"
fi

if [ "$FAILURES" -gt 0 ]; then
    echo "test-cargo-linker: $FAILURES failure(s)"
    exit 1
fi
echo "test-cargo-linker: all tests passed"
