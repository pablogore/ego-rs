#!/usr/bin/env bash
set -euo pipefail

WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [[ ! -f "$WORKSPACE_ROOT/Cargo.toml" ]]; then
  echo "SKIP: No Cargo.toml in workspace root — nothing to verify"
  exit 0
fi

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

VIOLATIONS=0

is_port_trait() {
  local file="$1"
  local trait_name="$2"

  case "$trait_name" in
    CommandBus|QueryBus|EventStore|*Port)
      return 0
      ;;
  esac

  case "$file" in
    */ports.rs|*/ports/*.rs|*/port.rs|*/port/*.rs)
      return 0
      ;;
  esac

  return 1
}

while read -r file; do
  attrs=""
  manual_mock=0

  while IFS= read -r line; do
    if [[ "$line" =~ ^[[:space:]]*#\[ ]]; then
      attrs="${attrs}"$'\n'"${line}"
      continue
    fi

    if echo "$line" | grep -Eiq 'manual mock|mock implementation|hand-written mock'; then
      manual_mock=1
      continue
    fi

    if echo "$line" | grep -Eq '^[[:space:]]*(pub([[:space:]]*\([^)]*\))?[[:space:]]+)?trait[[:space:]]+[A-Za-z_][A-Za-z0-9_]*'; then
      trait_name="$(echo "$line" | sed -E 's/^[[:space:]]*(pub([[:space:]]*\([^)]*\))?[[:space:]]+)?trait[[:space:]]+([A-Za-z_][A-Za-z0-9_]*).*/\3/')"

      if is_port_trait "$file" "$trait_name"; then
        if ! echo "$attrs" | grep -q '#\[automock\]' && [[ "$manual_mock" -ne 1 ]]; then
          printf "${RED}VIOLATION${NC} %s — trait '%s' is a port trait without #[automock] or documented manual mock\n" \
            "$file" "$trait_name"
          VIOLATIONS=$((VIOLATIONS + 1))
        fi
      fi
    fi

    if [[ ! "$line" =~ ^[[:space:]]*$ ]]; then
      attrs=""
      manual_mock=0
    fi
  done < "$file"
done < <(find "$WORKSPACE_ROOT" -name '*.rs' -not -path '*/target/*')

if [[ "$VIOLATIONS" -gt 0 ]]; then
  echo ""
  echo "Found $VIOLATIONS port trait(s) without mock support."
  echo "Every architectural boundary trait must use #[automock] or document an equivalent manual mock."
  exit 1
fi

printf "${GREEN}PASS${NC} All port traits declare mock support.\n"
