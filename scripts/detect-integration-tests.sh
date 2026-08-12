#!/usr/bin/env bash
set -euo pipefail

# detect-integration-tests.sh
#
# CI-time guard for EGO-RS Unit Test Governance (UT-R2, UT-R4).
#
# Detects forbidden infrastructure usage inside test code:
#   - Testcontainers / Docker references in tests
#   - Embedded databases and brokers in tests
#   - Docker Compose files in the repository
#
# Constitutional references:
#   - CC-R11: No Infrastructure Dependency
#   - CC-R12: Unit-Test Enforcement
#   - UT-R2:  No Real Infrastructure
#   - UT-R4:  No Testcontainers
#
# ---------------------------------------------------------------------------
# Scope: the root workspace, and exactly one carve-out
# ---------------------------------------------------------------------------
#
# The rules above are NOT weakened. Their scope is narrowed to exclude one
# path — `integration-tests/`, an independent Cargo workspace that is not a
# member of the root — and that path gains its own positive check below.
#
# The distinction matters. The root workspace must keep building and testing
# with no Docker, so infrastructure must stay out of it. But the invariants
# that need real PostgreSQL have to live somewhere, and they must version with
# the code they cover. Excluding the path without asserting where the
# infrastructure *is* would turn the carve-out into a blind spot: check 4
# exists so the guard states a positive fact rather than merely stopping.
#
# Check 4 asserts two facts, and both are asked of cargo rather than of text:
# the carve-out declares a dependency on Testcontainers (4a), and cargo does not
# resolve it as a member of the root workspace (4b). Neither question has a
# textual answer — a name can be absent from a members list that still globs the
# path in, and a word can be present in a manifest that declares nothing.
#
# ---------------------------------------------------------------------------
# A note on the pathspec, because this guard was silently dead
# ---------------------------------------------------------------------------
#
# Check 1 previously scanned with the pathspec `'**/tests/'`, which matches
# ZERO files: a pathspec ending in `/` with no file component matches nothing,
# because pathspecs match files and not directories. Measured in this
# repository — `'**/tests/'` matched 0 files, `'*/tests/*'` matched 83.
#
# So the entire forbidden-pattern list had never scanned anything, and every
# green run of this guard reported a safety it had not checked. The only check
# that ever caught the old `crates/integration-tests` was check 3, the
# dev-dependency scan.
#
# `scripts/detect-integration-tests-selftest.sh` now proves the fix by planting
# a known-bad file under a test path and asserting this guard fails. A guard
# that passes because it looks at nothing is worse than no guard at all.

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXIT_CODE=0

# The one path the production checks skip. Kept in a variable so the exclusion
# and the positive check below cannot drift apart.
CARVE_OUT='integration-tests'

# ---------------------------------------------------------------------------
# The membership evaluator, and why it is a separate function
# ---------------------------------------------------------------------------
#
# Reads `cargo metadata --no-deps --format-version 1` on stdin and prints every
# member manifest that lives under the carve-out. Silence means the carve-out is
# not a workspace member.
#
# It is a function taking stdin, rather than inline code, so the self-test can
# drive it with *controlled* metadata: real metadata can only demonstrate the
# passing case, and a check that has only ever been shown to pass is precisely
# what the pathspec note above is about.
#
# `manifest_path` is matched with a whitespace-tolerant pattern rather than by
# assuming cargo's current compact output. The prefix test is anchored on
# `<root>/<carve-out>/` — with the trailing slash, so a sibling named
# `integration-tests-helpers` is not mistaken for a path inside the carve-out.
# `|| true` on the grep is load-bearing under `set -o pipefail`: no match means
# exit 1, which would abort this script rather than report zero violations. That
# is the difference between "nothing is wrong" and "nothing was examined", and
# the self-test pins both directions.
membership_violations() {
    local root="$1" carve_out="$2"
    { grep -o '"manifest_path"[[:space:]]*:[[:space:]]*"[^"]*"' || true; } \
        | sed 's/.*"\([^"]*\)"$/\1/' \
        | while IFS= read -r manifest; do
            case "$manifest" in
                "${root}/${carve_out}/"*) echo "$manifest" ;;
            esac
        done
}

# ---------------------------------------------------------------------------
# The dependency evaluator
# ---------------------------------------------------------------------------
#
# Reads `cargo metadata --no-deps` for the carve-out on stdin and answers one
# question: does any package DECLARE a dependency on Testcontainers?
#
# `dependencies[].name` is cargo's resolved answer, so a commented-out line
# cannot satisfy it. Measured on a throwaway crate whose manifest contained only
# `# testcontainers = "0.24"`: the text matched, and cargo reported `['serde']`.
#
# Parsed with python3 rather than by grepping the JSON. The names we must not
# confuse live in sibling positions — `packages[].name`, `targets[].name` — so a
# flat text scan of this document would match the package's own name or a target
# and answer a different question than the one asked. Hand-rolling that is how a
# check ends up green for the wrong reason.
#
# Exit 0 = declared, 1 = not declared, 2 = metadata unusable. Every non-zero
# outcome is a FAIL at the call site; "unusable" is never "fine".
declares_infra_dependency() {
    python3 -c '
import json, sys

try:
    meta = json.load(sys.stdin)
except Exception as exc:
    print(f"metadata is not usable JSON: {exc}", file=sys.stderr)
    sys.exit(2)

if not isinstance(meta, dict) or "packages" not in meta:
    print("metadata has no packages array", file=sys.stderr)
    sys.exit(2)

names = sorted({
    dep.get("name", "")
    for pkg in meta.get("packages", [])
    for dep in pkg.get("dependencies", [])
})

if any(n == "testcontainers" or n.startswith("testcontainers-") for n in names):
    sys.exit(0)

print("declared dependencies: " + (", ".join(names) or "(none)"), file=sys.stderr)
sys.exit(1)
'
}

# Seams for the self-test, not user-facing modes. Each evaluates stdin and exits
# non-zero on a violation, so the self-test can assert both directions without
# mutating the real workspace.
if [ "${1:-}" = '--eval-membership' ]; then
    found=$(membership_violations "${2:-$ROOT}" "$CARVE_OUT")
    if [ -n "$found" ]; then
        echo "$found"
        exit 1
    fi
    exit 0
fi

if [ "${1:-}" = '--eval-dependency' ]; then
    declares_infra_dependency
    exit $?
fi

echo "--- [CC-R11/CC-R12] Scanning for forbidden infrastructure in tests..."

# ---------------------------------------------------------------------------
# 1. Testcontainers / Docker references in test files
# ---------------------------------------------------------------------------
FORBIDDEN_TEST_PATTERNS=(
    'testcontainers'
    'Testcontainers'
    'EmbeddedPostgres'
    'EmbeddedKafka'
    'EmbeddedRedis'
    'EmbeddedNats'
    'EmbeddedRabbitMQ'
    'EmbeddedElasticsearch'
    'FakeNatsCluster'
    'FakeKafkaCluster'
    'FakeKafkaConsumer'
    'FakeKafkaProducer'
)

for pattern in "${FORBIDDEN_TEST_PATTERNS[@]}"; do
    # `*/tests/*` genuinely matches files under any `tests/` directory. The
    # `:(exclude)` pathspec drops exactly the carve-out and nothing else.
    matches=$(git -C "$ROOT" grep -n "$pattern" -- '*/tests/*' ":(exclude)${CARVE_OUT}/*" 2>/dev/null || true)
    if [ -n "$matches" ]; then
        echo "FAIL: Forbidden test infrastructure pattern detected: '$pattern'"
        echo "$matches" | sed 's/^/  /'
        EXIT_CODE=1
    fi
done

# ---------------------------------------------------------------------------
# 2. Docker Compose files in the repository
# ---------------------------------------------------------------------------
#
# Unchanged in scope, deliberately. Testcontainers is the agreed mechanism;
# there is no reason to permit Compose inside the carve-out either.
COMPOSE_FILES=$(find "$ROOT" -maxdepth 2 \( -name 'docker-compose.yml' -o -name 'docker-compose.yaml' -o -name 'docker-compose.*.yml' -o -name 'docker-compose.*.yaml' \) 2>/dev/null || true)
if [ -n "$COMPOSE_FILES" ]; then
    echo "FAIL: Docker Compose files found in repository (violates UT-R4):"
    echo "$COMPOSE_FILES" | sed 's/^/  /'
    EXIT_CODE=1
fi

# ---------------------------------------------------------------------------
# 3. Dev-dependencies on infrastructure crates in Cargo.toml files
# ---------------------------------------------------------------------------
INFRA_DEV_DEPS=(
    'testcontainers'
    'embedded-postgres'
    'embedded-kafka'
    'embedded-redis'
)

for dep in "${INFRA_DEV_DEPS[@]}"; do
    # `'**/Cargo.toml'` matches every manifest at depth >= 1, so it would match
    # the carve-out's own manifest by construction. Excluded here, asserted in
    # check 4.
    matches=$(git -C "$ROOT" grep -n "$dep" -- '**/Cargo.toml' ":(exclude)${CARVE_OUT}/*" 2>/dev/null || true)
    if [ -n "$matches" ]; then
        echo "FAIL: Infrastructure dev-dependency detected: '$dep'"
        echo "$matches" | sed 's/^/  /'
        EXIT_CODE=1
    fi
done

# ---------------------------------------------------------------------------
# 4. Positive check — infrastructure lives in the carve-out, and only there
# ---------------------------------------------------------------------------
#
# Checks 1 and 3 stop looking at one path. This one asserts what is true of it,
# so the exclusion is a stated boundary rather than an unexamined hole.
#
# Skipped entirely when the carve-out does not exist, so this guard keeps
# working on a tree where the infrastructure suite has not been scaffolded yet.
if [ -d "$ROOT/$CARVE_OUT" ]; then
    # 4a. The carve-out must DECLARE the infrastructure it is excused for.
    #
    # Asked of cargo, not of the manifest text. Two earlier revisions of this
    # check were text searches, and each was green for a reason that was not the
    # fact:
    #
    # - Directory-wide, it stayed green with the dependency deleted, on the
    #   strength of a `use testcontainers::...` line in a test file.
    # - Scoped to the manifest, it stayed green on a commented-out line. A
    #   `# testcontainers = "0.24"` declares nothing; cargo reported `['serde']`
    #   for a manifest that matched the grep.
    #
    # The exclusion is justified by the suite *depending* on the infrastructure,
    # and only the resolved dependency list settles that.
    if ! command -v python3 >/dev/null 2>&1; then
        echo "FAIL: python3 is required to read the carve-out's dependency graph."
        echo "  This check refuses to fall back to a text search: that is the"
        echo "  very substitution it exists to remove."
        EXIT_CODE=1
    elif ! carve_metadata=$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 --offline \
        --manifest-path "${CARVE_OUT}/Cargo.toml" 2>&1) &&
        ! carve_metadata=$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 \
            --manifest-path "${CARVE_OUT}/Cargo.toml" 2>&1); then
        echo "FAIL: could not read '${CARVE_OUT}/Cargo.toml' through cargo."
        echo "  An unreadable manifest is an unverified one."
        while IFS= read -r line; do echo "  $line"; done <<<"$carve_metadata"
        EXIT_CODE=1
    elif ! dependency_report=$(printf '%s' "$carve_metadata" | declares_infra_dependency 2>&1); then
        echo "FAIL: '${CARVE_OUT}/Cargo.toml' declares no Testcontainers dependency."
        echo "  The carve-out is excluded from checks 1 and 3 because that is"
        echo "  where infrastructure-backed tests are supposed to live. A"
        echo "  carve-out that does not depend on the infrastructure is an"
        echo "  exclusion protecting nothing — either the suite moved, or the"
        echo "  exclusion should go."
        while IFS= read -r line; do echo "  $line"; done <<<"$dependency_report"
        EXIT_CODE=1
    fi

    # 4b. The carve-out must not be a root workspace member, or `cargo test
    # --workspace` at the root would compile and run it — the exact coupling
    # that made the old suite untenable.
    #
    # Asked of cargo, not of the manifest text. The previous version grepped the
    # root manifest for the literal string `"integration-tests`, which is green
    # for any membership that never spells the name. Measured: with a glob
    # (`members = ["*"]`) and no `[workspace]` table in the carve-out's own
    # manifest, cargo resolves it as a member and the text search sees nothing.
    # The name is not the fact; the resolved member list is.
    #
    # A metadata failure is a FAIL, never a pass. Empty output from a command
    # that errored is indistinguishable from a clean result, and treating the
    # two alike is how check 1 came to scan nothing. This also covers the loud
    # variant of the same mistake: a glob *plus* the nested `[workspace]` table
    # makes cargo refuse with "multiple workspace roots found in the same
    # workspace", which must surface here rather than be swallowed.
    # `--offline` first because this guard must not depend on the network, then a
    # plain retry so a tree whose lockfile cargo wants to touch is not reported
    # as a membership failure it is not.
    if ! metadata=$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 --offline 2>&1) &&
        ! metadata=$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 2>&1); then
        echo "FAIL: could not read the root workspace membership from cargo."
        echo "  This check cannot pass without that answer — an unreadable"
        echo "  workspace is an unverified one."
        while IFS= read -r line; do echo "  $line"; done <<<"$metadata"
        EXIT_CODE=1
    else
        offenders=$(printf '%s' "$metadata" | membership_violations "$ROOT" "$CARVE_OUT")
        if [ -n "$offenders" ]; then
            echo "FAIL: '${CARVE_OUT}' resolves as a member of the ROOT workspace:"
            while IFS= read -r line; do echo "  $line"; done <<<"$offenders"
            echo "  The root workspace must neither compile nor run it, so that"
            echo "  'cargo test --workspace' stays hermetic and needs no Docker."
            echo "  Restore the carve-out's own [workspace] table, or stop"
            echo "  globbing it into the root's members list."
            EXIT_CODE=1
        fi
    fi
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
if [ "$EXIT_CODE" -eq 0 ]; then
    echo "PASS: No forbidden infrastructure usage detected in tests."
else
    echo "FAIL: One or more infrastructure dependency violations detected."
fi

exit "$EXIT_CODE"
