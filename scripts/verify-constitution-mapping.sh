#!/usr/bin/env bash
set -euo pipefail

# verify-constitution-mapping.sh
#
# Governance checks that need the toolchain: the workspace builds, its tests
# pass, clippy is clean and the tree is formatted.
#
# # What this enforces
#
#   cargo test --workspace
#   cargo clippy --workspace
#   cargo fmt --all -- --check
#
# # What this does NOT enforce, despite what its header used to claim
#
# The previous header advertised eight checks. Three of them existed. These five
# were named and implemented by nothing at all, and are listed here so the gap is
# visible instead of hidden behind a passing gate:
#
#   - all tasks have required fields
#   - all tasks complete with evidence
#   - coverage >= 85%          (coverage lives in verify-coverage.sh, against a
#                               measured floor rather than this aspiration)
#   - workflow stages not skipped
#   - contract versions bumped on breaking changes
#
# A gate that claims eight checks and runs three is not a partial gate; it is a
# misleading one, because the seven-check summary reads as coverage that does not
# exist.
#
# # What this replaced
#
# The script could not fail. `EXIT_CODE` was initialised to 0 and never assigned
# again, so `exit "$EXIT_CODE"` was a constant, and every check was written to
# match: `cargo test`, `cargo clippy` and `cargo fmt` each degraded a failure to
# "WARN: ... (may be expected in some contexts)". Which contexts were never
# named, and no context makes a failing test suite acceptable to a governance
# gate.
#
# Two of its checks were worse than permissive:
#
#   - `cargo fmt --check` was run from wherever the caller happened to be, and
#     `cargo fmt` does not search ancestor directories for a manifest the way
#     cargo test and clippy do. Measured: with a real formatting violation in the
#     tree, `cargo fmt --check` from the repository root exits 1 and does catch
#     it, but from `scripts/` it exits 1 with "Failed to find targets" — a
#     tooling error, not a formatting verdict, and the old code reported both as
#     the same WARN. `--all` is added for explicitness rather than to fix a live
#     hole: this workspace declares no `default-members`, so the default set and
#     the full set coincide today, and they would silently stop coinciding the
#     moment someone adds one.
#
#   - `incomplete_tasks=$(grep -c ... || echo 0)` produced two values, not one.
#     `grep -c` prints `0` AND exits 1 when nothing matches, so the `|| echo 0`
#     ran as well and the variable held `0\n0`. `[ "$incomplete_tasks" -gt 0 ]`
#     then failed with "integer expression expected", which made the `if` false
#     and sent control to the branch that prints PASS. The check reported success
#     by way of its own error.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXIT_CODE=0

# cargo resolves the manifest against the current directory. `cargo fmt` in
# particular does not search ancestors, so this is what makes the script's result
# independent of where it was invoked from.
cd "$ROOT"

echo "--- [G-R1 through G-R8] Verifying constitution mapping and governance..."

# ---------------------------------------------------------------------------
# Toolchain gates
# ---------------------------------------------------------------------------
#
# stderr is deliberately not silenced. When one of these fails, the tool's own
# diagnostic is the only thing that tells the operator what to fix.
#
# Note that step 5 of the orchestrator already ran the whole suite under
# tarpaulin, so `cargo test` here repeats it. That duplication is left alone
# rather than optimised away, because an instrumented run and a plain one are not
# the same evidence and collapsing them is a decision for a change that says so.

run_cargo() {
    local label="$1"
    shift

    echo "Checking ${label}..."
    if "$@"; then
        echo "PASS: ${label}"
        return
    fi

    echo "FAIL: ${label} — see its output above."
    EXIT_CODE=1
}

run_cargo "cargo test"   cargo test --workspace --quiet
run_cargo "cargo clippy" cargo clippy --workspace --quiet
run_cargo "cargo fmt"    cargo fmt --all -- --check

# ---------------------------------------------------------------------------
# AGENTS.md
# ---------------------------------------------------------------------------
#
# Fails closed when absent. Both blocks below used to be wrapped in
# `if [ -f ... ]`, so a missing or renamed AGENTS.md retired them in silence —
# the same shape of defect as a component registry that skips paths it cannot
# resolve.

if [ ! -f "$ROOT/AGENTS.md" ]; then
    echo "FAIL: AGENTS.md is missing, so the governance checks below did not run."
    echo "      Restore it, or move these checks to wherever the tasks now live."
    EXIT_CODE=1
else
    # Informational, and labelled as such. Counting occurrences of "evidence:"
    # says nothing about whether the evidence is present or adequate; this line
    # reports a number and asserts nothing, which is why it cannot fail.
    evidence_count=$(grep -c "evidence:" "$ROOT/AGENTS.md" || true)
    echo "INFO: ${evidence_count} 'evidence:' entries in AGENTS.md (a count, not an assertion)."

    # A keyword heuristic, and it warns rather than fails for the same reason the
    # architectural scan in detect-missing-docs.sh warns: the words "incomplete"
    # and "pending" appearing in a document do not establish that a task is
    # unfinished, and failing on prose would produce noise nobody could act on.
    incomplete_tasks=$(grep -c "incomplete\|pending" "$ROOT/AGENTS.md" || true)
    if [ "$incomplete_tasks" -gt 0 ]; then
        echo "WARN: ${incomplete_tasks} line(s) in AGENTS.md mention 'incomplete' or"
        echo "      'pending'. This is a keyword match, not a finding."
    else
        echo "PASS: no 'incomplete' or 'pending' mentions in AGENTS.md"
    fi
fi

if [ "$EXIT_CODE" -eq 0 ]; then
    echo "PASS: Constitution mapping and governance validation completed."
else
    echo "FAIL: One or more governance checks failed."
fi

exit "$EXIT_CODE"
