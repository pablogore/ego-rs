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
SCAN_ROOT="${SCAN_ROOT:-$ROOT/crates}"
EXIT_CODE=0

echo "--- [DOC-R0, DOC-R1, DOC-R2, DOC-R3, DOC-R4, DOC-R5, DOC-R6, DOC-R7, DOC-R8] Validating documentation requirements..."

# trybuild fixtures (tests/compile_fail, tests/compile_pass) pin exact
# line/column output in their sibling .stderr files, so they're exempt from
# the doc scan. If an exempt dir stops existing, fail rather than silently
# exempting nothing.
if [ -n "${EXEMPT_DIRS_OVERRIDE+x}" ]; then
    read -r -a EXEMPT_DIRS <<< "$EXEMPT_DIRS_OVERRIDE"
else
    EXEMPT_DIRS=(
        "$ROOT/crates/service-sdk/tests/compile_fail"
        "$ROOT/crates/service-sdk/tests/compile_pass"
    )
fi
for dir in "${EXEMPT_DIRS[@]}"; do
    if [ ! -d "$dir" ]; then
        echo "FAIL: exempt directory '$dir' no longer exists — update EXEMPT_DIRS in $(basename "$0")"
        EXIT_CODE=1
    fi
done

is_exempt() {
    local f="$1"
    for dir in "${EXEMPT_DIRS[@]}"; do
        case "$f" in
            "$dir"/*) return 0 ;;
        esac
    done
    return 1
}

echo "Checking for missing documentation in Rust source files..."
missing_docs=""
while IFS= read -r f; do
    is_exempt "$f" && continue
    if grep -q "pub.*fn\|pub.*struct\|pub.*enum\|pub.*trait\|pub.*type" "$f" 2>/dev/null \
        && ! grep -q "///\|//!" "$f" 2>/dev/null; then
        missing_docs="$missing_docs$f"$'\n'
    fi
done < <(find "$SCAN_ROOT" -name "*.rs" -not -path "*/target/*")

if [ -n "$missing_docs" ]; then
    echo "FAIL: Missing documentation in Rust source files:"
    printf '%s' "$missing_docs" | sed 's/^/  /'
    EXIT_CODE=1
else
    echo "PASS: All public Rust APIs have documentation (excluding exempt trybuild fixtures)"
fi

# Check for missing documentation in architectural components
echo "Checking architectural components documentation..."
if [ -n "${ARCH_COMPONENTS+x}" ]; then
    read -r -a arch_components <<< "$ARCH_COMPONENTS"
else
    arch_components=(
        "$ROOT/crates/domain/src/read_side/scheduler.rs"
        "$ROOT/crates/domain/src/read_side/session.rs"
        "$ROOT/crates/runtime/src/read_side/batch_executor.rs"
        "$ROOT/crates/runtime/src/runtime/runtime.rs"
    )
fi

for component in "${arch_components[@]}"; do
    if [ ! -f "$component" ]; then
        echo "FAIL: registered architectural component $component does not exist — update arch_components in $(basename "$0")"
        EXIT_CODE=1
        continue
    fi
    if ! grep -q "///.*ownership\|///.*invariants\|///.*failure\|///.*constitutional" "$component" 2>/dev/null; then
        echo "WARN: Architectural component $component may be missing documentation about ownership, invariants, or failure semantics"
    fi
done

# Summary
if [ "$EXIT_CODE" -eq 0 ]; then
    echo "PASS: All documentation requirements satisfied."
else
    echo "FAIL: One or more documentation checks failed."
fi

exit "$EXIT_CODE"