#!/usr/bin/env bash
set -euo pipefail

# verify-coverage.sh
#
# Constitutional check for MT-R7: "Coverage metrics are necessary but not
# sufficient."
#
# Both halves of that rule are enforced here, and it is worth being explicit
# about which is which:
#
#   necessary      Coverage MUST be measured. If it cannot be, this script
#                  fails. An unmeasured codebase is not a passing one, and
#                  reporting PASS because the tool was missing is how a gate
#                  stops meaning anything.
#
#   not sufficient A percentage says nothing about whether the tests assert
#                  anything worth asserting. That is MT-R8 and PC-R1..PC-R9,
#                  and they belong to detect-test-smells.sh. This script does
#                  NOT check them and must not claim to.
#
# # What this replaced, and why it is worth recording
#
# The previous version could not fail. `EXIT_CODE` was initialised to 0 and
# never assigned again, so `exit "$EXIT_CODE"` was a constant. On top of that:
#
#   - it never `cd`-ed to the repository root, so cargo resolved the manifest
#     against the caller's directory and running it from `scripts/` looked for
#     `scripts/Cargo.toml`;
#   - `cargo tarpaulin ... || true` discarded the tool's failure outright;
#   - it then looked for `lcov.info` or `coverage.xml`, neither of which
#     `--out Xml` produces — the real file is `cobertura.xml` — so even a
#     completely successful run took the "no coverage files generated" branch;
#   - and that branch printed a message without failing;
#   - its only real assertion was that files named `*test*.rs` contain more
#     than zero lines, which is not coverage by any reading;
#   - its header claimed line and branch coverage floors of 85% that no code
#     checked, while `make test-cov` names a third number, 95.
#
# It also left `cobertura.xml` untracked in the repository root, so a
# verification script dirtied the working tree it was verifying.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# The floor the tree actually holds, measured rather than aspired to.
#
# 2026-08-19: `cargo tarpaulin --workspace` reported 66.30% (5924/8935 lines).
# The floor is that number rounded down, so this gate fails when coverage drops
# and passes when it does not.
#
# Raising it is a deliberate act, never an automatic one: a floor that follows
# the measurement upward on its own would ratchet silently and turn an unrelated
# refactor into a gate failure nobody chose. Move this number when coverage
# genuinely rises, in a change that says so.
#
# This is also the only place the floor is written down, and the correction is
# worth stating precisely because the first version of this comment got it
# wrong. The 85% in this script's own former header was enforced by nothing. The
# 95% in `make test-cov` was different: tarpaulin's `--fail-under` does enforce
# it, so at 66.30% that target exited non-zero on every run. A gate that always
# fails and a gate that cannot fail are the same defect seen from two sides —
# both stop carrying information, and both train people to skip them. The
# Makefile now asks this script for the number, so the two cannot disagree.
COVERAGE_FLOOR=66

# Published rather than duplicated, and answered before any measurement work so
# that asking stays free.
if [ "${1:-}" = "--print-floor" ]; then
    echo "$COVERAGE_FLOOR"
    exit 0
fi

# Kept out of the working tree. `target/` is gitignored; the repository root is
# not, and the previous version wrote there.
OUTPUT_DIR="$ROOT/target/coverage"

echo "--- [MT-R7] Verifying coverage requirements..."

if ! command -v cargo-tarpaulin >/dev/null 2>&1; then
    echo "FAIL: coverage is UNMEASURED — cargo-tarpaulin is not installed."
    echo "      MT-R7 makes measurement necessary, so an unmeasured tree does"
    echo "      not pass. Install it with: cargo install cargo-tarpaulin"
    exit 1
fi

echo "Measuring coverage across the workspace (this is the slow check)..."

# `cd` first: cargo resolves the manifest against the current directory, and
# this script is routinely invoked from elsewhere.
cd "$ROOT"

# Tarpaulin writes nothing and reports no error when `--output-dir` names a
# directory that does not exist, so it is created rather than assumed. Found by
# checking for the report instead of trusting the flag.
mkdir -p "$OUTPUT_DIR"

# The tool's failure is captured, never discarded. `set -e` would abort here on
# a non-zero status before it could be classified, so it is read explicitly.
set +e
report="$(cargo tarpaulin --workspace --timeout 120 --out Xml --output-dir "$OUTPUT_DIR" 2>&1)"
tarpaulin_status=$?
set -e

echo "$report"

if [ "$tarpaulin_status" -ne 0 ]; then
    echo "FAIL: coverage is UNMEASURED — cargo tarpaulin exited ${tarpaulin_status}."
    echo "      This is a tooling or build failure, not a statement about"
    echo "      coverage. The output above is the tool's own."
    exit 1
fi

# Tarpaulin's summary line reads e.g.
#   66.30% coverage, 5924/8935 lines covered, +54.56% change in coverage
percent="$(printf '%s\n' "$report" | grep -oE '[0-9]+\.[0-9]+% coverage' | tail -1 | grep -oE '[0-9]+\.[0-9]+' || true)"

if [ -z "$percent" ]; then
    echo "FAIL: coverage is UNMEASURED — tarpaulin exited 0 but reported no"
    echo "      coverage percentage. Its summary line may have changed format;"
    echo "      this check must not guess a number it did not read."
    exit 1
fi

# Decimal comparison, done in awk because the shell cannot do it at all:
# `[ 65.99 -lt 66 ]` is a syntax error, not a false. Truncating to an integer
# first would work for a whole-number floor like this one, but it silently
# stops working the moment someone writes a fractional floor — so the
# comparison is done on the real values rather than on a convenience.
if awk -v got="$percent" -v floor="$COVERAGE_FLOOR" 'BEGIN { exit !(got < floor) }'; then
    echo "FAIL: coverage ${percent}% is below the declared floor of ${COVERAGE_FLOOR}%."
    echo "      Coverage went down. Either restore it, or move the floor"
    echo "      deliberately in a change that explains why."
    exit 1
fi

echo "PASS: coverage ${percent}%, at or above the declared floor of ${COVERAGE_FLOOR}%."

# Stated only when it is there. Naming a path this script did not confirm is the
# same class of claim the old version made about `lcov.info`.
if [ -f "${OUTPUT_DIR}/cobertura.xml" ]; then
    echo "      Report: ${OUTPUT_DIR}/cobertura.xml"
else
    echo "      No XML report was written; the percentage above is tarpaulin's"
    echo "      own summary, which is what this gate reads."
fi
echo "NOTE: MT-R7 is only half-satisfied by this number. Whether those tests"
echo "      assert anything meaningful is MT-R8 and PC-R1..PC-R9, enforced by"
echo "      detect-test-smells.sh — not here."

exit 0
