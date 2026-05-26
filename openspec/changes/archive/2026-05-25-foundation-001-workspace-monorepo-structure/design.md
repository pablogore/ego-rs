## Context

The project already defines architectural layers in `architecture-governance`: domain, application, infrastructure, and transport. This change converts those rules into a concrete Rust workspace convention so contributors know where crates belong and how ownership is declared.

## Goals / Non-Goals

**Goals:**
- Define canonical workspace crate groups under `crates/`.
- Define crate naming rules.
- Make `layers.toml` the source of truth for crate layer ownership.
- Define public module boundary conventions.
- Keep dependency rules aligned with `architecture-governance`.

**Non-Goals:**
- Add new architecture layers.
- Change `architecture-governance`.
- Refactor business behavior.
- Introduce a new dependency validation engine.

## Decisions

### Canonical Workspace Groups

Workspace crates live under `crates/`. Canonical groups are `domain`, `application`, `infrastructure`, `transport`, `contracts`, and `observability`. `contracts` and `observability` are package groups, not architecture layers.

### Layer Ownership

Every workspace crate is mapped in `layers.toml` to exactly one existing architecture layer: domain, application, infrastructure, or transport.

### Dependency Rule Alignment

Workspace dependency checks mirror `architecture-governance` by referencing the existing architecture layers and allowed dependency directions only. This change does not introduce new dependency semantics, new layers, or a replacement enforcement model.

### Crate Naming

Crate package names use `ego-<group>` for top-level crates and `ego-<group>-<module>` for split module crates.

### Module Boundaries

Each crate exposes intentional public APIs from `src/lib.rs`. Implementation modules stay private unless they are part of the crate boundary.

Future crates use `src/lib.rs` as the crate facade. Public modules or re-exports in `src/lib.rs` are the intentional cross-crate API; implementation modules are declared with private `mod` items and stay crate-internal unless the change explicitly promotes them to the public boundary. Internal helpers should prefer private visibility, using `pub(crate)` only for same-crate collaboration across modules.

## Risks / Trade-offs

- [Risk] Package groups may be mistaken for layers -> Mitigation: require `layers.toml` mapping to existing layers only.
- [Risk] Naming changes may touch imports -> Mitigation: audit and rename only when necessary.
- [Risk] More crates can add overhead -> Mitigation: split by module only when there is a real boundary.
