# Proposal: CORE-014 Authorization Providers

## Intent

The `security-sdk` already ships a working `AuthorizationProvider` SPI, `RbacProvider`, `RoleStore`/`InMemoryRoleStore`, `AccessRequest::from_permission`, `AuthorizationDecision`, and the `authorize_in_context` seam (delivered under CORE-011/012, fully tested per FR-008/009/010, TS-006/007/011/013). The gap CORE-014 closes is the missing **Level-1 built-in reference implementations** and the **declarative authorization ergonomics**: today the only always-allow / always-deny implementations are private test stubs (`AlwaysAllow`/`AlwaysDeny`), so dev/test/demo runtimes and lockdown/secure-by-default deployments must hand-roll a provider, and operations must call `authorize_in_context` manually with no `#[authorize("resource:action")]` attribute.

## Provider Architecture

CORE-014 establishes Level 1 of a three-level authorization provider hierarchy:

```
Level 1 — Built-in (CORE-014, in security-sdk)
  - AllowAllAuthorizationProvider
  - DenyAllAuthorizationProvider
  - RbacProvider

Level 2 — Advanced (future, separate crates)
  - AbacAuthorizationProvider
  - RebacAuthorizationProvider

Level 3 — External Integrations (future, adapter crates)
  - OpenFGA
  - SpiceDB
  - Zanzibar-compatible implementations
```

Level 2 and Level 3 are out of scope for CORE-014. They are documented here to establish the hierarchy and ensure the SPI design accommodates future growth without modification.

## SPI Ownership

`AuthorizationProvider` is the single official authorization extension point in `security-sdk`.

**Constraints:**
- Future authorization implementations MUST implement `AuthorizationProvider`.
- Alternative extension traits (`PolicyProvider`, `PermissionProvider`, `OpenFgaProvider`, etc.) are NOT supported and will not be introduced.
- The SPI contract (`AuthorizationProvider` trait signature) MUST remain stable. New providers add implementations; they do not modify the trait.
- This constraint applies to both built-in (Level 1) and all future (Level 2/3) providers.

## Scope

### In Scope
- `AllowAllAuthorizationProvider` in `security-sdk` (public, `Arc<dyn AuthorizationProvider>`-injectable) — dev/test/demo.
- `DenyAllAuthorizationProvider` in `security-sdk` — lockdown / secure-by-default; returns `Deny { reason }`.
- Public exports for both in `providers/mod.rs` + crate root; `#![deny(missing_docs)]` compliant.
- Tests-first coverage for both providers (allow path, deny path, object-safety, `Send + Sync`).
- `#[authorize("resource:action")]` proc-macro is owned exclusively by CORE-015. CORE-014 owns ONLY the built-in provider implementations.
- `cargo test --workspace` green.

### Out of Scope
- `RbacProvider`, `RoleStore`, `InMemoryRoleStore`, `AccessRequest::from_permission`, `AuthorizationDecision` — ALREADY IMPLEMENTED, not re-delivered here.
- ABAC, ReBAC, OpenFGA, SpiceDB (future Level-2/3 crates); Zanzibar model.
- JWT-based authz (owned by `security-jwt`).
- Multi-provider composition / policy chaining.
- Resource wildcards (deferred to CORE-009A per existing RbacProvider docs).

## Capabilities

### New Capabilities
- None — no new spec file.

### Modified Capabilities
- `security-sdk`: add built-in `AllowAllAuthorizationProvider` and `DenyAllAuthorizationProvider` reference implementations alongside FR-008/009.

## Approach

Implement the two built-in reference implementations as trivial deterministic structs in `crates/security-sdk/src/providers/` (e.g. `allow_all`, `deny_all` modules), promoting the existing private test stubs to documented public types. `AllowAll` returns `Ok(Allow)`; `DenyAll` returns `Ok(Deny { reason })` with a fixed reason. Both ignore principal/request/ctx. Strict TDD: write provider tests before implementation.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/security-sdk/src/providers/mod.rs` | Modified | Export new providers |
| `crates/security-sdk/src/providers/{allow_all,deny_all}/` | New | Two built-in providers |
| `crates/security-sdk/src/lib.rs` | Modified | Re-export new providers |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Naming drift (`AllowAll` vs spec `RbacProvider` style) | Low | Follow existing `*Provider` suffix convention |

## Rollback Plan

Revert the providers commit independently; it is additive (new modules only). No existing public type changes, so removing them restores prior `cargo test --workspace` green state.

## Dependencies

- `async-trait` (already present). No new external deps.

## Future Evolution

The authorization provider ecosystem is planned as a sequence of changes:

```
CORE-014  →  CORE-015  →  CORE-016  →  CORE-017  →  CORE-018  →  CORE-019
Built-in      #[authorize]   ABAC         ReBAC        OpenFGA      SpiceDB
Providers     Macro
```

Each change introduces new implementations of `AuthorizationProvider` without modifying the SPI. This roadmap is informational; scope boundaries are defined per-change.

### Composite Provider (Architecture Reservation)

Future implementations MAY introduce a `CompositeAuthorizationProvider` that itself implements `AuthorizationProvider`, enabling policy chaining without modifying the SPI. CORE-014 does NOT introduce composition. This slot is reserved for a future change.

## Success Criteria

- [x] `AllowAllAuthorizationProvider` and `DenyAllAuthorizationProvider` implement `AuthorizationProvider`, are public, `Send + Sync`, object-safe, `Arc<dyn>`-injectable.
- [x] AllowAll → `Ok(Allow)`; DenyAll → `Ok(Deny { reason })`, covered by tests written before impl.
- [x] No new external dependencies.
- [x] `#![deny(missing_docs)]` satisfied for all new public items.
- [x] `#[authorize]` macro is explicitly deferred to CORE-015. CORE-014 owns built-in providers only.
- [x] `cargo test --workspace` passes.
