#!/usr/bin/env bash
set -euo pipefail

WORKSPACE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LAYERS_FILE="$WORKSPACE_ROOT/layers.toml"

ALLOWED_DIR="t"

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

pass() { printf "${GREEN}PASS${NC} %s\n" "$*"; }
fail() { printf "${RED}FAIL${NC} %s\n" "$*"; ALLOWED_DIR="f"; }

if [[ ! -f "$LAYERS_FILE" ]]; then
  echo "ERROR: layers.toml not found at $LAYERS_FILE"
  exit 1
fi

if [[ ! -f "$WORKSPACE_ROOT/Cargo.toml" ]]; then
  echo "SKIP: No Cargo.toml in workspace root — nothing to verify"
  exit 0
fi

trim_toml_value() {
  echo "$1" | xargs | sed 's/^"//;s/"$//'
}

layer_for() {
  local needle="$1"
  while IFS='|' read -r crate layer; do
    if [[ "$crate" == "$needle" ]]; then
      printf "%s" "$layer"
      return
    fi
  done <<< "$layer_entries"
}

path_for() {
  local needle="$1"
  while IFS='|' read -r crate path; do
    if [[ "$crate" == "$needle" ]]; then
      printf "%s" "$path"
      return
    fi
  done <<< "$workspace_entries"
}

allowed_dependency() {
  case "$1:$2" in
    transport:application|transport:domain|infrastructure:application|infrastructure:domain|application:domain)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

# Parse layers.toml: extract "crate-name" = "layer" pairs.
layer_entries=""
while IFS='=' read -r key val; do
  key=$(trim_toml_value "$key")
  val=$(trim_toml_value "$val")
  if [[ -n "$key" && "$key" != \[* ]]; then
    case "$val" in
      domain|application|infrastructure|transport)
        ;;
      *)
        fail "$key maps to unknown layer '$val'"
        ;;
    esac
    layer_entries="${layer_entries}${key}|${val}"$'\n'
  fi
done < <(grep '=' "$LAYERS_FILE" | grep -v '^\s*#')

if [[ -z "$(printf "%s" "$layer_entries" | sed '/^[[:space:]]*$/d')" ]]; then
  echo "No crates defined in layers.toml — nothing to verify"
  exit 0
fi

workspace_members=$(awk '
  /^[[:space:]]*members[[:space:]]*=/ { in_members=1; next }
  in_members && /^[[:space:]]*\]/ { in_members=0; next }
  in_members && $0 ~ /"/ {
    line=$0
    gsub(/[",]/, "", line)
    gsub(/^[[:space:]]+|[[:space:]]+$/, "", line)
    if (line != "") print line
  }
' "$WORKSPACE_ROOT/Cargo.toml")

workspace_entries=""
for member in $workspace_members; do
  cargo_toml="$WORKSPACE_ROOT/$member/Cargo.toml"
  if [[ ! -f "$cargo_toml" ]]; then
    fail "$member is listed in workspace members but Cargo.toml was not found"
    continue
  fi

  package_name=$(awk -F= '/^[[:space:]]*name[[:space:]]*=/ {
    value=$2
    gsub(/[ "]/, "", value)
    print value
    exit
  }' "$cargo_toml")

  if [[ -z "$package_name" ]]; then
    fail "$member/Cargo.toml does not declare a package name"
    continue
  fi

  workspace_entries="${workspace_entries}${package_name}|${member}"$'\n'
done

echo "Verifying hexagonal layer dependencies..."
echo ""

while IFS='|' read -r crate path; do
  [[ -z "$crate" ]] && continue
  if [[ -z "$(layer_for "$crate")" ]]; then
    fail "$crate is listed in workspace members but missing from layers.toml"
  fi
done <<< "$workspace_entries"

while IFS='|' read -r crate layer; do
  [[ -z "$crate" ]] && continue

  member_path=$(path_for "$crate")
  if [[ -z "$member_path" ]]; then
    fail "$crate is listed in layers.toml but missing from workspace members"
    continue
  fi

  cargo_toml="$WORKSPACE_ROOT/$member_path/Cargo.toml"

  # Extract dependency names from [dependencies] section
  deps=$(awk '
    /^\[dependencies\]/ { in_deps=1; next }
    /^\[/ { in_deps=0 }
    in_deps && $0 ~ /^[[:space:]]*[^#[:space:]][^=]*=/ {
      line=$0
      sub(/=.*/, "", line)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", line)
      gsub(/"/, "", line)
      print line
    }
  ' "$cargo_toml")

  if [[ -z "$deps" ]]; then
    pass "$crate ($layer) — no internal dependencies"
    continue
  fi

  for dep in $deps; do
    dep_layer=$(layer_for "$dep")
    if [[ -z "$dep_layer" ]]; then
      continue
    fi
    if allowed_dependency "$layer" "$dep_layer"; then
      pass "$crate ($layer) → $dep ($dep_layer)"
    else
      fail "$crate ($layer) → $dep ($dep_layer) — VIOLATION: $layer may not depend on $dep_layer"
    fi
  done
done <<< "$layer_entries"

echo ""
if [[ "$ALLOWED_DIR" == "f" ]]; then
  echo "Layer dependency violations detected. Allowed directions:"
  echo "  transport      → application, domain"
  echo "  infrastructure → application, domain"
  echo "  application    → domain"
  echo "  domain         → (no internal dependencies)"
  exit 1
fi

echo "All layer dependencies follow hexagonal architecture rules."
