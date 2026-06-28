# Delta for security-sdk — CORE-014 Built-in Authorization Providers

## ADDED Requirements

### Requirement: FR-017 — AllowAllAuthorizationProvider

`AllowAllAuthorizationProvider` MUST be a public struct in `crates/security-sdk`.
It MUST implement `AuthorizationProvider` (FR-008) and MUST always return
`Ok(AuthorizationDecision::Allow)` regardless of the `Principal`, `AccessRequest`,
or `SecurityContext` inputs.
The type MUST be `Send + Sync` and MUST be storable as `Arc<dyn AuthorizationProvider>`.
It MUST carry a `#[doc]` comment that explicitly states it is intended for
development, integration tests, and demo runtimes only — NOT for production use.
The existing private `AlwaysAllow` stub in `authorization/mod.rs` MUST be replaced
or superseded by this public type; the private stub MUST NOT be re-exported.

#### Scenario: allow_all_returns_allow_for_any_principal_and_request

- GIVEN an `AllowAllAuthorizationProvider` instance
- AND any valid `Principal`, `AccessRequest`, and `SecurityContext`
- WHEN `authorize(principal, request, ctx).await` is called
- THEN `Ok(AuthorizationDecision::Allow)` is returned

#### Scenario: allow_all_is_send_sync

- GIVEN the `AllowAllAuthorizationProvider` type
- WHEN a compile-time bounds assertion is written: `fn _assert<T: Send + Sync>() {} _assert::<AllowAllAuthorizationProvider>()`
- THEN it compiles without error

#### Scenario: allow_all_arc_injectable

- GIVEN an `AllowAllAuthorizationProvider` instance
- WHEN it is stored as `let _: Arc<dyn AuthorizationProvider> = Arc::new(AllowAllAuthorizationProvider)`
- THEN the code compiles (object safety guarantee from FR-008 is preserved)

---

### Requirement: FR-018 — DenyAllAuthorizationProvider

`DenyAllAuthorizationProvider` MUST be a public struct in `crates/security-sdk`.
It MUST implement `AuthorizationProvider` (FR-008) and MUST always return
`Ok(AuthorizationDecision::Deny { reason: "deny-all".to_string() })` regardless
of the `Principal`, `AccessRequest`, or `SecurityContext` inputs.
The `reason` string MUST be exactly `"deny-all"`.
The type MUST be `Send + Sync` and MUST be storable as `Arc<dyn AuthorizationProvider>`.
It MUST carry a `#[doc]` comment that explicitly states it is intended for
lockdown / hardening mode and secure-by-default configurations.
The existing private `AlwaysDeny` stub in `authorization/mod.rs` MUST be replaced
or superseded by this public type; the private stub MUST NOT be re-exported.

#### Scenario: deny_all_returns_deny_for_any_principal_and_request

- GIVEN a `DenyAllAuthorizationProvider` instance
- AND any valid `Principal`, `AccessRequest`, and `SecurityContext`
- WHEN `authorize(principal, request, ctx).await` is called
- THEN `Ok(AuthorizationDecision::Deny { .. })` is returned

#### Scenario: deny_all_reason_is_deny_all

- GIVEN a `DenyAllAuthorizationProvider` instance
- WHEN `authorize(principal, request, ctx).await` is called and the result is matched
- THEN the `reason` field of the `Deny` variant equals the string `"deny-all"`

#### Scenario: deny_all_is_send_sync

- GIVEN the `DenyAllAuthorizationProvider` type
- WHEN a compile-time bounds assertion is written: `fn _assert<T: Send + Sync>() {} _assert::<DenyAllAuthorizationProvider>()`
- THEN it compiles without error

#### Scenario: deny_all_arc_injectable

- GIVEN a `DenyAllAuthorizationProvider` instance
- WHEN it is stored as `let _: Arc<dyn AuthorizationProvider> = Arc::new(DenyAllAuthorizationProvider)`
- THEN the code compiles (object safety guarantee from FR-008 is preserved)

---

### Requirement: FR-019 — Public re-export of built-in providers

`AllowAllAuthorizationProvider` and `DenyAllAuthorizationProvider` MUST be
re-exported from the crate's public API alongside `RbacProvider`.
They MUST be accessible via `ego_security_sdk::AllowAllAuthorizationProvider`
and `ego_security_sdk::DenyAllAuthorizationProvider` without requiring callers
to navigate internal module paths.
The `providers` module (or equivalent re-export path) MUST list both new types
in the same logical grouping as `RbacProvider` (FR-010).

#### Scenario: crate-root re-export compiles

- GIVEN a crate depending only on `security-sdk`
- WHEN it writes `use ego_security_sdk::{AllowAllAuthorizationProvider, DenyAllAuthorizationProvider}`
- THEN it compiles without errors

#### Scenario: providers coexist with RbacProvider in the same export path

- GIVEN the `security-sdk` public API
- WHEN all authorization provider exports are enumerated
- THEN `AllowAllAuthorizationProvider`, `DenyAllAuthorizationProvider`, and `RbacProvider` are all reachable from the same module scope

---

### Requirement: FR-020 — Missing-docs compliance for new public items

Every new public item introduced in CORE-014 MUST carry a `///` doc comment.
This requirement is a direct extension of NFR-001 to the new items.
The workspace build MUST fail (`#![deny(missing_docs)]` is already active per NFR-001)
if any of the new public types, methods, or impls lack documentation.

#### Scenario: build fails without doc comments

- GIVEN `AllowAllAuthorizationProvider` or `DenyAllAuthorizationProvider` declared without a `///` doc comment
- WHEN `cargo build --workspace` is executed
- THEN the build fails with a `missing_docs` lint error

#### Scenario: build succeeds with doc comments present

- GIVEN all new public items carry doc comments describing purpose and intended use context
- WHEN `cargo build --workspace` is executed
- THEN the build succeeds with exit code 0

---

## MODIFIED Requirements

### Requirement: FR-013 — Extensibility — new providers without modifying public contracts

The public contracts (`AuthenticationProvider`, `AuthorizationProvider`, `RoleStore`) MUST be
stable enough that a new provider crate can implement any of them without modifying
`security-sdk`'s source. The built-in `AllowAllAuthorizationProvider` and
`DenyAllAuthorizationProvider` (FR-017, FR-018) serve as reference implementations
demonstrating this contract in the crate itself.
(Previously: only external-crate extensibility was demonstrated via test stubs.)

**Given** a new crate `security-custom` that depends on `security-sdk` and implements `AuthorizationProvider`
**When** `security-custom` is compiled
**Then** it compiles without forking or patching `security-sdk`; the new provider is injectable anywhere `Arc<dyn AuthorizationProvider>` is accepted

**Given** the existing `RbacProvider`
**When** its `RoleStore` dependency is swapped to a hypothetical `PostgresRoleStore: RoleStore`
**Then** `RbacProvider`'s source code requires no modification

**Given** `AllowAllAuthorizationProvider` and `DenyAllAuthorizationProvider` in `security-sdk`
**When** their implementations are inspected
**Then** both implement `AuthorizationProvider` without modifying the trait's definition

**Test**: `extensibility::tests::external_provider_impl_compiles` — existing test. `extensibility::tests::custom_role_store_wires_into_rbac_provider` — existing test. (FR-017 and FR-018 tests in `providers::allow_all` and `providers::deny_all` modules serve as in-crate extensibility evidence.)

---

## Non-Functional Delta

These are extensions of existing NFRs — not new NFR numbers:

- **NFR-001 extension**: `AllowAllAuthorizationProvider` and `DenyAllAuthorizationProvider` MUST satisfy missing-docs at build time (covered by FR-020 above).
- **NFR-002 extension**: All new tests for FR-017 and FR-018 MUST be written before the corresponding implementations (Strict TDD, RED → GREEN → REFACTOR).

---

## Architectural Constraints

The following constraints apply to the CORE-014 implementation and all future providers built on this SPI:

- **SPI stability**: `AuthorizationProvider` trait signature MUST remain unchanged by CORE-014. Future providers are added as new implementations, not trait modifications.
- **No transport coupling**: The `AuthorizationProvider` trait MUST NOT introduce HTTP-specific types, headers, or transport concepts. Authorization logic is transport-agnostic.
- **No new extension points**: CORE-014 MUST NOT introduce additional extension traits (`PolicyProvider`, `PermissionProvider`, etc.). `AuthorizationProvider` remains the sole extension point.
- **Future provider compatibility**: The SPI design MUST allow new Level 2/3 providers (ABAC, ReBAC, OpenFGA, SpiceDB) to be added in separate crates without modifying `security-sdk`.
- **Composition reserved**: Multi-provider composition is NOT part of CORE-014. Any future `CompositeAuthorizationProvider` MUST itself implement `AuthorizationProvider`; the composition is additive, not a trait change.

---

## Test Scenario Summary

| Scenario ID | Requirement | Description |
|-------------|-------------|-------------|
| TS-014 | FR-017 | AllowAll returns Allow for arbitrary inputs |
| TS-015 | FR-017 | AllowAll compile-time Send+Sync assertion |
| TS-016 | FR-018 | DenyAll returns Deny for arbitrary inputs |
| TS-017 | FR-018 | DenyAll reason string equals "deny-all" |
| TS-018 | FR-018 | DenyAll compile-time Send+Sync assertion |
| TS-019 | FR-019 | Crate-root use statements compile for both providers |

---

## Out of Scope for This Delta

- `#[authorize("resource:action")]` proc-macro (`service-sdk-macros`) — owned exclusively by CORE-015. FR-012 already names `AccessRequest::from_permission` as the stable parsing target for this future macro; no modification to FR-012 is required here.
- Multi-provider composition, ABAC, ReBAC, policy chaining.
- JWT-based authorization (owned by `security-jwt`).
- Resource wildcards (deferred to CORE-009A per existing RbacProvider docs).
