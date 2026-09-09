#!/usr/bin/env bash
# Test suite for .github/scripts/backport-drift.sh and the loop/injection
# invariants both release-related workflows must hold. Pure git + bash: no
# network call and no `gh` invocation anywhere in this file (design.md,
# "Testing Strategy"). RED-first per Strict TDD: this file is written and
# run against a repo that does not yet have backport-drift.sh, confirmed to
# fail only because the script is missing, before the script is written.
#
# Usage: bash .github/scripts/release-guards.test.sh
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DRIFT_SCRIPT="$SCRIPT_DIR/backport-drift.sh"

PASS=0
FAIL=0

pass() {
  PASS=$((PASS + 1))
  echo "PASS: $1"
}

fail() {
  FAIL=$((FAIL + 1))
  echo "FAIL: $1"
}

assert_empty() {
  local desc="$1" actual="$2"
  if [ -z "$actual" ]; then
    pass "$desc"
  else
    fail "$desc (expected empty stdout, got: ${actual})"
  fi
}

assert_not_empty_containing() {
  local desc="$1" actual="$2" needle="$3"
  if [ -n "$actual" ] && printf '%s' "$actual" | grep -q -- "$needle"; then
    pass "$desc"
  else
    fail "$desc (expected non-empty stdout containing '${needle}', got: ${actual})"
  fi
}

# Asserts that a haystack contains a needle, naming what was missing when it
# does not — the static invariant cases below read better as "the file must
# say X" than as "grep -c must equal 1".
assert_contains() {
  local desc="$1" haystack="$2" needle="$3"
  if [[ "$haystack" == *"$needle"* ]]; then
    pass "$desc"
  else
    fail "$desc (expected to find: ${needle})"
  fi
}

assert_not_contains() {
  local desc="$1" haystack="$2" needle="$3"
  if printf '%s' "$haystack" | grep -q -- "$needle"; then
    fail "$desc (unexpectedly found '${needle}')"
  else
    pass "$desc"
  fi
}

assert_exit_code() {
  local desc="$1" expected="$2" actual="$3"
  if [ "$expected" = "$actual" ]; then
    pass "$desc"
  else
    fail "$desc (expected exit ${expected}, got ${actual})"
  fi
}

# Prints a git-parseable "@<epoch> +0000" timestamp N hours in the past.
# `date +%s` is portable across the BSD `date` (macOS) and GNU `date`
# (Linux CI); GIT_AUTHOR_DATE / GIT_COMMITTER_DATE reject relative strings
# like "100 hours ago" even though `git commit --date` accepts them, so the
# epoch form is the portable common ground.
hours_ago() {
  local hours="$1"
  echo "@$(( $(date +%s) - hours * 3600 )) +0000"
}

# Builds a throwaway repo under mktemp -d with `main` and `develop` branches
# sharing one initial commit. Echoes the repo path.
new_base_repo() {
  local dir
  dir="$(mktemp -d)"
  git -C "$dir" init --quiet --initial-branch=main
  git -C "$dir" config user.email "test@example.com"
  git -C "$dir" config user.name "Test"
  git -C "$dir" commit --quiet --allow-empty -m "chore: initial commit"
  git -C "$dir" branch develop main
  echo "$dir"
}

# --- Case 1 (task 2.1 / 2.2): a hotfix merged normally into develop is not drift ---
test_merged_normally_is_not_drift() {
  local repo
  repo="$(new_base_repo)"
  (
    cd "$repo"
    git checkout --quiet main
    git commit --quiet --allow-empty -m "fix: patch a bug on main"
    git checkout --quiet develop
    git merge --quiet --no-ff -m "merge hotfix into develop" main
  )
  local out exit_code
  out="$(cd "$repo" && bash "$DRIFT_SCRIPT" develop main)"
  exit_code=$?
  assert_empty "merged-normally hotfix produces empty stdout" "$out"
  assert_exit_code "merged-normally hotfix exits 0" "0" "$exit_code"
  rm -rf "$repo"
}

# --- Case 2 (task 2.3 / 2.4): a cherry-picked backport is not drift ---
test_cherry_picked_is_not_drift() {
  local repo hotfix_sha
  repo="$(new_base_repo)"
  (
    cd "$repo"
    git checkout --quiet main
    git commit --quiet --allow-empty -m "fix: another bug on main"
  )
  hotfix_sha="$(git -C "$repo" rev-parse main)"
  (
    cd "$repo"
    git checkout --quiet develop
    git cherry-pick --quiet --allow-empty "$hotfix_sha"
  )
  local out exit_code
  out="$(cd "$repo" && bash "$DRIFT_SCRIPT" develop main)"
  exit_code=$?
  assert_empty "cherry-picked hotfix produces empty stdout" "$out"
  assert_exit_code "cherry-picked hotfix exits 0" "0" "$exit_code"
  rm -rf "$repo"
}

# --- Case 3 (task 2.5 / 2.6): an old un-backported hotfix is reported ---
test_old_unbackported_is_reported() {
  local repo old_ts
  repo="$(new_base_repo)"
  old_ts="$(hours_ago 100)"
  (
    cd "$repo"
    git checkout --quiet main
    GIT_AUTHOR_DATE="$old_ts" GIT_COMMITTER_DATE="$old_ts" \
      git commit --quiet --allow-empty -m "fix: old unbackported hotfix"
  )
  local out exit_code
  out="$(cd "$repo" && bash "$DRIFT_SCRIPT" develop main)"
  exit_code=$?
  assert_not_empty_containing "old unbackported hotfix is reported" "$out" "old unbackported hotfix"
  assert_exit_code "old unbackported hotfix still exits 0" "0" "$exit_code"
  rm -rf "$repo"
}

# --- Case 4 (task 2.7 / 2.8): a fresh un-backported hotfix, within the
# BACKPORT_WINDOW_HOURS grace period, is NOT yet reported. Per Phase 0's
# resolved decision (branch-promotion-integrity/spec.md, "...After A Grace
# Period"), AD-9's window stands. ---
test_fresh_unbackported_within_window_is_not_reported() {
  local repo
  repo="$(new_base_repo)"
  (
    cd "$repo"
    git checkout --quiet main
    git commit --quiet --allow-empty -m "fix: fresh unbackported hotfix"
  )
  local out exit_code
  out="$(cd "$repo" && bash "$DRIFT_SCRIPT" develop main)"
  exit_code=$?
  assert_empty "fresh in-window unbackported hotfix produces empty stdout" "$out"
  assert_exit_code "fresh in-window unbackported hotfix exits 0" "0" "$exit_code"
  rm -rf "$repo"
}

# --- Case 5 (task 2.9 / 2.10): repo-selection isolation. The script must
# never use `git -C` or cd, so cwd is the sole repository authority. ---
test_repo_isolation() {
  local repo_a repo_b old_ts
  old_ts="$(hours_ago 100)"
  repo_a="$(new_base_repo)"
  (
    cd "$repo_a"
    git checkout --quiet main
    GIT_AUTHOR_DATE="$old_ts" GIT_COMMITTER_DATE="$old_ts" \
      git commit --quiet --allow-empty -m "fix: drift only in repo A"
  )
  repo_b="$(new_base_repo)"
  (
    cd "$repo_b"
    git checkout --quiet main
    GIT_AUTHOR_DATE="$old_ts" GIT_COMMITTER_DATE="$old_ts" \
      git commit --quiet --allow-empty -m "fix: drift only in repo B"
  )
  local out
  out="$(cd "$repo_b" && bash "$DRIFT_SCRIPT" develop main)"
  assert_not_empty_containing "repo isolation reports repo B's own drift" "$out" "drift only in repo B"
  assert_not_contains "repo isolation does not leak repo A's drift" "$out" "drift only in repo A"
  rm -rf "$repo_a" "$repo_b"
}

# --- Case 6 (task 2.11 / 3.5): static invariants over the workflow files.
# No `git push` line targets a branch ref, and no `run:` block interpolates
# `${{ github.event... }}` (design.md Threat Matrix). PR2 scopes this to
# release.yml only, since backport-drift.yml does not exist yet; PR3
# extends the same assertions to backport-drift.yml. ---
test_no_push_targets_a_branch_ref() {
  local file="$REPO_ROOT/.github/workflows/release.yml"
  if [ -f "$file" ]; then
    local bad
    bad="$(grep -n 'git push' "$file" | grep -E 'origin[[:space:]]+(main|develop|HEAD)([[:space:]]|$)' || true)"
    assert_empty "release.yml: no git push targets a branch ref" "$bad"
  else
    fail "release.yml not found for static invariant check"
  fi

  local drift_file="$REPO_ROOT/.github/workflows/backport-drift.yml"
  if [ -f "$drift_file" ]; then
    local bad_drift
    bad_drift="$(grep -n 'git push' "$drift_file" | grep -E 'origin[[:space:]]+(main|develop|HEAD)([[:space:]]|$)' || true)"
    assert_empty "backport-drift.yml: no git push targets a branch ref" "$bad_drift"
  fi
}

test_no_run_block_interpolates_github_event() {
  local file="$REPO_ROOT/.github/workflows/release.yml"
  if [ -f "$file" ]; then
    local matches
    matches="$(grep -n '\${{[[:space:]]*github\.event' "$file" || true)"
    assert_empty "release.yml: no run: block interpolates \${{ github.event... }}" "$matches"
  else
    fail "release.yml not found for static invariant check"
  fi

  local drift_file="$REPO_ROOT/.github/workflows/backport-drift.yml"
  if [ -f "$drift_file" ]; then
    local drift_matches
    drift_matches="$(grep -n '\${{[[:space:]]*github\.event' "$drift_file" || true)"
    assert_empty "backport-drift.yml: no run: block interpolates \${{ github.event... }}" "$drift_matches"
  fi
}

# --- Case 7: the release guard must distinguish "already tagged at this
# commit" from "this version is tagged somewhere else". The first is a benign
# re-run, the second is a release-history inconsistency that must fail the
# job rather than skip silently.
#
# `git rev-parse <tag>` on an annotated tag returns the tag object's SHA, not
# the commit's, so a guard built on it alone can never make that distinction.
# The guard must resolve the tag to a commit and compare it against HEAD. ---
test_release_guard_compares_the_tagged_commit() {
  local file="$REPO_ROOT/.github/workflows/release.yml"
  if [ ! -f "$file" ]; then
    fail "release.yml not found for static invariant check"
    return
  fi

  assert_contains "release.yml: the tag guard resolves the tag to a commit" \
    "$(cat "$file")" "git rev-list -n1"
  assert_contains "release.yml: the tag guard compares against HEAD" \
    "$(cat "$file")" "git rev-parse HEAD"
  assert_contains "release.yml: a version tagged at another commit fails the job" \
    "$(cat "$file")" "::error::Tag"
}

echo "== release-guards.test.sh =="

if [ ! -f "$DRIFT_SCRIPT" ]; then
  echo "backport-drift.sh does not exist yet at $DRIFT_SCRIPT — the classification/isolation cases below are expected to fail (RED)."
fi

test_merged_normally_is_not_drift
test_cherry_picked_is_not_drift
test_old_unbackported_is_reported
test_fresh_unbackported_within_window_is_not_reported
test_repo_isolation
test_no_push_targets_a_branch_ref
test_no_run_block_interpolates_github_event
test_release_guard_compares_the_tagged_commit

echo "== ${PASS} passed, ${FAIL} failed =="

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
