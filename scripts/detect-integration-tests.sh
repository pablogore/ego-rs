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
    if [ -z "$(git -C "$ROOT" grep -l 'testcontainers' -- "${CARVE_OUT}/*" 2>/dev/null || true)" ]; then
        echo "FAIL: '${CARVE_OUT}/' exists but declares no Testcontainers usage."
        echo "  The carve-out is excluded from checks 1 and 3 because that is"
        echo "  where infrastructure-backed tests are supposed to live. A"
        echo "  carve-out holding no such tests is an exclusion protecting"
        echo "  nothing — either the suite moved, or the exclusion should go."
        EXIT_CODE=1
    fi

    # The carve-out must not be a root workspace member, or `cargo test
    # --workspace` at the root would compile and run it — the exact coupling
    # that made the old suite untenable.
    if git -C "$ROOT" grep -q "\"${CARVE_OUT}" -- 'Cargo.toml' 2>/dev/null; then
        if git -C "$ROOT" grep -n "\"${CARVE_OUT}" -- 'Cargo.toml' | grep -qv 'exclude'; then
            echo "FAIL: '${CARVE_OUT}' appears in the root workspace manifest outside an exclude."
            echo "  The root workspace must neither compile nor run it."
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
