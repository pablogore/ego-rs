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

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXIT_CODE=0

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
    matches=$(git -C "$ROOT" grep -n "$pattern" -- '**/tests/' 2>/dev/null || true)
    if [ -n "$matches" ]; then
        echo "FAIL: Forbidden test infrastructure pattern detected: '$pattern'"
        echo "$matches" | sed 's/^/  /'
        EXIT_CODE=1
    fi
done

# ---------------------------------------------------------------------------
# 2. Docker Compose files in the repository
# ---------------------------------------------------------------------------
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
    matches=$(git -C "$ROOT" grep -n "$dep" -- '**/Cargo.toml' 2>/dev/null || true)
    if [ -n "$matches" ]; then
        echo "FAIL: Infrastructure dev-dependency detected: '$dep'"
        echo "$matches" | sed 's/^/  /'
        EXIT_CODE=1
    fi
done

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
if [ "$EXIT_CODE" -eq 0 ]; then
    echo "PASS: No forbidden infrastructure usage detected in tests."
else
    echo "FAIL: One or more infrastructure dependency violations detected."
fi

exit "$EXIT_CODE"
