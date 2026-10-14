#!/usr/bin/env bash
# Reports hotfix commits on the head ref (default: origin/main) that have no
# patch-equivalent commit on the upstream ref (default: origin/develop),
# using `git cherry -v` (patch-id based, excludes merge commits by design).
# A commit younger than BACKPORT_WINDOW_HOURS (default 72) is a grace-period
# candidate, not drift yet (branch-promotion-integrity spec, "...After A
# Grace Period"; design.md AD-9) — see release-guards.test.sh cases 3/4.
#
# Never uses `git -C` or `cd`: the caller's cwd is the sole repository
# authority (release-guards.test.sh case 5, repo-selection isolation).
# Stateless: no cache, no issue/API calls here — that's the caller's job.
# Always exits 0 — this is a report generator, not a gate.
#
# Usage: bash backport-drift.sh [<upstream-ref> [<head-ref>]]
#   Defaults: origin/develop origin/main
set -u

upstream="${1:-origin/develop}"
head="${2:-origin/main}"
window_hours="${BACKPORT_WINDOW_HOURS:-72}"
window_seconds=$((window_hours * 3600))
now="$(date +%s)"

report=""

while IFS= read -r line; do
  [ -z "$line" ] && continue
  case "$line" in
    +\ *)
      sha="${line#+ }"
      sha="${sha%% *}"
      committer_ts="$(git show -s --format=%ct "$sha" 2>/dev/null)" || continue
      age=$((now - committer_ts))
      [ "$age" -lt "$window_seconds" ] && continue
      subject="$(git show -s --format=%s "$sha" 2>/dev/null)"
      report="${report}- \`${sha:0:7}\` ${subject}
"
      ;;
  esac
done < <(git cherry -v "$upstream" "$head" 2>/dev/null)

if [ -n "$report" ]; then
  printf '## Backport drift: commits on `%s` missing from `%s`\n\n%s' "$head" "$upstream" "$report"
fi

exit 0
