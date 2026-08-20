#!/usr/bin/env bash
set -euo pipefail

# validate-constitution.sh
#
# Single entry point for all constitutional validation checks. It orchestrates
# the validation layers and reports whether the system complies with the EGO-RS
# Constitution.
#
# # What this replaced
#
# The orchestrator computed `ROOT` and then invoked each check as
# `scripts/<name>.sh` — a path relative to the caller's directory, not to the
# repository. Run from anywhere other than the repository root, every one of the
# seven checks failed to launch with "No such file or directory", and the script
# reported "SOME CONSTITUTIONAL VALIDATIONS FAILED".
#
# That is the worst shape this kind of failure can take. It does not merely fail
# to check; it reports *violations* when what actually happened is that nothing
# ran. An operator reading that output would go looking for constitutional
# breaches that were never detected, and a run from `scripts/` — the directory
# the scripts live in, and the obvious place to run them from — was guaranteed
# to produce it.
#
# Two things changed. Each check is invoked by absolute path so the caller's
# directory is irrelevant, and a check that could not be launched at all is
# reported separately from one that ran and failed. Those are different facts
# and only one of them is about the codebase.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPTS="$ROOT/scripts"

failed=()
unavailable=()

echo "=== EGO-RS Constitution Validation ==="

# Runs one check by absolute path, classifying the three outcomes that matter:
# it passed, it ran and failed, or it never ran.
run_check() {
    local label="$1"
    local script="$2"
    local path="$SCRIPTS/$script"

    echo ""
    echo "${label}"

    if [ ! -x "$path" ]; then
        echo "UNAVAILABLE: $script is missing or not executable at $path"
        unavailable+=("$script")
        return
    fi

    # `set -e` would abort the orchestrator on the first failing check, which
    # would hide every check after it. Each status is read explicitly so all of
    # them run and the summary is complete.
    if "$path"; then
        return
    fi

    failed+=("$script")
}

run_check "1. Running CI-time validations..."          "detect-violations.sh"
run_check "2. Running documentation validation..."     "detect-missing-docs.sh"
run_check "3. Running test quality validation..."      "detect-test-smells.sh"
run_check "4. Running mock-only test detection..."     "detect-mock-only-tests.sh"
run_check "5. Running coverage validation..."          "verify-coverage.sh"
run_check "6. Running constitution mapping validation..." "verify-constitution-mapping.sh"
run_check "7. Running integration test validation..."  "detect-integration-tests.sh"

echo ""

if [ "${#unavailable[@]}" -gt 0 ]; then
    echo "❌ CONSTITUTIONAL VALIDATION COULD NOT COMPLETE"
    echo "These checks never ran, so nothing is known about what they cover:"
    printf '  - %s\n' "${unavailable[@]}"
fi

if [ "${#failed[@]}" -gt 0 ]; then
    echo "❌ CONSTITUTIONAL VALIDATIONS FAILED"
    echo "These checks ran and reported violations:"
    printf '  - %s\n' "${failed[@]}"
    echo "Their own output above names what to fix."
fi

if [ "${#unavailable[@]}" -eq 0 ] && [ "${#failed[@]}" -eq 0 ]; then
    echo "✅ ALL CONSTITUTIONAL VALIDATIONS PASSED"
    echo "The system complies with the EGO-RS Constitution."
    exit 0
fi

exit 1
