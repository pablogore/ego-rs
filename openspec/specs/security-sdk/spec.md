# Spec: Security SDK

## Overview

Introduce `crates/security-sdk` — a transport-agnostic, provider-agnostic security crate that gives ego-rs one canonical model for identity (`Principal`), credentials (`Credential`), authentication (`AuthenticationProvider`), authorization (`AuthorizationProvider` + RBAC), and security context propagation (`SecurityContext`). Service SDK is the first consumer: `ServiceContext` gains an additive, optional `security: Option<Arc<SecurityContext>>` field. All existing code compiles unchanged with the field defaulting to `None`.

---

## Requirements

### FR-001: Principal — construction and identity model

`Principal` is a value type representing a successfully authenticated actor. It MUST carry:
- `kind: PrincipalKind` — one of `User`, `Service`, `Process`, `Agent`
- `subject_id: SubjectId` — an opaque non-empty string (e.g. `user:123`, `service:billing`, `machine:agent` — illustrative examples, no format enforced at the core level)
- `roles: HashSet<Role>` — a set of zero or more assigned roles
- `claims: Vec<Claim>` — zero or more name/value claims
- `attributes: HashMap<String, String>` — arbitrary key-value metadata

**Given** the required fields `kind`, `subject_id`, and empty collections for roles, claims, and attributes
**When** `Principal::new(kind, subject_id)` (or builder equivalent) is called
**Then** a valid `Principal` is returned with `kind` and `subject_id` set and empty collections

**Test**: `principal::tests::constructs_with_required_fields` — assert each field matches the input; roles/claims/attributes are empty.

---

### FR-002: Principal — PrincipalKind variants

`PrincipalKind` MUST define exactly four variants: `User`, `Service`, `Process`, `Agent`. The enum MUST be non-exhaustive to allow future extension without breaking existing match arms.

**Given** the four variant names
**When** a `Principal` is constructed with each `PrincipalKind` variant
**Then** `principal.kind()` returns the same variant; pattern matching on a known variant compiles and matches correctly

**Test**: `principal::tests::all_principal_kinds_roundtrip` — construct one `Principal` per kind; assert `kind()` equals the expected variant.

---

### FR-003: Principal — SubjectId as opaque string and arbitrary attributes

The system SHALL support stable subject identifiers as opaque strings (non-empty). `SubjectId` is a newtype over `String` with only non-empty validation — no `<kind>:<id>` format is enforced at the core level; the `AuthenticationProvider` decides how to interpret the identifier. Attributes MUST accept arbitrary string key-value pairs beyond the fixed `kind`/`subject_id`/`roles`/`claims` fields.

**Given** `subject_id = "user:abc-123"` (illustrative) and attributes `{"department": "engineering", "region": "us-east-1"}`
**When** a `Principal` is constructed with those values
**Then** `principal.subject_id()` returns `"user:abc-123"` and `principal.attribute("department")` returns `Some("engineering")`

**Given** an empty string for `subject_id`
**When** construction is attempted
**Then** the call returns `Err(SecurityError::InvalidSubjectId)` — the `SubjectId` newtype rejects empty strings at construction time with the typed error variant dedicated to subject id validation

**Test**: `principal::tests::subject_id_and_attributes` — assert roundtrip for a multi-attribute principal; assert empty `subject_id` is rejected.

---

### FR-004: Credential — Basic, Bearer, and Custom variants

`Credential` MUST define three variants:
- `Basic { username: String, secret: String }` — cleartext username/password pair
- `Bearer(String)` — an opaque token string (e.g. JWT)
- `Custom { scheme: String, payload: Vec<u8> }` — extensibility escape hatch

No variant contains transport types (no `http::HeaderValue`, no `tonic::metadata::MetadataValue`, etc.).

**Given** each variant's required fields
**When** each `Credential` variant is constructed
**Then** the variant is pattern-matchable and its fields are accessible

**Test**: `credential::tests::all_variants_construct_and_match` — construct one of each variant; assert pattern matching returns the expected field values.

---

### FR-005: AuthenticationProvider — object-safe async trait

`AuthenticationProvider` MUST be an object-safe async trait with the following signature:

```rust
#[async_trait]
pub trait AuthenticationProvider: Send + Sync {
    async fn authenticate(
        &self,
        credential: &Credential,
    ) -> Result<Principal, SecurityError>;
}
```

The trait MUST contain no generic methods, no `Self`-returning methods, and no transport types anywhere in its signature. It MUST be storable as `Arc<dyn AuthenticationProvider>`. Providers that need tenant or environment context receive it at construction time via dependency injection, not at call time.

**Given** a concrete struct that implements `AuthenticationProvider`
**When** it is stored as `Arc<dyn AuthenticationProvider>`
**Then** the code compiles without trait-object safety errors

**Test**: `authentication::tests::provider_is_object_safe` — compile-time test: `let _: Arc<dyn AuthenticationProvider> = Arc::new(stub);` must compile.

---

### FR-006: BasicAuthenticationProvider — valid and invalid credential paths

`BasicAuthenticationProvider` MUST be constructed with an injected `Arc<dyn CredentialVerifier>` that owns the verification logic. The provider itself holds no credentials — it delegates all username/secret validation to the injected verifier.

`BasicAuthenticationProvider` MUST:
- Accept a `Credential::Basic` and delegate to `CredentialVerifier::verify` → return `Ok(Principal)` on success
- Return `Err(SecurityError::AuthenticationFailed)` when the verifier rejects the credential
- Return `Err(SecurityError::ProviderError)` when the verifier returns a backend error
- Reject any non-`Basic` credential variant → return `Err(SecurityError::InvalidCredential)`

**Given** a `BasicAuthenticationProvider` constructed with a `CredentialVerifier` that accepts `("alice", "s3cr3t")`
**When** `authenticate(Credential::Basic { username: "alice", secret: "s3cr3t" })` is called
**Then** `Ok(principal)` is returned where `principal.subject_id()` encodes the username

**Given** the same provider
**When** `authenticate(Credential::Basic { username: "alice", secret: "wrong" })` is called
**Then** `Err(SecurityError::AuthenticationFailed)` is returned

**Given** the same provider
**When** `authenticate(Credential::Bearer("some-token".to_string()))` is called
**Then** `Err(SecurityError::InvalidCredential)` is returned (no verifier call is made)

**Given** the same provider and a verifier that returns a backend error
**When** `authenticate(Credential::Basic { username: "alice", secret: "s3cr3t" })` is called
**Then** `Err(SecurityError::ProviderError)` is returned

**Test**: `providers::basic::tests::valid_credential_authenticates`, `invalid_secret_fails`, `non_basic_credential_rejected`, `backend_error_maps_to_provider_error`.

---

### FR-008: AuthorizationProvider — object-safe async trait

`AuthorizationProvider` MUST be an object-safe async trait with the following signature:

```rust
#[async_trait]
pub trait AuthorizationProvider: Send + Sync {
    async fn authorize(
        &self,
        principal: &Principal,
        request: &AccessRequest,
        ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError>;
}
```

`AccessRequest` MUST carry `resource: Resource` and `action: Action` as value types (not transport types). The trait MUST be storable as `Arc<dyn AuthorizationProvider>`.

**Given** a concrete struct that implements `AuthorizationProvider`
**When** it is stored as `Arc<dyn AuthorizationProvider>`
**Then** the code compiles without trait-object safety errors

**Test**: `authorization::tests::provider_is_object_safe` — compile-time test: `let _: Arc<dyn AuthorizationProvider> = Arc::new(stub);`.

---

### FR-009: AuthorizationDecision — Allow and Deny variants

`AuthorizationDecision` MUST define exactly two variants:
- `Allow` — the request is permitted
- `Deny { reason: String }` — the request is denied with a human-readable reason

The decision type MUST NOT expose provider-internal types in its fields.

**Given** an `AuthorizationProvider` stub that returns `AuthorizationDecision::Allow`
**When** `authorize(principal, request, ctx).await` is called
**Then** the result is `Ok(AuthorizationDecision::Allow)` and is pattern-matchable as `Allow`

**Given** an `AuthorizationProvider` stub that returns `AuthorizationDecision::Deny { reason: "insufficient role".to_string() }`
**When** `authorize(principal, request, ctx).await` is called
**Then** the result is `Ok(AuthorizationDecision::Deny { reason })` and `reason` is accessible

**Test**: `authorization::tests::allow_and_deny_are_matchable`.

---

### FR-010: RbacProvider and RoleStore — allow/deny and backend swappability

`RbacProvider` MUST evaluate whether a `Principal`'s assigned roles grant the `Permission` implied by `(Resource, Action)`, consulting a `RoleStore` trait for role-to-permission lookups. It MUST:
- Return `Ok(AuthorizationDecision::Allow)` when at least one of the principal's roles has the required permission
- Return `Ok(AuthorizationDecision::Deny { reason })` when none of the principal's roles has the required permission
- Depend only on the `RoleStore` trait — never on `InMemoryRoleStore` or any concrete backend type

`RoleStore` MUST be an object-safe async trait:

```rust
#[async_trait]
pub trait RoleStore: Send + Sync {
    async fn permissions_for_role(
        &self,
        role: &Role,
    ) -> Result<Vec<Permission>, SecurityError>;
}
```

`InMemoryRoleStore` MUST implement `RoleStore` and accept role-to-permission mappings at construction time.

**Given** an `InMemoryRoleStore` with mapping `Role("admin") → [Permission("orders", "read")]`
**And** a `RbacProvider` wrapping that store
**And** a `Principal` with role `Role("admin")`
**When** `authorize(principal, AccessRequest { resource: "orders", action: "read" }, &ctx).await` is called
**Then** `Ok(AuthorizationDecision::Allow)` is returned

**Given** the same setup
**And** a `Principal` with role `Role("viewer")` (not mapped to `orders:read`)
**When** the same request is made
**Then** `Ok(AuthorizationDecision::Deny { reason: _ })` is returned

**Given** a `RbacProvider` constructed with a custom `Arc<dyn RoleStore>` (not `InMemoryRoleStore`)
**When** it is compiled
**Then** no import of `InMemoryRoleStore` is required by the provider module

**Test**: `providers::rbac::tests::role_grants_allow`, `missing_role_returns_deny`, `custom_role_store_compiles`.

---

### FR-011: SecurityContext — requires Principal, explicit construction

`SecurityContext` MUST:
- Hold an authenticated `Principal` as a **required, non-optional field** — `SecurityContext` cannot exist without a `Principal`. Invariant: if `SecurityContext` is present, a `Principal` is guaranteed.
- Be constructable from a `Principal` without ambient state, thread-locals, or globals
- Expose `principal(&self) -> &Principal`
- Be `Clone`, `Send`, and `Sync`

**Given** a `Principal` value
**When** `SecurityContext::new(principal)` is called
**Then** `ctx.principal()` returns a reference to that principal

**Given** two threads constructing `SecurityContext` independently
**When** each accesses its own context
**Then** neither context leaks state to the other (no shared global/thread-local)

**Test**: `context::tests::constructs_from_principal`, `context::tests::no_ambient_state_leak` — construct two contexts in parallel; assert each holds its own principal independently.

---

### FR-012: SecurityContext — explicit propagation through ServiceContext

The existing field `security: Option<Arc<SecurityContext>>` on `ServiceContext` (introduced by the Security SDK) is unchanged. This change defines the semantics and modifies `authorize_in_context` behavior:

- `security == None` means "security capability not installed in this runtime" — a valid deployment state, not a propagation failure
- `security == Some(arc_ctx)` means "capability installed; `arc_ctx.principal()` is always valid"
- The field MUST default to `None` when not set
- The field MUST be propagated unchanged through all runtime execution paths
- The field MUST never be resolved via thread-local, task-local, or any global mechanism

When `security == None`, `authorize_in_context` MUST return `SecurityError::CapabilityNotEnabled` instead of `SecurityError::MissingContext`. The `MissingContext` variant is retained in the enum for potential future internal-invariant detection.
(Previously: `authorize_in_context` returned `SecurityError::MissingContext` when `security == None`)

#### Scenario: Backward compatibility — field defaults to None

- GIVEN an existing test that constructs a `ServiceContext` without specifying `security`
- WHEN the test is compiled after this change
- THEN it compiles without errors or warnings (the field defaults to `None`)

#### Scenario: Security propagates through call chain unchanged

- GIVEN a `SecurityContext` wrapped in `Some(Arc::new(ctx))`
- WHEN a `ServiceContext` carrying that field is passed through a `RuntimeBuilder`-wired call chain
- THEN the receiving component reads the same `SecurityContext` from `service_ctx.security`

#### Scenario: authorize_in_context returns CapabilityNotEnabled for unconfigured runtime

- GIVEN a `ServiceContext` with `security == None` (valid state: no security installed)
- WHEN `authorize_in_context` is called
- THEN `Err(SecurityError::CapabilityNotEnabled)` is returned

#### Scenario: MissingContext retained in enum

- GIVEN the `SecurityError` enum definition
- WHEN all variants are enumerated
- THEN `MissingContext` is present alongside `CapabilityNotEnabled`

**Test**: `service_sdk::context::tests::security_field_defaults_to_none` — construct `ServiceContext` without the field; assert `security.is_none()`. `service_sdk::context::tests::security_propagates_through_chain` — wire a call chain, assert `security` field is identical at both ends. `authorization::tests::none_security_returns_capability_not_enabled` — assert `Err(CapabilityNotEnabled)` is returned for unconfigured runtime. `declarative_authz::none_security_returns_capability_not_enabled` — same assertion in declarative authorization path.

`AccessRequest::from_permission(descriptor)` is the stable parsing API that a future `#[authorize(...)]` macro targets. Its format is `"<resource>:<action>"` — exactly one colon separator, neither segment empty.

---

### FR-014: SecurityError::CapabilityNotEnabled variant

`SecurityError` MUST include a `CapabilityNotEnabled` variant with no payload fields. This variant represents "security was never installed in the runtime" — distinct from `MissingContext` which represents "capability exists but was not propagated".

#### Scenario: Variant matches correctly

- GIVEN a `SecurityError::CapabilityNotEnabled` value
- WHEN the value is pattern-matched
- THEN it matches the `CapabilityNotEnabled` arm (no payload)

#### Scenario: Returned when runtime has no security

- GIVEN a runtime built without `.with_security()`
- WHEN `authorize_in_context` is called on a `ServiceContext` with `security == None`
- THEN `Err(SecurityError::CapabilityNotEnabled)` is returned

---

### FR-013: Extensibility — new providers without modifying public contracts

The public contracts (`AuthenticationProvider`, `AuthorizationProvider`, `RoleStore`) MUST be stable enough that a new provider crate can implement any of them without modifying `security-sdk`'s source.

**Given** a new crate `security-custom` that depends on `security-sdk` and implements `AuthorizationProvider`
**When** `security-custom` is compiled
**Then** it compiles without forking or patching `security-sdk`; the new provider is injectable anywhere `Arc<dyn AuthorizationProvider>` is accepted

**Given** the existing `RbacProvider`
**When** its `RoleStore` dependency is swapped to a hypothetical `PostgresRoleStore: RoleStore`
**Then** `RbacProvider`'s source code requires no modification

**Test**: `extensibility::tests::external_provider_impl_compiles` — define a stub `struct AlwaysAllow` implementing `AuthorizationProvider` in the test module; assert it compiles and is injectable as `Arc<dyn AuthorizationProvider>`. `extensibility::tests::custom_role_store_wires_into_rbac_provider` — define a stub `RoleStore` impl; construct `RbacProvider` with it; assert compilation succeeds.

---

## Non-Functional Requirements

### NFR-001: Missing docs enforcement

`crates/security-sdk/src/lib.rs` MUST include `#![deny(missing_docs)]`. Every public type, trait, method, field, and constant in the crate MUST carry a doc comment. The workspace build (`cargo build --workspace`) MUST fail if any public item lacks documentation.

**Verification**: `cargo build --workspace` returns exit code 0. If a doc comment is removed from any public item, the build returns a non-zero exit code with a `missing_docs` error.

---

### NFR-002: Workspace test gate

`cargo test --workspace` MUST pass with all existing tests green and all new security-sdk tests green. No existing test may be modified to pass by relaxing assertions — only by updating construction sites to include `security: None`.

---

### NFR-003: No transport types in provider traits

No public trait, type, or method signature in `security-sdk` may import from `http`, `hyper`, `tonic`, `axum`, `actix-web`, or any HTTP/gRPC messaging library. This is verified structurally: `grep -r "use http\|use hyper\|use tonic\|use axum\|use actix" crates/security-sdk/src/` MUST return no results.

---

### NFR-004: No provider-specific types in SecurityError

`SecurityError` MUST expose no provider-specific or library-specific types in its public variants or fields. Specifically: no `jsonwebtoken::Error`, no LDAP error types, no OpenFGA error types. Provider errors MUST be wrapped behind a neutral `ProviderError(String)` or equivalent opaque variant.

**Verification**: Inspect `SecurityError` public variants; none reference an external crate's error type directly in the variant payload.

---

### NFR-005: No Ambient Security State

No code in `security-sdk` or `service-sdk` MUST store `SecurityContext` or `ServiceContext`
in a thread-local, task-local (`tokio::task_local!`), or global (`static`, `once_cell`,
`lazy_static`). The security field travels exclusively through explicit `ServiceContext`
passing. The service context itself MUST also travel exclusively through explicit parameter
passing — no task-local `CURRENT_CONTEXT` for `ServiceContext` is permitted.

(Previously: covered `SecurityContext` ambient storage only; this delta extends the prohibition
to `ServiceContext` task-local/thread-local/global patterns, aligning with CORE-010A.)

#### Scenario: No task-local or thread-local ServiceContext in codebase

- GIVEN the full workspace compiles successfully
- WHEN `grep -rn "task_local.*ServiceContext\|CURRENT_CONTEXT" crates/` is executed
- THEN zero matches are returned

#### Scenario: No task-local or thread-local SecurityContext in codebase

- GIVEN the full workspace compiles successfully
- WHEN `grep -rn "thread_local\|task_local\|lazy_static\|once_cell::sync::Lazy" crates/security-sdk/src/ crates/service-sdk/src/context/` is executed
- THEN zero matches related to security-context or service-context ambient storage are returned

#### Scenario: SecurityContext constructed without ambient side effects

- GIVEN a `Principal` value
- WHEN `SecurityContext::new(principal)` is called from two independent async tasks
- THEN neither task's context is visible from the other task
- AND no shared static or task-local storage is written

#### Scenario: ServiceContext not obtainable from ambient state

- GIVEN a component that needs a `ServiceContext`
- WHEN its source code is inspected
- THEN the `ServiceContext` value appears in at least one of: function parameter, constructor
  argument, or owned struct field — never in a `current()` call or task-local read

---

## Integration Requirement

### INT-001: ServiceContext backward compatibility

Every existing call site that constructs a `ServiceContext` — including all existing unit tests, integration tests, and service implementations in the workspace — MUST compile unchanged after the `security` field is added.

The `security` field MUST be reachable via a builder method (e.g. `.with_security(ctx: Arc<SecurityContext>) -> Self`) so call sites that want to set it do not need raw struct literal syntax.

**Verification**: `cargo test --workspace` passes with zero compile errors on existing construction sites before any new security-related tests are written.

---

## Invariants

**INV-001**: Provider object safety — `AuthenticationProvider`, `AuthorizationProvider`, and `RoleStore` MUST remain object-safe at all times. Adding a generic method to any of them is a breaking change.

**INV-002**: Credential immutability — a `Credential` MUST NOT be stored on a `Principal`. Credentials are inputs to authentication; only the resulting `Principal` persists.

**INV-003**: SecurityError neutrality — `SecurityError` variants MUST NOT reference any external crate's concrete error type. Internal `#[from]` conversions to opaque strings are permitted; public-surface `#[from]` to external error types are not.

**INV-004**: JWT locality — deferred to CORE-009A. No JWT dependency exists in the security-sdk.

**INV-005**: SecurityContext origin — a `SecurityContext` can only be constructed explicitly with a `Principal`. There is no `SecurityContext::default()`, no `SecurityContext::unauthenticated()`, no constructor without a `Principal`, and no ambient constructor. The `principal` field is `Principal`, not `Option<Principal>`.

**INV-006**: ServiceContext additive-only — the `security` field is the only change to `ServiceContext` in this change. No existing field is renamed, retyped, removed, or made non-`pub`. The nesting refactor (`ServiceContext { TelemetryContext, SecurityContext }`) is deferred and not part of the Security SDK.

**INV-007 — SecurityContext Propagation**: Every `ServiceContext` clone MUST preserve `security: Option<Arc<SecurityContext>>` unchanged. No runtime component may discard, replace, or set to `None` the `security` field unless explicitly documented. A `ServiceContext` constructed for a new scope inherits the `security` value from its parent context or receives `None` explicitly — it never silently loses an authenticated identity.

---

## Error Conditions

| Condition | Trigger | Expected Result |
|-----------|---------|-----------------|
| Invalid Basic credential | Wrong secret in `BasicAuthenticationProvider` | `Err(SecurityError::AuthenticationFailed)` |
| Wrong credential type | Non-`Basic` credential passed to `BasicAuthenticationProvider` | `Err(SecurityError::InvalidCredential)` |
| RBAC role not found | Principal's role not present in `RoleStore` | `Ok(AuthorizationDecision::Deny { reason })` |
| RBAC permission missing | Role present but lacks the requested `Resource:Action` | `Ok(AuthorizationDecision::Deny { reason })` |
| Provider internal error | `RoleStore` backend fails (I/O, store unreachable) | `Err(SecurityError::ProviderError(_))` — opaque, no store-specific type |
| Empty SubjectId | `SubjectId::new("")` | `Err(SecurityError::InvalidSubjectId)` — dedicated variant; `InvalidCredential` is reserved for credential scheme/format errors |
| Invalid `AccessRequest` descriptor | `AccessRequest::from_permission` with missing `:`, empty resource, or empty action | `Err(SecurityError::InvalidAccessRequest)` |
| Security not installed | `authorize_in_context` with `security == None` | `Err(SecurityError::CapabilityNotEnabled)` |

---

## Test Scenarios

### TS-001: Principal full construction
1. Construct `Principal` with kind `User`, `subject_id = "user:42"`, roles `["admin"]`, claims `[("email", "alice@example.com")]`, attributes `{"region": "eu-west-1"}`.
2. Assert `kind()` is `User`.
3. Assert `subject_id()` is `"user:42"`.
4. Assert `roles()` contains `Role("admin")`.
5. Assert `claims()` contains the email claim.
6. Assert `attribute("region")` returns `Some("eu-west-1")`.

### TS-002: Credential variant coverage
1. Construct `Credential::Basic { username: "bob", secret: "pw" }` — pattern-match; assert fields.
2. Construct `Credential::Bearer("tok.en.here")` — pattern-match; assert token string.
3. Construct `Credential::Custom { scheme: "X-Api-Key", payload: b"key123".to_vec() }` — assert scheme and payload (raw bytes).

### TS-003: Basic authentication — valid and invalid paths
1. Create a mock `CredentialVerifier` that accepts `("alice", "s3cr3t")` and rejects all other pairs.
2. Construct `BasicAuthenticationProvider::new(Arc::new(mock_verifier))`.
3. Call `authenticate(Basic { "alice", "s3cr3t" })` — assert `Ok(principal)`.
4. Call `authenticate(Basic { "alice", "bad" })` — assert `Err(AuthenticationFailed)`.
5. Call `authenticate(Bearer("…"))` — assert `Err(InvalidCredential)` (verifier is never called).
6. Create a mock verifier that returns a backend error; call `authenticate(Basic { … })` — assert `Err(ProviderError)`.

### TS-006: RBAC allow path
1. Create `InMemoryRoleStore` with `Role("editor") → [Permission("posts", "write")]`.
2. Create `RbacProvider` with that store.
3. Construct `Principal` with role `Role("editor")`.
4. Call `authorize(principal, AccessRequest { resource: "posts", action: "write" }, &ctx)`.
5. Assert `Ok(AuthorizationDecision::Allow)`.

### TS-007: RBAC deny path
1. Same store as TS-006.
2. Construct `Principal` with role `Role("viewer")` (not mapped to `posts:write`).
3. Call `authorize` with same request.
4. Assert `Ok(AuthorizationDecision::Deny { reason: _ })`.

### TS-008: SecurityContext propagation through ServiceContext
1. Authenticate a `Principal` via `BasicAuthenticationProvider::new(Arc::new(mock_verifier))`.
2. Construct `SecurityContext::new(principal)`.
3. Construct `ServiceContext` with `security: Some(Arc::new(ctx))`.
4. Pass `ServiceContext` to a mock service handler.
5. Inside the handler, assert `service_ctx.security.is_some()`.
6. Assert `service_ctx.security.as_ref().unwrap().principal().subject_id()` equals the original subject ID.

### TS-009: ServiceContext backward compatibility
1. Construct `ServiceContext` using all existing constructor patterns without providing `security`.
2. Assert the code compiles.
3. Assert `service_ctx.security.is_none()`.

### TS-010: Extensibility — external provider compiles
1. In a test module, define `struct AlwaysAllow;` implementing `AuthorizationProvider` that always returns `Allow`.
2. Store it as `let _: Arc<dyn AuthorizationProvider> = Arc::new(AlwaysAllow);`.
3. Assert compilation succeeds without modifying any `security-sdk` source.

### TS-011: Declarative authorization integration path
1. Build a `ServiceContext` with `security: Some(arc_ctx)`.
2. Extract `SecurityContext` from the context.
3. Build `AccessRequest::from_permission("orders:read")` — this is the stable parsing API targeted by the future `#[authorize(...)]` macro; format is `"<resource>:<action>"`.
4. Call `Arc<dyn AuthorizationProvider>::authorize(principal, &request, &sec_ctx).await`.
5. Map `AuthorizationDecision::Deny { reason }` to `Err(SecurityError::AuthorizationDenied { reason })`.
6. Assert `Ok(AuthorizationDecision::Allow)` for a principal with the required role; assert the mapped error for a principal without it.

### TS-012: `ServiceContext` clone preserves security field (INV-007)
1. Construct a `SecurityContext` from a `Principal` and wrap it: `let arc = Arc::new(ctx)`.
2. Construct `ServiceContext::new().with_security(Arc::clone(&arc))`.
3. Clone the `ServiceContext` (or derive a scoped child via any builder path).
4. Assert `cloned.security.is_some()`.
5. Assert `Arc::ptr_eq(original.security.as_ref().unwrap(), cloned.security.as_ref().unwrap())` — same `Arc` pointer, not a new allocation.

### TS-013: `AccessRequest::from_permission` parsing
1. `AccessRequest::from_permission("orders:read")` → `resource.kind == "orders"`, `action.0 == "read"`.
2. `AccessRequest::from_permission("bad")` (no colon) → `Err(SecurityError::InvalidAccessRequest(_))`.
3. `AccessRequest::from_permission(":action")` (empty resource) → `Err(SecurityError::InvalidAccessRequest(_))`.
4. `AccessRequest::from_permission("resource:")` (empty action) → `Err(SecurityError::InvalidAccessRequest(_))`.

---

## Strict TDD Notes

All requirements MUST have corresponding tests written BEFORE the implementation they validate. The test command is `cargo test --workspace`.

The order of test-first implementation:
1. `SecurityError` enum (no deps; needed by all others)
2. `Principal`, `PrincipalKind`, `SubjectId`, `Credential` (pure value types)
3. `AuthenticationProvider` trait + `BasicAuthenticationProvider`
4. `AuthorizationProvider` trait + `AccessRequest` + `AuthorizationDecision`
5. `RoleStore` trait + `InMemoryRoleStore` + `RbacProvider`
6. `SecurityContext`
7. `ServiceContext` integration (`security` field + builder method)
8. End-to-end propagation and declarative authorization path (TS-008, TS-011)

---

## Deferred Requirements (CORE-009A)

The following requirements are **out of scope for the Security SDK** and will be implemented in the follow-on change CORE-009A. They are recorded here for traceability.

### FR-007 [DEFERRED]: JwtAuthenticationProvider — local validation, HS256/RS256/ES256

`JwtAuthenticationProvider` MUST:
- Accept a `Credential::Bearer` containing a structurally valid JWT with a matching signature and non-expired `exp` claim, using algorithm-appropriate key material from `LocalKeyStore` → return `Ok(Principal)` with claims extracted
- Reject an expired token (past `exp`) → return `Err(SecurityError::AuthenticationFailed)`
- Reject a token with a tampered payload → return `Err(SecurityError::AuthenticationFailed)`
- Reject a token signed with a different key than the one in `LocalKeyStore` → return `Err(SecurityError::AuthenticationFailed)`
- Make NO network calls at any point during validation
- Support three algorithms: `HS256` (symmetric, HMAC secret), `RS256` (asymmetric, RSA key pair), `ES256` (asymmetric, EC key pair)

The `AuthenticationProvider` trait (FR-005) is stable in the Security SDK and will be implemented by `JwtAuthenticationProvider` in CORE-009A without modifying `security-sdk`'s source.
