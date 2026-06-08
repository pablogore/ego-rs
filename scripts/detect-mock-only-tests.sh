#!/usr/bin/env bash
set -euo pipefail

# detect-mock-only-tests.sh
#
# CI-time validation to detect tests that only verify mock calls
# without asserting observable behavior.
#
# This is a basic implementation that looks for common patterns
# indicating mock-only tests.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXIT_CODE=0

echo "--- [MT-R3] Detecting mock-only tests..."

# Find test files
test_files=$(find "$ROOT/crates" -name "*test*.rs" -not -path "*/target/*" 2>/dev/null || true)

if [ -n "$test_files" ]; then
    echo "Checking for mock-only tests..."
    
    # Look for tests that only verify mock calls without asserting behavior
    mock_only_indicators=(
        "expect.*call"
        "mock.*verify"
        "verify.*called"
        "should_panic"
        "assert_eq.*mock"
        "assert.*mock"
    )
    
    for file in $test_files; do
        # Skip if file is too large or binary
        if [ -f "$file" ] && [ "$(wc -l < "$file")" -lt 1000 ]; then
            for pattern in "${mock_only_indicators[@]}"; do
                if grep -q "$pattern" "$file" 2>/dev/null; then
                    # Check if there are also assertions about behavior
                    has_behavior_assertion=$(grep -E "assert.*eq|assert.*true|assert.*false|should_panic|expect.*error" "$file" 2>/dev/null || true)
                    if [ -z "$has_behavior_assertion" ]; then
                        echo "WARN: Potential mock-only test in $file (only verifies mock calls)"
                    fi
                fi
            done
        fi
    done
fi

echo "PASS: Mock-only test detection completed."

exit "$EXIT_CODE"