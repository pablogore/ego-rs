#!/usr/bin/env bash
set -euo pipefail

# detect-missing-docs.sh
#
# Constitutional check for the DOC rules.
#
#   DOC-R1  All Rust source files MUST contain rustdoc documentation
#   DOC-R2  Documentation is mandatory for both public and private APIs
#   DOC-R4  Undocumented source files are constitutional violations
#   DOC-R5  CI MUST fail if required rustdoc is missing
#   DOC-R6  Documentation exemptions MUST be explicit
#   DOC-R3  Architectural components MUST document ownership, invariants,
#   DOC-R8  failure semantics and the constitutional rules they enforce
#
# # What this replaced
#
# The file scan had no notion of an exemption, so it failed on thirteen
# `trybuild` fixtures that cannot be documented — see the registry below — and
# the whole check had been red for long enough that nobody ran the orchestrator.
# A gate that fails for a case it cannot fix teaches people to skip it, which is
# how it stops protecting anything.
#
# The architectural half was worse: it was inert. All seven of its hardcoded
# paths were wrong — `scheduler.rs` and `session.rs` live under `read_side/`,
# `runtime.rs` under `runtime/`, `batch_executor.rs` under
# `runtime/src/read_side/`, and `worker.rs`, `offset_store.rs` and
# `dedup_store.rs` do not exist as files at all — and the loop body was guarded
# by `[ -f "$component" ]`, so it never executed once. DOC-R3 and DOC-R8 were
# enforced by nothing.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXIT_CODE=0

echo "--- [DOC-R1..DOC-R8] Validating documentation requirements..."

# ---------------------------------------------------------------------------
# DOC-R6: the one explicit exemption, and why it is not a convenience
# ---------------------------------------------------------------------------
#
# `trybuild` fixtures under `tests/compile_fail/` and `tests/compile_pass/`.
#
# Their expected output is pinned in a sibling `.stderr` file that quotes exact
# line and column positions. Measured rather than assumed: adding a single
# `//!` line to `compile_fail/idempotent_without_operation.rs` moved every
# diagnostic down by one — `:35:14` became `:36:14`, `:9:5` became `:10:5`,
# `:12:30` became `:13:30` — and `cargo test -p ego-service-sdk --test
# idempotent_marker` failed with `mismatch`, exit 101. Documenting these files
# does not merely add noise; it breaks the suite that depends on them.
#
# The exemption is a directory rule rather than a list of filenames on purpose:
# a list would go stale every time a fixture is added, and a stale exemption is
# worse than none.
EXEMPT_DIRS=(
    "tests/compile_fail"
    "tests/compile_pass"
)

is_exempt() {
    local path="$1"
    local dir
    for dir in "${EXEMPT_DIRS[@]}"; do
        case "$path" in
        *"/$dir/"*) return 0 ;;
        esac
    done
    return 1
}

# ---------------------------------------------------------------------------
# DOC-R1, DOC-R2, DOC-R4, DOC-R5: every source file carries documentation
# ---------------------------------------------------------------------------

echo "Checking for missing documentation in Rust source files..."

declared_files=0
undocumented=""
exempted=0

while IFS= read -r file; do
    # Only files that declare something are in scope; a fixture that merely
    # calls a function declares no API and has nothing to document.
    if ! grep -qE 'pub[[:space:]].*(fn|struct|enum|trait|type)' "$file" 2>/dev/null; then
        continue
    fi
    declared_files=$((declared_files + 1))

    if grep -qE '^\s*(///|//!)' "$file" 2>/dev/null; then
        continue
    fi

    if is_exempt "$file"; then
        exempted=$((exempted + 1))
        continue
    fi

    undocumented="${undocumented}  ${file#"$ROOT"/}"$'\n'
done < <(find "$ROOT/crates" -name "*.rs" -not -path "*/target/*")

if [ -n "$undocumented" ]; then
    echo "FAIL: Rust source files declaring an API with no documentation:"
    printf '%s' "$undocumented"
    echo "      Document them, or — if they genuinely cannot be documented —"
    echo "      add an explicit exemption to EXEMPT_DIRS with the reason."
    EXIT_CODE=1
else
    echo "PASS: every declaring source file is documented (${declared_files} scanned, ${exempted} explicitly exempt)"
fi

# The exemption must not go stale in the other direction either: a rule that
# matches nothing is a rule nobody would notice had stopped applying.
if [ "$exempted" -eq 0 ]; then
    echo "FAIL: the DOC-R6 exemption matched no file."
    echo "      Either the fixture directories moved, or the exemption is no"
    echo "      longer needed and should be removed rather than left standing."
    EXIT_CODE=1
fi

# ---------------------------------------------------------------------------
# DOC-R3, DOC-R8: architectural components
# ---------------------------------------------------------------------------
#
# Paths corrected against the tree. DOC-R3 also names Worker, Offset Store and
# Dedup Store; no file in this repository corresponds to them today, so they are
# deliberately absent from this registry and must be added when they appear.
ARCH_COMPONENTS=(
    "crates/domain/src/read_side/scheduler.rs"
    "crates/domain/src/read_side/session.rs"
    "crates/runtime/src/read_side/batch_executor.rs"
    "crates/runtime/src/runtime/runtime.rs"
)

echo "Checking architectural components documentation..."

for component in "${ARCH_COMPONENTS[@]}"; do
    # Fail-closed on rot. This is the defect that made the old block inert: it
    # skipped silently when a path stopped resolving, so moving a file quietly
    # retired its check.
    if [ ! -f "$ROOT/$component" ]; then
        echo "FAIL: registered architectural component does not exist: $component"
        echo "      It moved or was removed. Update this registry — a component"
        echo "      list that names missing files checks nothing."
        EXIT_CODE=1
        continue
    fi

    if ! grep -qEi '(///|//!).*(ownership|invariant|failure|constitutional)' "$ROOT/$component" 2>/dev/null; then
        echo "WARN: $component does not mention ownership, invariants, failure"
        echo "      semantics or a constitutional reference in its rustdoc."
    fi
done

# Stated rather than implied: the check above greps for four words. That
# correlates with DOC-R3/DOC-R8 and does not establish them — whether the prose
# actually describes ownership and failure semantics is a review question, which
# is why it warns rather than fails.
echo "NOTE: the architectural check is a keyword heuristic. It can show that a"
echo "      component says nothing about ownership or failure; it cannot show"
echo "      that what it does say is adequate."

if [ "$EXIT_CODE" -eq 0 ]; then
    echo "PASS: All documentation requirements satisfied."
else
    echo "FAIL: One or more documentation checks failed."
fi

exit "$EXIT_CODE"
