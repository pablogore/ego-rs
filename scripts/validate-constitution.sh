#!/usr/bin/env bash
set -euo pipefail

# validate-constitution.sh
#
# Single entry point for all constitutional validation checks.
# This script orchestrates all validation layers and ensures
# that the system complies with the EGO-RS Constitution.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CHECKS_DIR="${CHECKS_DIR:-$ROOT/scripts}"
UNAVAILABLE=()
FAILED=()

run_check() {
    local label="$1"
    local name="$2"
    local path="$CHECKS_DIR/$name.sh"

    echo ""
    echo "$label"
    if [ ! -x "$path" ]; then
        echo "⚠️  $name.sh is missing or not executable"
        UNAVAILABLE+=("$name")
        return
    fi
    if ! "$path"; then
        FAILED+=("$name")
    fi
}

echo "=== EGO-RS Constitution Validation ==="

# Run all validation scripts in sequence
run_check "1. Running CI-time validations..." detect-violations
run_check "2. Running documentation validation..." detect-missing-docs
run_check "3. Running test quality validation..." detect-test-smells
run_check "4. Running mock-only test detection..." detect-mock-only-tests
run_check "5. Running coverage validation..." verify-coverage
run_check "6. Running constitution mapping validation..." verify-constitution-mapping
run_check "7. Running integration test validation..." detect-integration-tests

# Summary
echo ""
if [ ${#UNAVAILABLE[@]} -gt 0 ]; then
    echo "⚠️  UNAVAILABLE (script missing or not executable):"
    printf '   - %s\n' "${UNAVAILABLE[@]}"
fi
if [ ${#FAILED[@]} -gt 0 ]; then
    echo "❌ FAILED (ran and reported a violation):"
    printf '   - %s\n' "${FAILED[@]}"
fi

if [ ${#UNAVAILABLE[@]} -eq 0 ] && [ ${#FAILED[@]} -eq 0 ]; then
    echo "✅ ALL CONSTITUTIONAL VALIDATIONS PASSED"
    echo "The system complies with the EGO-RS Constitution."
    exit 0
fi

echo "Please fix the violations above before proceeding."
exit 1
