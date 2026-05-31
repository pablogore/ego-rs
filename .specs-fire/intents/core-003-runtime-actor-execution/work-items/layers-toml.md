---
id: layers-toml
title: Update layers.toml
complexity: low
mode: autopilot
depends_on: [workspace-setup]
---

## Tasks

### core-003-6-1-update-layers-toml

Add to `layers.toml`:
```toml
"ego-runtime"      = "foundation"
"ego-runtime-tokio" = "infrastructure"
```

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/proposal.md` (lines 104-114)
- `layers.toml` (current state)

## Files changed

- `layers.toml` — add 2 lines

## Completion

- `grep -q 'ego-runtime' layers.toml`
- `grep -q 'ego-runtime-tokio' layers.toml`
- No existing entries modified

## Dependencies

Requires workspace-setup (crates exist). Independent of most other items — can run early.
