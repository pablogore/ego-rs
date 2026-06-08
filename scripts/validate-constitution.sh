#!/usr/bin/env bash
set -euo pipefail

# validate-constitution.sh
#
# Single entry point for all constitutional validation checks.
# This script orchestrates all validation layers and ensures
# that the system complies with the EGO-RS Constitution.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXIT_CODE=0

echo "=== EGO-RS Constitution Validation ==="

# Run all validation scripts in sequence
echo ""
echo "1. Running CI-time validations..."
scripts/detect-violations.sh || EXIT_CODE=1

echo ""
echo "2. Running documentation validation..."
scripts/detect-missing-docs.sh || EXIT_CODE=1

echo ""
echo "3. Running test quality validation..."
scripts/detect-test-smells.sh || EXIT_CODE=1

echo ""
echo "4. Running mock-only test detection..."
scripts/detect-mock-only-tests.sh || EXIT_CODE=1

echo ""
echo "5. Running coverage validation..."
scripts/verify-coverage.sh || EXIT_CODE=1

echo ""
echo "6. Running constitution mapping validation..."
scripts/verify-constitution-mapping.sh || EXIT_CODE=1

echo ""
echo "7. Running integration test validation..."
scripts/detect-integration-tests.sh || EXIT_CODE=1

# Summary
echo ""
if [ "$EXIT_CODE" -eq 0 ]; then
    echo "✅ ALL CONSTITUTIONAL VALIDATIONS PASSED"
    echo "The system complies with the EGO-RS Constitution."
else
    echo "❌ SOME CONSTITUTIONAL VALIDATIONS FAILED"
    echo "Please fix the violations above before proceeding."
fi

exit "$EXIT_CODE"
