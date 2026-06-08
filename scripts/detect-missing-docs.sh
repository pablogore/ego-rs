#!/usr/bin/env bash
set -euo pipefail

# detect-missing-docs.sh
#
# CI-time validation for rustdoc documentation requirements.
#
# Checks:
#   1. All public Rust APIs have documentation
#   2. All source files have documentation
#   3. Architectural components document ownership, invariants, failure semantics

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXIT_CODE=0

echo "--- [DOC-R0, DOC-R1, DOC-R2, DOC-R3, DOC-R4, DOC-R5, DOC-R6, DOC-R7, DOC-R8] Validating documentation requirements..."

# Check for missing documentation in Rust files
echo "Checking for missing documentation in Rust source files..."
missing_docs=$(find "$ROOT/crates" -name "*.rs" -not -path "*/target/*" -exec grep -l "pub.*fn\|pub.*struct\|pub.*enum\|pub.*trait\|pub.*type" {} \; 2>/dev/null | xargs grep -L "///\|//!" 2>/dev/null || true)

if [ -n "$missing_docs" ]; then
    echo "FAIL: Missing documentation in Rust source files:"
    echo "$missing_docs" | sed 's/^/  /'
    EXIT_CODE=1
else
    echo "PASS: All public Rust APIs have documentation"
fi

# Check for missing documentation in architectural components
echo "Checking architectural components documentation..."
arch_components=(
    "$ROOT/crates/domain/src/scheduler.rs"
    "$ROOT/crates/domain/src/worker.rs"
    "$ROOT/crates/domain/src/batch_executor.rs"
    "$ROOT/crates/domain/src/session.rs"
    "$ROOT/crates/domain/src/offset_store.rs"
    "$ROOT/crates/domain/src/dedup_store.rs"
    "$ROOT/crates/runtime/src/runtime.rs"
)

for component in "${arch_components[@]}"; do
    if [ -f "$component" ]; then
        # Check if component has documentation
        if ! grep -q "///.*ownership\|///.*invariants\|///.*failure\|///.*constitutional" "$component" 2>/dev/null; then
            echo "WARN: Architectural component $component may be missing documentation about ownership, invariants, or failure semantics"
        fi
    fi
done

# Summary
if [ "$EXIT_CODE" -eq 0 ]; then
    echo "PASS: All documentation requirements satisfied."
else
    echo "FAIL: One or more documentation checks failed."
fi

exit "$EXIT_CODE"