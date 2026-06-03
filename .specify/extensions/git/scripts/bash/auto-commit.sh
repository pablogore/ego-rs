#!/usr/bin/env bash
set -euo pipefail

# auto-commit.sh — Stage and commit all changes after a Spec Kit command.
#
# Usage: auto-commit.sh <event_name>
# Example: auto-commit.sh after_specify

EVENT="${1:-}"
if [ -z "$EVENT" ]; then
  echo "[auto-commit] ERROR: no event name provided" >&2
  exit 1
fi

# Locate config relative to this script's location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXTENSION_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
CONFIG_FILE="$EXTENSION_DIR/git/git-config.yml"

if [ ! -f "$CONFIG_FILE" ]; then
  echo "[auto-commit] SKIP: config file not found at $CONFIG_FILE"
  exit 0
fi

# Parse config using grep (no yq dependency)
# Look for exact event key under auto_commit, fall back to default
ENABLED=false
COMMIT_MSG=""

if grep -q "^${EVENT}:" "$CONFIG_FILE" 2>/dev/null; then
  # Read enabled flag for this event
  EVENT_ENABLED=$(grep -A 3 "^${EVENT}:" "$CONFIG_FILE" | grep "enabled:" | head -1 | sed 's/.*enabled:[[:space:]]*//' | tr -d '[:space:]')
  EVENT_MSG=$(grep -A 3 "^${EVENT}:" "$CONFIG_FILE" | grep "message:" | head -1 | sed 's/.*message:[[:space:]]*//' | sed 's/^"//;s/"$//')
  if [ "$EVENT_ENABLED" = "true" ]; then
    ENABLED=true
    COMMIT_MSG="$EVENT_MSG"
  fi
elif grep -q "^default:" "$CONFIG_FILE" 2>/dev/null; then
  DEFAULT_ENABLED=$(grep -A 1 "^default:" "$CONFIG_FILE" | grep "enabled:" | head -1 | sed 's/.*enabled:[[:space:]]*//' | tr -d '[:space:]')
  DEFAULT_MSG=$(grep -A 1 "^default:" "$CONFIG_FILE" | grep "message:" | head -1 | sed 's/.*message:[[:space:]]*//' | sed 's/^"//;s/"$//')
  if [ "$DEFAULT_ENABLED" = "true" ]; then
    ENABLED=true
    COMMIT_MSG="$DEFAULT_MSG"
  fi
fi

if [ "$ENABLED" != "true" ]; then
  echo "[auto-commit] SKIP: auto-commit not enabled for event '$EVENT'"
  exit 0
fi

# Check for uncommitted changes
if ! git diff --quiet --exit-code 2>/dev/null || ! git diff --cached --quiet --exit-code 2>/dev/null; then
  HAS_CHANGES=true
else
  # Also check for untracked files
  UNTRACKED=$(git ls-files --others --exclude-standard 2>/dev/null | head -1)
  if [ -n "$UNTRACKED" ]; then
    HAS_CHANGES=true
  else
    HAS_CHANGES=false
  fi
fi

if [ "$HAS_CHANGES" != "true" ]; then
  echo "[auto-commit] SKIP: no changes to commit"
  exit 0
fi

# Use default message if none configured
if [ -z "$COMMIT_MSG" ]; then
  COMMIT_MSG="[Spec Kit] Auto-commit ($EVENT)"
fi

echo "[auto-commit] Staging all changes..."
git add .

echo "[auto-commit] Committing..."
git commit -m "$COMMIT_MSG"

echo "[auto-commit] DONE: $COMMIT_MSG"
