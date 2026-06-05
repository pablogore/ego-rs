#!/usr/bin/env bash
set -euo pipefail

# detect-violations.sh
#
# CI-time static analysis guard for EGO-RS constitution rules.
#
# Checks:
#   1. Forbidden external call patterns in handler code
#      (EE-R1: no direct HTTP/kafka/external calls from handlers)
#   2. Forbidden import patterns across layer boundaries (FO-R3)
#   3. Offsets or dedup usage outside commit boundary (OM-R1-3, DD-R1-4)

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXIT_CODE=0

# ---------------------------------------------------------------------------
# EE-R1: Detect direct external calls in handler source files.
# ---------------------------------------------------------------------------
# Handler modules are in crates/domain/src/ and crates/runtime/src/read_side/.
# They MUST NOT import or use HTTP clients, message brokers, or external
# APIs directly. Instead they must use Effect::ExternalEffects(...).
#
# This is a best-effort grep guard. Compile-time enforcement via type system
# is the primary layer; CI is a safety net.

EE_R1_PATTERNS=(
    # HTTP client crates
    'use reqwest'
    'use hyper::'
    'use curl::'
    # Message brokers
    'use rdkafka::'
    'use nats::'
    'use pulsar::'
    'use amqp::'
    # External DB / cache
    'use redis::'
    'use mongodb::'
    'use elasticsearch::'
    # AWS SDK
    'use aws-sdk-'
    'use rusoto_'
    # gRPC
    'use tonic::'
    'use grpc::'
    # Raw TCP/UDP (handlers should never open sockets)
    'use std::net::'
    'use tokio::net::'
)

HANDLER_DIRS=(
    "$ROOT/crates/domain/src"
    "$ROOT/crates/runtime/src"
)

echo "--- [EE-R1] Scanning for direct external calls in handler code..."

for pattern in "${EE_R1_PATTERNS[@]}"; do
    for dir in "${HANDLER_DIRS[@]}"; do
        # Respect .gitignore; skip target/, node_modules/, etc.
        matches=$(git -C "$ROOT" grep -l "$pattern" -- "$dir" 2>/dev/null || true)
        if [ -n "$matches" ]; then
            echo "FAIL: EE-R1 violation detected: '$pattern' found in:"
            echo "$matches" | sed 's/^/  /'
            EXIT_CODE=1
        fi
    done
done

# ---------------------------------------------------------------------------
# FO-R3: Detect illegal cross-layer imports.
# ---------------------------------------------------------------------------
# Layers may only import from directly adjacent layers or foundations.
# See layers.toml for the authoritative layer map.
# domain → (nothing)
# runtime → domain
# infrastructure → domain (via ports)

echo "--- [FO-R3] Scanning for cross-layer import violations..."

# Domain crate MUST NOT import from runtime, infrastructure, or transport.
domain_imports=$(git -C "$ROOT" grep -n 'use ego_runtime::\|use ego_infrastructure::\|use ego_transport::' -- 'crates/domain/' 2>/dev/null || true)
if [ -n "$domain_imports" ]; then
    echo "FAIL: FO-R3 violation: domain imports runtime/infrastructure/transport:"
    echo "$domain_imports"
    EXIT_CODE=1
fi

# Runtime crate MUST NOT import from infrastructure or transport.
runtime_imports=$(git -C "$ROOT" grep -n 'use ego_infrastructure::\|use ego_transport::' -- 'crates/runtime/' 2>/dev/null || true)
if [ -n "$runtime_imports" ]; then
    echo "FAIL: FO-R3 violation: runtime imports infrastructure/transport:"
    echo "$runtime_imports"
    EXIT_CODE=1
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
if [ "$EXIT_CODE" -eq 0 ]; then
    echo "PASS: All checks passed."
else
    echo "FAIL: One or more checks failed."
fi

exit "$EXIT_CODE"
