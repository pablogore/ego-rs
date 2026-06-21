# Proposal: Security SDK

## Intent

Introduce a transport-agnostic, provider-agnostic **Security SDK** — a single cross-cutting crate (`crates/security-sdk`) that gives ego-rs one canonical model for *who is acting* (Principal), *what they presented* (Credential), *how identity is resolved* (Authentication providers), *what they may do* (Authorization providers, RBAC-first), and *how that decision-context travels* (SecurityContext) — so every layer and every future transport authenticates and authorizes against the same primitives instead of each reinventing security in its own coupled way.

## Problem

ego-rs has no unified security capability today. There is no `Principal` type, no `Credential` model, no authentication or authorization abstraction, and no security identity in the runtime context. The current `ServiceContext` (`crates/service-sdk/src/context/mod.rs`) carries tenancy, correlation, tracing, deadlines, and cancellation — but **nothing about authenticated identity**. Concretely:

- **Every transport would solve security independently.** HTTP, gRPC, and future transports each have their own auth mechanisms (headers, metadata, tokens). With no shared model, each one would resolve identity and permissions its own way, producing duplicated logic and inconsistent behavior.
- **Business logic would couple to concrete providers.** Without a provider abstraction, a command handler or service that needs "is this caller allowed?" would reach directly for JWT, Keycloak, LDAP, OpenFGA, etc. — welding business code to a vendor and making providers impossible to swap.
- **Identity cannot propagate.** A request authenticated at the transport edge has no canonical place to live as it flows through Services → Command Handlers → Persistent Entities → Projections → Scheduler. There is no `SecurityContext` to carry the authenticated `Principal`, so downstream components either re-authenticate or operate blind.
- **No consistent authorization decision.** There is no Allow/Deny vocabulary, no RBAC, and no single point that answers "may this principal perform this action on this resource?" — so integrations between modules cannot agree on what an authorization result even looks like.

Net effect: ego-rs can route and execute work deterministically, but it cannot say *who* triggered that work or *whether they were allowed to* — and any attempt to add that today would hard-couple a layer to a security vendor.

## Proposed Solution

Build `crates/security-sdk` as a **cross-cutting SDK crate** (a sibling of `service-sdk`, not a member of domain/application/infrastructure/transport) holding shared security primitives that any layer may reference without creating circular dependencies. The crate's public surface is a set of *contracts* (traits + canonical models); concrete providers are the only place real security mechanisms live, and even those are deliberately minimal in this iteration.

The change has six behavioral pillars.

### 1. Canonical identity model (Principal + Credential)

A provider-neutral vocabulary for identity:

- **`Principal`** — the authenticated actor, with a `PrincipalKind` (`User`, `Service`, `Process`, `Agent`), an opaque `SubjectId` string (e.g. `user:123`, `service:billing`, `machine:agent` — illustrative, no format enforced at the core level), a set of `Role`s, a set of `Claim`s, and arbitrary `Attribute` key-values (FR-001, FR-002, FR-003).
- **`Credential`** — what a caller presents before authentication: `Basic { username, secret }`, `Bearer(token)`, `Custom { scheme, payload }` (FR-004). Credentials are inputs to authentication, never stored on a `Principal`.

### 2. Authentication contract + minimal providers

- **`AuthenticationProvider`** — an object-safe async trait, `async fn authenticate(&self, credential: &Credential) -> Result<Principal, SecurityError>`. No HTTP/gRPC types anywhere in the signature (FR-005). Providers that need tenant or environment context receive it at construction time via dependency injection, not at call time.
- **`BasicAuthenticationProvider`** — validates `Basic` credentials against an injected verifier (FR-006).

> **Note:** JWT authentication deferred to CORE-009A.

### 3. Authorization contract + RBAC

- **`AuthorizationProvider`** — an object-safe async trait, `async fn authorize(&self, principal: &Principal, request: &AccessRequest, ctx: &SecurityContext) -> Result<AuthorizationDecision, SecurityError>` (FR-008). `AccessRequest` names a `Resource` + `Action`; `AuthorizationDecision` is `Allow` or `Deny { reason }` (FR-009).
- **`RbacProvider`** — evaluates `Permission`s mapped to `Role`s for `Resource`/`Action` pairs (FR-010). It depends on a **`RoleStore` trait** (async) from day one for role/permission lookup, never a concrete type; the shipped backend is `InMemoryRoleStore`. Future backends (PostgreSQL, Redis, LDAP, OpenFGA, SpiceDB) plug in without touching the provider (FR-013, ASSUMPTION-003).

### 4. SecurityContext (explicit propagation)

- **`SecurityContext`** holds the authenticated `Principal` (non-optional — if `SecurityContext` is present, a Principal is guaranteed) plus any decision-relevant scope, and is **propagated explicitly** — no thread-local, no task-local, no global, no implicit ambient state (FR-011, ASSUMPTION-004).

### 5. Integration into ServiceContext (the critical seam)

`SecurityContext` becomes part of the runtime context in `service-sdk` as an **additive, optional field**:

```rust
// crates/service-sdk/src/context/mod.rs
pub struct ServiceContext {
    // ...existing fields (tenant_id, correlation_id, trace_id, deadline, ...)
    pub security: Option<Arc<SecurityContext>>,
}
```

`Option<Arc<SecurityContext>>` is mandatory for **backward compatibility**: every existing service and test that constructs a `ServiceContext` keeps compiling, and unauthenticated/internal paths simply carry `None`. When `security` is `Some(...)`, the `SecurityContext` inside it is guaranteed to hold a `Principal` (invariant: if `SecurityContext` is present, a Principal is guaranteed). Once set, the field is propagated unchanged by `RuntimeBuilder` and graph execution across Services, Command Handlers, Persistent Entities, Projections, and the Scheduler (FR-012). The target end-state — a `ServiceContext { TelemetryContext, SecurityContext }` grouping (ASSUMPTION-004) — is the design direction, but this change ships the additive field only and does **not** force the telemetry-grouping refactor as a hard prerequisite.

### 6. Declarative authorization integration point (Service SDK is first consumer)

Service SDK is the first official consumer, and the priority integration is declarative authorization over service operations, e.g. `#[authorize("orders:read")]` on a `#[service]` operation (ASSUMPTION-001). The Security SDK **does not implement the macro**, but it defines the exact integration point the macro will target: a stable, callable path of the form "resolve `SecurityContext` from the current `ServiceContext` → build an `AccessRequest` → call `AuthorizationProvider::authorize` → map `Deny` to a `SecurityError`." The macro (future work) only generates a call into this path.

### What this enables (extensibility)

New `AuthenticationProvider`s, `AuthorizationProvider`s, and `RoleStore` backends can be added in separate crates (`security-openfga`, `security-keycloak`, `security-ldap`, `security-oauth2`, …) **without modifying the public contracts defined here** (FR-013).

## Scope

### In scope

- New crate `crates/security-sdk` with internal modules: `principal`, `credential`, `authentication`, `authorization`, `policy`, `context`, `providers/{basic,rbac}`, `error`.
- `Principal` model: `PrincipalKind` (User/Service/Process/Agent), `SubjectId` (opaque non-empty string), `Role`, `Claim`, `Attribute`s (FR-001–FR-003).
- `Credential` model: `Basic`, `Bearer`, `Custom` (FR-004).
- `AuthenticationProvider` object-safe async trait; `BasicAuthenticationProvider` (FR-005–FR-006).
- `AuthorizationProvider` object-safe async trait; `AccessRequest` (Resource + Action); `AuthorizationDecision` (Allow/Deny) (FR-008, FR-009).
- `RbacProvider` over a `RoleStore` async trait; `InMemoryRoleStore` shipped (FR-010, FR-013, ASSUMPTION-003).
- `SecurityContext` with explicit (non-ambient) propagation holding a required authenticated `Principal` (FR-011).
- `ServiceContext` integration: add `security: Option<Arc<SecurityContext>>`; propagate via `RuntimeBuilder` + graph execution across Services, Command Handlers, Persistent Entities, Projections, Scheduler (FR-012, ASSUMPTION-004).
- Unified `SecurityError` enum (`thiserror`) covering authentication failure, authorization denial, and provider errors — leaking no provider-specific types.
- A defined, stable declarative-authorization integration point for the future `#[authorize(...)]` macro (ASSUMPTION-001).
- Tests (strict TDD, tests first): principal/credential construction, basic authentication, RBAC allow/deny over `InMemoryRoleStore`, `SecurityContext` propagation through a `ServiceContext`-carried call, error mapping.

### Out of scope (explicit non-goals)

- OAuth2 flows, OpenID Connect, LDAP, Active Directory, Keycloak.
- JWT authentication (JwtAuthenticationProvider, LocalKeyStore, HS256/RS256/ES256) — deferred to CORE-009A.
- JWKS, OIDC discovery, remote key fetching, key rotation.
- OpenFGA, SpiceDB, ABAC, ReBAC, policy DSLs.
- HTTP middleware, gRPC interceptors, any transport adapter — the security-sdk is transport-agnostic; transports only *translate* their auth into these primitives later.
- Distributed authorization.
- The `#[authorize(...)]` macro itself (only its integration point is defined here).
- Forcing the `ServiceContext { TelemetryContext, SecurityContext }` nesting refactor — only the additive `security` field ships now.
- Provider crates `security-openfga`/`security-spicedb`/`security-keycloak`/`security-ldap`/`security-oauth2` (future evolution).

## Key Design Decisions

1. **`security-sdk` is a cross-cutting SDK crate, not a layer member.** It sits beside `service-sdk` and provides shared primitives any layer may import. Rationale: authentication/authorization identity is cross-cutting; placing it in domain/application/infrastructure/transport would either pollute the domain or invite circular deps. Tradeoff: one more top-level crate to govern, but it preserves the strict layer dependency rules.

2. **Provider traits are the only seam; the core depends on no security vendor.** `AuthenticationProvider`, `AuthorizationProvider`, and `RoleStore` are object-safe async traits. Rationale: FR-005/FR-008/FR-013 demand decoupling from JWT frameworks, OAuth2, Keycloak, LDAP, OpenFGA, SpiceDB. Tradeoff: a layer of indirection for every call, in exchange for swappable providers and zero vendor lock-in in business code.

3. **`SecurityContext` integrates as `Option<Arc<SecurityContext>>` on `ServiceContext`.** Additive and optional. Rationale: `ServiceContext` is constructed in many existing services/tests (`crates/service-sdk/src/context/mod.rs`); a required field or a struct-nesting refactor would be source-breaking across the workspace. `Arc` makes propagation cheap and clone-safe across the execution graph; `Option` lets internal/unauthenticated paths carry `None`. Tradeoff: callers must handle `None` (no authenticated identity) — which is the correct, explicit default.

4. **Propagation is explicit — never ambient.** No thread-local/task-local/global storage for security identity. Rationale: ASSUMPTION-004 and determinism. EGO is a deterministic execution engine; ambient mutable security state would be a hidden input that breaks replay/testability. Tradeoff: identity must be threaded through `ServiceContext` deliberately, which is more verbose but auditable and deterministic.

5. **`RoleStore` trait from day one; `InMemoryRoleStore` as the only shipped backend.** `RbacProvider` is generic/trait-object over `RoleStore`. Rationale: ASSUMPTION-003 — future PostgreSQL/Redis/LDAP/OpenFGA/SpiceDB backends must plug in without refactoring the provider. Tradeoff: slightly more upfront abstraction than a hardcoded in-memory map, but it prevents a guaranteed future rewrite.

6. **JWT authentication is deferred to CORE-009A.** `JwtAuthenticationProvider`, `LocalKeyStore`, and HS256/RS256/ES256 support are out of scope for the Security SDK. The `AuthenticationProvider` trait is stable and will be implemented by CORE-009A. Rationale: separating JWT into its own change keeps the Security SDK focused on core primitives and contracts, and avoids the `jsonwebtoken` dependency in this crate. Tradeoff: JWT authentication is not immediately available; it ships as the first follow-on change.

7. **One `SecurityError` enum, no provider leakage.** A single `thiserror`-derived enum covers `AuthenticationFailed`, `InvalidCredential`, `AuthorizationDenied { reason }`, `ProviderError`, etc., wrapping provider failures behind a neutral variant. Rationale: FR-005/FR-008 require contracts that do not expose `jsonwebtoken`/LDAP/OpenFGA error types. Tradeoff: some provider-specific error detail is flattened into messages rather than typed variants — acceptable for a neutral contract.

8. **Declarative authorization point is defined, macro is deferred.** The Security SDK ships the callable path the `#[authorize(...)]` macro will target, but not the macro. Rationale: ASSUMPTION-001 names Service SDK as first consumer with declarative authz as priority, while the macro belongs to `service-sdk-macros` and a later change. Tradeoff: the integration contract must be designed for a consumer that does not exist yet, so it is specified explicitly to avoid rework.

9. **Object-safe async traits (`async_trait`) for all providers.** Rationale: providers are stored and invoked as `Arc<dyn AuthenticationProvider>` / `Arc<dyn AuthorizationProvider>` / `Arc<dyn RoleStore>`; object safety is required for dynamic dispatch and runtime injection. Tradeoff: `async_trait` boxing overhead, consistent with the existing `service-sdk` provider style.

## Affected Components

- `Cargo.toml` (workspace root) — add `crates/security-sdk` to workspace members.
- `crates/security-sdk/Cargo.toml` — **new**: `async-trait`, `thiserror`, `serde`/`serde_json`; `tokio` in `[dev-dependencies]` only; `#![deny(missing_docs)]`.
- `crates/security-sdk/src/lib.rs` — **new**: module tree + public re-exports of contracts and canonical models.
- `crates/security-sdk/src/principal/` — **new**: `Principal`, `PrincipalKind`, `SubjectId`, `Role`, `Claim`, attributes.
- `crates/security-sdk/src/credential/` — **new**: `Credential` (Basic/Bearer/Custom).
- `crates/security-sdk/src/authentication/` — **new**: `AuthenticationProvider` trait.
- `crates/security-sdk/src/authorization/` — **new**: `AuthorizationProvider` trait, `AccessRequest`, `AuthorizationDecision`, `Resource`, `Action`.
- `crates/security-sdk/src/policy/` — **new**: RBAC policy types (`Permission`, role→permission evaluation), `RoleStore` trait.
- `crates/security-sdk/src/context/` — **new**: `SecurityContext`.
- `crates/security-sdk/src/providers/basic/` — **new**: `BasicAuthenticationProvider`.
- `crates/security-sdk/src/providers/rbac/` — **new**: `RbacProvider`, `InMemoryRoleStore`.
- `crates/security-sdk/src/error/` — **new**: `SecurityError`.
- `crates/security-sdk/tests/` — **new**: behavioral integration tests.
- `crates/service-sdk/src/context/mod.rs` — **modified**: add `security: Option<Arc<SecurityContext>>` field + builder method; depend on `security-sdk`.
- `crates/service-sdk/src/runtime/runtime_builder.rs` — **modified**: ensure the `security` field is carried through build + graph execution (Services, Command Handlers, Persistent Entities, Projections, Scheduler).
- `crates/service-sdk/Cargo.toml` — **modified**: add `security-sdk` dependency.

## Risks

- **`ServiceContext` change is workspace-visible.** Even an additive optional field touches every construction/clone site. Risk: compile breakage if any builder uses positional struct literals. Mitigation: keep the field `Option` with a `Default`-friendly builder method; gate on `cargo test --workspace`; fix call sites in this change.
- **`TelemetryContext` does not yet exist.** ASSUMPTION-004 shows a nested target shape, but `ServiceContext` is currently flat (verified in `crates/service-sdk/src/context/mod.rs`). Risk: over-engineering a nesting refactor that is not yet warranted. Mitigation: ship the additive `security` field only; record the nesting as design direction, not a Security SDK deliverable.
- **Object safety vs. ergonomic generics.** Provider traits must stay object-safe for `Arc<dyn _>` injection. Risk: a convenience generic method breaks object safety. Mitigation: keep trait methods object-safe (`async_trait`, no generic methods, no `Self`-returning methods); push generics to constructors.
- **Determinism regression.** Security identity entering the deterministic engine must not introduce hidden inputs. Risk: ambient/implicit propagation sneaking in. Mitigation: explicit-propagation-only rule (no thread/task-local); reviewed against EGO determinism guarantees.
- **`#[deny(missing_docs)]` + coverage under strict TDD.** New public surface is large. Risk: missing docs or uncovered error branches block the build/coverage gate. Mitigation: document every public item; cover allow/deny, auth-failure, and provider-error branches explicitly with tests first.
- **Provider-error leakage.** Store errors must not appear in the public `SecurityError`. Risk: accidental `#[from]` on an external error type. Mitigation: wrap behind a neutral `ProviderError` variant; review the error enum's public type signatures.

## Success Criteria

- A `Principal` can be constructed with kind (User/Service/Process/Agent), a canonical `SubjectId`, roles, claims, and attributes; a `Credential` exists for Basic/Bearer/Custom.
- `BasicAuthenticationProvider` authenticates valid Basic credentials into a `Principal` and rejects invalid ones with a `SecurityError`.
- `RbacProvider` over `InMemoryRoleStore` returns `Allow` for a principal whose roles grant the requested Resource/Action and `Deny { reason }` otherwise.
- `SecurityContext` carries the authenticated `Principal` and propagates explicitly through a `ServiceContext`-carried call across the execution graph; no thread-local/task-local/global state is used.
- `ServiceContext` gains `security: Option<Arc<SecurityContext>>`; all existing services and tests still compile and pass with the field defaulting to `None`.
- The declarative-authorization integration path (resolve context → build `AccessRequest` → `authorize` → map `Deny` to `SecurityError`) is callable and tested, ready for a future `#[authorize(...)]` macro.
- `SecurityError` exposes no provider-specific types in its public signature.
- `cargo test --workspace` is green; `#![deny(missing_docs)]` holds for `security-sdk`.

## Dependencies

- **`async-trait`** — already a workspace dependency; required for object-safe async provider traits.
- **`thiserror`** — already a workspace dependency; used for `SecurityError`.
- **`serde` / `serde_json`** — already workspace dependencies; for serializable claims/attributes where needed (no transport coupling).
- **`tokio`** — `[dev-dependencies]` only; `#[tokio::test]` for async tests. Production code requires no Tokio runtime — `async_trait` resolves to `std::future::Future`, and `InMemoryRoleStore` uses `std::collections::HashMap` directly.
- **`service-sdk` (SPEC-008 / security-sdk change)** — first consumer; `ServiceContext` is the integration surface for `SecurityContext`. The Security SDK depends on `service-sdk`'s `ServiceContext`/`RuntimeBuilder` being present.
- No external network services, no OAuth2/OIDC/JWKS/LDAP/OpenFGA/SpiceDB dependencies in this iteration.
