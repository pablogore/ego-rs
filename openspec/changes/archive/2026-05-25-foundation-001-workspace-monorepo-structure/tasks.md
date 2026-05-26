## 1. Workspace Layout

- [x] 1.1 Create or confirm canonical package group directories under `crates/`
- [x] 1.2 Ensure root `Cargo.toml` workspace members match first-party crates under `crates/`
- [x] 1.3 Add `contracts` and `observability` package groups when required by implementation — not currently required; directories can be created under `crates/` when implementation needs them

## 2. Layer Ownership

- [x] 2.1 Populate `layers.toml` with exactly one layer mapping per workspace crate
- [x] 2.2 Verify support package groups map only to existing architecture layers — no crates exist under `crates/contracts/` or `crates/observability/`; constraint enforced by spec requiring every crate to map to one of the four architecture layers in `layers.toml`
- [x] 2.3 Run or update layer validation for missing and stale mappings

## 3. Naming And Boundaries

- [x] 3.1 Verify crate package names follow `ego-<group>` or `ego-<group>-<module>`
- [x] 3.2 Confirm each crate exposes intentional public APIs through `src/lib.rs`
- [x] 3.3 Document internal module boundary conventions for future crates

## 4. Governance Alignment

- [x] 4.1 Confirm dependency rules mirror `architecture-governance` without redefining it
- [x] 4.2 Validate the final workspace structure with the existing layer verification script
