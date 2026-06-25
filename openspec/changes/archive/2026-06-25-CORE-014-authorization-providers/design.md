# Design: CORE-014 Authorization Providers

## Technical Approach

Promote the two private test stubs (`AlwaysAllow`/`AlwaysDeny` in `authorization/mod.rs`) into two public, documented, stateless built-in reference implementations under `crates/security-sdk/src/providers/`, mirroring the existing `basic/` and `rbac/` directory-module layout. Each is a unit struct implementing `AuthorizationProvider` via `#[async_trait]`, returning a constant decision and ignoring all inputs. Both are re-exported through `providers/mod.rs` and the crate root next to `RbacProvider`. Satisfies FR-017–FR-020 and the FR-013 modification. The `#[authorize]` macro is OUT of this delta (deferred to CORE-015). Strict TDD: tests precede impl.

## Architecture Decisions

| AD | Decision | Alternatives rejected | Rationale |
|----|----------|----------------------|-----------|
| AD-1 Module location | Directory modules: `providers/allow_all/mod.rs` + `providers/deny_all/mod.rs` | (A) single `providers/builtin.rs`; (B) inline in `authorization/mod.rs` | Matches the EXACT existing convention — every concrete provider (`basic/mod.rs`, `rbac/mod.rs`) is its own directory module. (B) would leave the trait file holding concrete impls, breaking the contract-vs-implementation separation already established. Proposal also names `allow_all`/`deny_all` modules. |
| AD-2 Naming | `AllowAllAuthorizationProvider`, `DenyAllAuthorizationProvider` | Keep stub names `AlwaysAllow`/`AlwaysDeny` | Spec FR-017/018/019 mandate these exact names. They follow the `*Provider` suffix used by `RbacProvider`/`BasicAuthenticationProvider` and the CORE-013 `*AuthenticationProvider` convention. The stubs are renamed/removed (not re-exported) per spec. |
| AD-3 Struct internals | Unit struct: `pub struct AllowAllAuthorizationProvider;` | Empty braces `{}` | Both are stateless. The spec scenarios construct them as `Arc::new(AllowAllAuthorizationProvider)` (no braces) — a unit struct satisfies that literally. Unit struct is the idiomatic Rust form for zero-field types and matches the existing stub style. |
| AD-4 async-trait | Use `#[async_trait]` on the impl | Native `async fn` in trait | The `AuthorizationProvider` trait IS declared `#[async_trait]` (authorization/mod.rs:20) and `#[cfg_attr(test, mockall::automock)]`. Impls MUST match. `RbacProvider` confirms the pattern. Native async-fn-in-trait is incompatible with the existing trait declaration and would not be object-safe for `Arc<dyn>`. |
| AD-5 Test location | Inline `#[cfg(test)] mod tests` in each provider's `mod.rs` | Separate `tests/` integration files | Both `basic/mod.rs:59` and `rbac/mod.rs:74` use inline `#[cfg(test)] mod tests`. FR-019 crate-root re-export test (TS-019) needs the public path; it goes in `allow_all` tests (or `deny_all`) as a compile-only `use ego_security_sdk::{...}` assertion. |

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

### Composite Provider (Architecture Reservation)

Future implementations MAY introduce a `CompositeAuthorizationProvider` that itself implements `AuthorizationProvider`, enabling policy chaining without modifying the SPI. CORE-014 does NOT introduce composition. This slot is reserved for a future change.

## Data Flow

    Arc<dyn AuthorizationProvider>
        │  (injected like RbacProvider)
        ▼
    AllowAllAuthorizationProvider::authorize ──► Ok(Allow)
    DenyAllAuthorizationProvider::authorize  ──► Ok(Deny { reason: "deny-all" })
        │
        └──► consumed by existing authorize_in_context seam (unchanged)

Inputs (principal, request, ctx) are ignored — decision is constant.

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/security-sdk/src/providers/allow_all/mod.rs` | Create | `AllowAllAuthorizationProvider` unit struct + `#[async_trait]` impl returning `Ok(Allow)`; doc comment marking it dev/test/demo ONLY; inline `#[cfg(test)] mod tests` (TS-014, TS-015, arc-injectable, TS-019). |
| `crates/security-sdk/src/providers/deny_all/mod.rs` | Create | `DenyAllAuthorizationProvider` unit struct + impl returning `Ok(Deny { reason: "deny-all".to_string() })`; doc comment marking it lockdown/secure-by-default; inline tests (TS-016, TS-017, TS-018, arc-injectable). |
| `crates/security-sdk/src/providers/mod.rs` | Modify | Add `pub mod allow_all; pub mod deny_all;` and `pub use allow_all::AllowAllAuthorizationProvider; pub use deny_all::DenyAllAuthorizationProvider;` in the same grouping as `rbac::RbacProvider`. |
| `crates/security-sdk/src/lib.rs` | Modify | Extend the `providers::{...}` re-export block to expose both new types at crate root (FR-019). |
| `crates/security-sdk/src/authorization/mod.rs` | Modify | Remove/replace the private `AlwaysAllow`/`AlwaysDeny` test stubs; update the `#[cfg(test)] mod tests` to import the new public providers (or keep local minimal stubs only where the seam tests need a distinct DenyProvider). Private stubs MUST NOT be re-exported. |

## Interfaces / Contracts

No new trait or signature. New public types implement the EXISTING contract:

    #[async_trait]
    impl AuthorizationProvider for AllowAllAuthorizationProvider {
        async fn authorize(&self, _: &Principal, _: &AccessRequest, _: &SecurityContext)
            -> Result<AuthorizationDecision, SecurityError> { Ok(AuthorizationDecision::Allow) }
    }

DenyAll returns `Ok(AuthorizationDecision::Deny { reason: "deny-all".to_string() })` — reason string is exactly `"deny-all"` (FR-018).

## Implementation Plan (Strict TDD)

1. Read existing stubs in `authorization/mod.rs` (done) — source of the impl bodies.
2. RED: create `allow_all/mod.rs` with doc + struct + impl skeleton and inline failing tests; same for `deny_all/mod.rs`.
3. GREEN: fill impl bodies (constant decisions).
4. Wire `providers/mod.rs` (`pub mod` + `pub use`) and `lib.rs` crate-root re-export.
5. Remove private `AlwaysAllow`/`AlwaysDeny` stubs from `authorization/mod.rs`; repoint its tests to the public providers (keep the local `DenyProvider`/`ErrorProvider` seam stubs as-is — they test distinct behaviors).
6. REFACTOR + `cargo test --workspace` green; verify `#![deny(missing_docs)]` (FR-020).

## Test Plan

| TS | Requirement | File | Test fn |
|----|-------------|------|---------|
| TS-014 | FR-017 | `providers/allow_all/mod.rs` | `allow_all_returns_allow_for_any_principal_and_request` (tokio::test) |
| TS-015 | FR-017 | `providers/allow_all/mod.rs` | `allow_all_is_send_sync` (compile-time `_assert::<AllowAllAuthorizationProvider>()`) |
| — | FR-017 | `providers/allow_all/mod.rs` | `allow_all_arc_injectable` (`Arc<dyn AuthorizationProvider>` assignment) |
| TS-016 | FR-018 | `providers/deny_all/mod.rs` | `deny_all_returns_deny_for_any_principal_and_request` (tokio::test) |
| TS-017 | FR-018 | `providers/deny_all/mod.rs` | `deny_all_reason_is_deny_all` (asserts reason == "deny-all") |
| TS-018 | FR-018 | `providers/deny_all/mod.rs` | `deny_all_is_send_sync` (compile-time assertion) |
| — | FR-018 | `providers/deny_all/mod.rs` | `deny_all_arc_injectable` |
| TS-019 | FR-019 | `providers/allow_all/mod.rs` | `crate_root_reexport_compiles` (`use ego_security_sdk::{AllowAllAuthorizationProvider, DenyAllAuthorizationProvider}`) |
| FR-020 | FR-020 | enforced by `cargo build --workspace` | missing_docs lint (no dedicated test fn) |

## Migration / Rollout

No migration. Purely additive (two new modules + re-exports) plus removal of private test-only stubs. Rollback = revert the commit; no public type changes outside the new exports.

## Open Questions

None.
