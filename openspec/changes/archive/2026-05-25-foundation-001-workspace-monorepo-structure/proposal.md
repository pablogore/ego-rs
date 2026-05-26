## Why

The project has architecture governance rules, but the workspace layout still needs a concrete monorepo contract for crate boundaries, naming, ownership, dependency rules, and module boundaries. This change operationalizes `architecture-governance` without redefining it.

## What Changes

- Define the canonical Rust workspace layout under `crates/`.
- Define crate naming and internal package conventions.
- Define layer ownership through `layers.toml`.
- Define module boundary rules for public crate APIs.
- Require dependency rules to mirror `architecture-governance`.

## Capabilities

### New Capabilities

- `workspace-monorepo-structure`: Workspace layout, crate naming, layer ownership, dependency rules, internal package conventions, and module boundaries.

### Modified Capabilities

<!-- None - this change operationalizes `architecture-governance` without changing its requirements. -->

## Impact

- Root `Cargo.toml` workspace members.
- `crates/` package layout.
- `layers.toml` ownership mapping.
- Future crate creation and review workflow.
