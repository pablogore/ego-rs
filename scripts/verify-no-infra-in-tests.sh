#!/usr/bin/env bash
set -euo pipefail

WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [[ ! -f "$WORKSPACE_ROOT/Cargo.toml" ]]; then
  echo "SKIP: No Cargo.toml in workspace root — nothing to verify"
  exit 0
fi

# Infrastructure crates/types that must not appear in test files
FORBIDDEN_PATTERNS=(
  'sqlx'
  'diesel'
  'postgres::'
  'rusqlite'
  'mongodb::'
  'redis::'
  'rdkafka'
  'kafka::'
  'reqwest::'
  'hyper::client'
  'hyper::Client'
  'tonic::transport'
  'aws_sdk'
  'aws_config'
  'tokio::net::TcpListener'
  'tokio::net::TcpStream'
  'std::net::TcpStream'
  'std::net::TcpListener'
  'actix_web::test'
  'actix_rt::test'
  'rocket::local'
  'warp::test'
  'axum_test'
  'std::process::Command'
  'std::fs::File'
  'std::fs::write'
  'std::fs::read'
  'tokio::fs::File'
  'tempfile'
  'tempdir'
)

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

VIOLATIONS=0

# Find all Rust test files: either #[cfg(test)] modules or files in tests/ dirs
find "$WORKSPACE_ROOT" -name '*.rs' -not -path '*/target/*' | while read -r file; do
  in_test_module=0

  while IFS= read -r line; do
    if echo "$line" | grep -q '#\[cfg(test)\]'; then
      in_test_module=1
    fi
    if [[ "$in_test_module" -eq 1 ]] && echo "$line" | grep -q '^}'; then
      in_test_module=0
    fi

    if [[ "$in_test_module" -eq 0 ]]; then
      continue
    fi

    for pattern in "${FORBIDDEN_PATTERNS[@]}"; do
      if echo "$line" | grep -q "$pattern"; then
        printf "${RED}VIOLATION${NC} %s:%s — imports '%s' in test context\n" \
          "$file" "$(echo "$line" | sed 's/^[[:space:]]*//')" "$pattern"
        VIOLATIONS=$((VIOLATIONS + 1))
      fi
    done
  done < "$file"
done

if [[ "$VIOLATIONS" -gt 0 ]]; then
  echo ""
  echo "Found $VIOLATIONS infrastructure import violations in test code."
  echo "Tests must use mocks, not real infrastructure types."
  exit 1
fi

echo "${GREEN}PASS${NC} No infrastructure types imported in test code."
