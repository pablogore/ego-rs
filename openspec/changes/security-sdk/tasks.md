# Tasks: security-sdk

## Phase 1: Workspace & Crate Scaffolding

### TASK-001 — Workspace Cargo.toml: add security-sdk member

**Type**: config
**Depends on**: none
**Files**:
- `Cargo.toml` — modify — add `"crates/security-sdk"` to the `[workspace]` members array

**Description**:
Add `"crates/security-sdk"` to the workspace `members` list so the new crate is recognized by Cargo.
No production code exists yet; this task solely registers the crate with the build graph.

**Acceptance**:
- [x] `Cargo.toml` workspace members include `"crates/security-sdk"`
- [x] `cargo build --workspace` does not error on a missing member (crate directory may not exist yet, but the entry is ready)

---

### TASK-002 — New crate: `crates/security-sdk/Cargo.toml` + `src/lib.rs` skeleton

**Type**: config
**Depends on**: TASK-001
**Files**:
- `crates/security-sdk/Cargo.toml` — create — package metadata, prod and dev dependencies
- `crates/security-sdk/src/lib.rs` — create — `#![deny(missing_docs)]`, crate-level doc comment, and `pub mod` declarations for every module in the tree (initially empty)

**Description**:
Create the crate manifest and the root `lib.rs` that enforces `#![deny(missing_docs)]`.

`Cargo.toml` contents:
```toml
[package]
name = "ego-security-sdk"
version = "0.1.0"
edition = "2021"

[dependencies]
async-trait = "0.1"
thiserror = "1.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

[dev-dependencies]
mockall = "0.12"
tokio = { version = "1.0", features = ["macros", "rt-multi-thread"] }
```

`lib.rs` declares every submodule that will be created in subsequent tasks:
```
pub mod error;
pub mod principal;
pub mod credential;
pub mod authentication;
pub mod authorization;
pub mod policy;
pub mod context;
pub mod providers;
```

Each `mod.rs` can initially contain only `//! Module placeholder.` so the crate compiles.

**Acceptance**:
- [x] `cargo build -p ego-security-sdk` exits 0 with the skeleton in place
- [x] `lib.rs` starts with `#![deny(missing_docs)]`
- [x] All eight `pub mod` declarations are present
- [x] No `tokio` entry in `[dependencies]` (dev only)

---

## Phase 2: SecurityError

### TASK-003 — Tests: `SecurityError` variants and display strings

**Type**: test
**Depends on**: TASK-002
**Files**:
- `crates/security-sdk/src/error/mod.rs` — modify — add `#[cfg(test)] mod tests` block with tests written against the not-yet-compiled enum

**Description**:
Write the tests for `SecurityError` before implementing the enum.  Tests go in `crates/security-sdk/src/error/mod.rs` inside `#[cfg(test)] mod tests`.

Tests to write (all must compile-fail until TASK-004 provides the implementation):
- `display_authentication_failed` — assert `SecurityError::AuthenticationFailed("bad".into()).to_string()` contains `"authentication failed"`.
- `display_invalid_credential` — assert `SecurityError::InvalidCredential("wrong".into()).to_string()` contains `"invalid credential"`.
- `display_invalid_subject_id` — assert `SecurityError::InvalidSubjectId("".into()).to_string()` contains `"invalid subject id"`.
- `display_authorization_denied` — assert `SecurityError::AuthorizationDenied { reason: "nope".into() }.to_string()` contains `"authorization denied"`.
- `display_missing_context` — assert `SecurityError::MissingContext.to_string()` contains `"missing security context"`.
- `display_provider_error` — assert `SecurityError::ProviderError("io".into()).to_string()` contains `"provider error"`.
- `display_invalid_access_request` — assert `SecurityError::InvalidAccessRequest("bad".into()).to_string()` contains `"invalid access request"`.
- `no_external_type_in_variants` — a compile-time structural check: assert `SecurityError` implements `std::error::Error + Send + Sync + 'static`.

**Acceptance**:
- [x] All eight test bodies are written
- [x] Tests reference the exact variant names and field names specified in the design
- [x] File compiles to a state where `cargo test -p ego-security-sdk error` fails only because the enum is missing, not due to syntax errors

---

### TASK-004 — Implementation: `SecurityError` enum

**Type**: implementation
**Depends on**: TASK-003
**Files**:
- `crates/security-sdk/src/error/mod.rs` — modify — implement `SecurityError` using `thiserror`

**Description**:
Implement the full `SecurityError` enum exactly as specified in the design.

```rust
use thiserror::Error;

/// Unified, provider-neutral security error.
///
/// No third-party error type (e.g. `jsonwebtoken::Error`) appears in
/// this public surface. Provider failures are mapped to opaque strings.
#[derive(Debug, Error)]
pub enum SecurityError {
    /// Authentication ran but the credential was rejected.
    #[error("authentication failed: {0}")]
    AuthenticationFailed(String),

    /// The presented credential was malformed or of an unsupported scheme.
    #[error("invalid credential: {0}")]
    InvalidCredential(String),

    /// Subject identifier is invalid (must be non-empty).
    #[error("invalid subject id: {0}")]
    InvalidSubjectId(String),

    /// Authorization denied access.
    #[error("authorization denied: {reason}")]
    AuthorizationDenied {
        /// Why access was denied.
        reason: String,
    },

    /// No security context was present where one was required.
    #[error("missing security context")]
    MissingContext,

    /// A provider or backing store failed. Underlying cause is flattened to a
    /// string so no vendor type leaks through the public surface.
    #[error("provider error: {0}")]
    ProviderError(String),

    /// An access request descriptor was malformed (e.g. bad `"resource:action"` format).
    #[error("invalid access request: {0}")]
    InvalidAccessRequest(String),
}
```

No `#[from]` on any external type. All `From` conversions to `ProviderError` must be local to provider modules, not on the enum itself.

**Acceptance**:
- [x] All eight tests from TASK-003 pass
- [x] `cargo build -p ego-security-sdk` exits 0
- [x] `grep -r "jsonwebtoken\|#\[from\]" crates/security-sdk/src/error/` returns no results
- [x] `SecurityError: Send + Sync + 'static` (confirmed by the compile-time test)

---

## Phase 3: Value Types — SubjectId, Principal, Credential

### TASK-005 — Tests: `SubjectId` non-empty validation

**Type**: test
**Depends on**: TASK-004
**Files**:
- `crates/security-sdk/src/principal/subject_id.rs` — create — module with `#[cfg(test)] mod tests` block written first

**Description**:
Create `subject_id.rs` with only the `#[cfg(test)] mod tests` block.  The file must contain the `pub struct SubjectId` placeholder (commented out or as a forward declaration stub) so tests can reference it.

Tests to write:
- `non_empty_string_accepted` — `SubjectId::new("user:123")` returns `Ok(s)` and `s.as_str() == "user:123"`.
- `arbitrary_non_empty_accepted` — `SubjectId::new("service:billing")` returns `Ok`.
- `empty_string_rejected` — `SubjectId::new("")` returns `Err(SecurityError::InvalidSubjectId(_))`.
- `as_str_roundtrip` — `SubjectId::new("agent:x").unwrap().as_str()` equals `"agent:x"`.

**Acceptance**:
- [x] Four test bodies written in the `#[cfg(test)] mod tests` block
- [x] Tests reference `SubjectId::new`, `SubjectId::as_str`, and `SecurityError::InvalidSubjectId` by exact name

---

### TASK-006 — Implementation: `SubjectId` newtype

**Type**: implementation
**Depends on**: TASK-005
**Files**:
- `crates/security-sdk/src/principal/subject_id.rs` — modify — implement `SubjectId` struct and methods
- `crates/security-sdk/src/principal/mod.rs` — create — re-export `SubjectId`

**Description**:
Implement `SubjectId` as an opaque newtype over `String` with non-empty validation only.

```rust
use crate::error::SecurityError;

/// Opaque subject identifier — a non-empty string chosen by the provider.
///
/// No format is enforced at the core level. Examples like `"user:123"` or
/// `"service:billing"` are illustrative; the [`AuthenticationProvider`]
/// decides the actual structure.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubjectId(String);

impl SubjectId {
    /// Creates a `SubjectId` from a non-empty string.
    ///
    /// # Errors
    /// Returns [`SecurityError::InvalidSubjectId`] if `value` is empty.
    pub fn new(value: impl Into<String>) -> Result<Self, SecurityError> {
        let v = value.into();
        if v.is_empty() {
            return Err(SecurityError::InvalidSubjectId(
                "subject id must not be empty".into(),
            ));
        }
        Ok(Self(v))
    }

    /// Returns the full subject id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
```

`principal/mod.rs` re-exports: `pub use subject_id::SubjectId;` and later all principal types.

**Acceptance**:
- [x] All four tests from TASK-005 pass
- [x] Empty string returns `Err(SecurityError::InvalidSubjectId(_))` — NOT `InvalidCredential`
- [x] `as_str` returns the original non-empty value

---

### TASK-007 — Tests: `PrincipalKind`, `Role`, `Claim`, and `Principal` construction

**Type**: test
**Depends on**: TASK-006
**Files**:
- `crates/security-sdk/src/principal/principal.rs` — create — `#[cfg(test)] mod tests` block written first

**Description**:
Create `principal.rs` with the test block before any implementation.

Tests to write:
- `constructs_with_required_fields` — `Principal::new(PrincipalKind::User, subject)` sets `kind`, `subject`, and leaves `roles`, `claims`, `attributes` empty.
- `all_principal_kinds_roundtrip` — construct one `Principal` per `PrincipalKind` variant (User, Service, Process, Agent); assert `p.kind == variant`.
- `with_role_adds_to_set` — chain `.with_role(Role("admin".into()))` twice (same role); `roles` has exactly one entry (HashSet semantics).
- `with_claim_appends` — chain two distinct `.with_claim`; `claims` length is 2.
- `with_attribute_sets_key` — `.with_attribute("region", "us-east-1")`; `p.attributes["region"] == "us-east-1"`.
- `has_role_returns_true_when_present` — `has_role(&Role("admin".into()))` returns `true` after adding the role.
- `has_role_returns_false_when_absent` — `has_role(&Role("superuser".into()))` returns `false`.
- `subject_id_and_attributes` — full TS-001 scenario: kind User, subject `"user:42"`, role `"admin"`, claim `("email","alice@example.com")`, attribute `("region","eu-west-1")`; assert each accessor.

**Acceptance**:
- [x] Eight test bodies written
- [x] Tests reference exact field/method names from the design: `kind`, `subject`, `roles`, `claims`, `attributes`, `with_role`, `with_claim`, `with_attribute`, `has_role`

---

### TASK-008 — Implementation: `PrincipalKind`, `Role`, `Claim`, and `Principal`

**Type**: implementation
**Depends on**: TASK-007
**Files**:
- `crates/security-sdk/src/principal/principal.rs` — modify — implement all principal types
- `crates/security-sdk/src/principal/mod.rs` — modify — re-export all principal types

**Description**:
Implement the identity model exactly as specified in the design.

Key points:
- `PrincipalKind` must be `#[non_exhaustive]` to allow future variants without breaking existing match arms.
- `Principal::new(kind, subject)` is the only required-field constructor; optional fields start empty.
- `with_role`, `with_claim`, `with_attribute` are consuming builder methods returning `Self`.
- `roles` is `HashSet<Role>` — duplicate roles are silently deduplicated.
- `Role(pub String)` and `Claim { pub name, pub value }` are public newtype/struct.
- `Attribute` is a type alias `(String, String)`; `attributes` field is `HashMap<String, String>`.
- `has_role(&Role) -> bool` checks `self.roles.contains(role)`.

`principal/mod.rs` must re-export: `SubjectId`, `PrincipalKind`, `Principal`, `Role`, `Claim`.

**Acceptance**:
- [x] All eight tests from TASK-007 pass
- [x] `PrincipalKind` is declared `#[non_exhaustive]`
- [x] `HashSet` deduplication verified by the `with_role_adds_to_set` test
- [x] `cargo build -p ego-security-sdk` exits 0

---

### TASK-009 — Tests: `Credential` variants

**Type**: test
**Depends on**: TASK-008
**Files**:
- `crates/security-sdk/src/credential/mod.rs` — create — `#[cfg(test)] mod tests` block written first

**Description**:
Create `credential/mod.rs` with the test block before implementing the enum.

Tests to write (TS-002):
- `basic_variant_constructs_and_matches` — construct `Credential::Basic { username: "bob".into(), secret: "pw".into() }`; pattern-match and assert both fields.
- `bearer_variant_constructs_and_matches` — construct `Credential::Bearer("tok.en.here".into())`; pattern-match and assert the token string.
- `custom_variant_constructs_and_matches` — construct `Credential::Custom { scheme: "X-Api-Key".into(), payload: b"key123".to_vec() }`; pattern-match; assert `scheme` and `payload` bytes.
- `custom_payload_is_raw_bytes` — assert `payload` is `Vec<u8>`, not `String`; assign `payload: vec![0u8, 1, 2]` and assert length is 3.
- `credential_is_no_transport_type` — compile-time check: the file imports no HTTP/gRPC/tonic/axum/actix types.

**Acceptance**:
- [x] Five test bodies written
- [x] `Custom.payload` is `Vec<u8>` per the design

---

### TASK-010 — Implementation: `Credential` enum

**Type**: implementation
**Depends on**: TASK-009
**Files**:
- `crates/security-sdk/src/credential/mod.rs` — modify — implement `Credential` enum

**Description**:
Implement `Credential` with exactly three variants as specified in the design. No transport types anywhere in the enum. `Custom.payload` is `Vec<u8>`.

```rust
/// What a caller presents before authentication.
///
/// Credentials are inputs to authentication only — they are never stored on
/// a [`Principal`]. Holds no transport types (no HTTP headers, no gRPC metadata).
#[derive(Debug, Clone)]
pub enum Credential {
    /// Username + secret (Basic scheme).
    Basic {
        /// The username.
        username: String,
        /// The shared secret / password.
        secret: String,
    },
    /// A bearer token (e.g. a JWT) as an opaque string.
    Bearer(String),
    /// Any other scheme, with a free-form raw-bytes payload.
    Custom {
        /// Scheme name (e.g. `"api-key"`).
        scheme: String,
        /// Opaque raw bytes payload for that scheme.
        payload: Vec<u8>,
    },
}
```

**Acceptance**:
- [x] All five tests from TASK-009 pass
- [x] `grep -r "use http\|use hyper\|use tonic\|use axum\|use actix" crates/security-sdk/src/credential/` returns no results
- [x] `cargo build -p ego-security-sdk` exits 0

---

## Phase 4: Authentication

### TASK-011 — Tests: `AuthenticationProvider` object-safety

**Type**: test
**Depends on**: TASK-010
**Files**:
- `crates/security-sdk/src/authentication/mod.rs` — create — `#[cfg(test)] mod tests` block written first

**Description**:
Create `authentication/mod.rs` with tests before implementation.

Tests to write:
- `provider_is_object_safe` — compile-time test: define a minimal `struct StubAuthProvider;` inside the test module that implements `AuthenticationProvider` (returning `unimplemented!()`); store it as `let _: std::sync::Arc<dyn AuthenticationProvider> = std::sync::Arc::new(StubAuthProvider);`. If the trait is not object-safe this line fails to compile.
- `authenticate_signature_takes_credential_ref` — verify the signature is `async fn authenticate(&self, credential: &Credential) -> Result<Principal, SecurityError>` by calling it from the stub.

**Acceptance**:
- [x] Both test bodies written
- [x] Trait is annotated with `#[cfg_attr(test, mockall::automock)]` stub placeholder comment noting it will be added in TASK-012

---

### TASK-012 — Implementation: `AuthenticationProvider` trait

**Type**: implementation
**Depends on**: TASK-011
**Files**:
- `crates/security-sdk/src/authentication/mod.rs` — modify — implement `AuthenticationProvider` trait

**Description**:
Implement the `AuthenticationProvider` async trait using `async_trait`.

```rust
use async_trait::async_trait;
use crate::{credential::Credential, principal::Principal, error::SecurityError};

/// Resolves a presented [`Credential`] into an authenticated [`Principal`].
///
/// Object-safe and async: stored and invoked as `Arc<dyn AuthenticationProvider>`.
/// No transport types appear in this contract. Providers needing tenant or
/// environment context receive it at construction time via dependency injection.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait AuthenticationProvider: Send + Sync {
    /// Authenticates `credential` and returns the resolved [`Principal`].
    ///
    /// # Errors
    /// - [`SecurityError::AuthenticationFailed`] — credential rejected.
    /// - [`SecurityError::InvalidCredential`] — wrong scheme or malformed.
    /// - [`SecurityError::ProviderError`] — backend failure.
    async fn authenticate(
        &self,
        credential: &Credential,
    ) -> Result<Principal, SecurityError>;
}
```

No generic methods. No `Self`-returning methods. No associated types in dispatched methods.

**Acceptance**:
- [x] Both tests from TASK-011 pass
- [x] `cargo build -p ego-security-sdk` exits 0
- [x] `#[cfg_attr(test, mockall::automock)]` present

---

### TASK-013 — Tests: `BasicAuthenticationProvider` valid/invalid/wrong-variant paths

**Type**: test
**Depends on**: TASK-012
**Files**:
- `crates/security-sdk/src/providers/basic/mod.rs` — create — `#[cfg(test)] mod tests` block written first

**Description**:
Create the provider file skeleton with tests.

Tests to write (TS-003, FR-006):
- `valid_credential_authenticates` — create a `BasicAuthenticationProvider` configured for `("alice", "s3cr3t")`; call `authenticate(Credential::Basic { username: "alice", secret: "s3cr3t" }).await`; assert `Ok(p)` where `p.subject.as_str()` encodes alice's identity.
- `invalid_secret_fails` — same provider; call with `secret: "wrong"`; assert `Err(SecurityError::AuthenticationFailed(_))`.
- `non_basic_credential_rejected` — call with `Credential::Bearer("tok".into())`; assert `Err(SecurityError::InvalidCredential(_))`.
- `verifier_backend_error_surfaces_provider_error` — inject a `CredentialVerifier` stub that returns `Err(SecurityError::ProviderError("io".into()))`; call authenticate; assert the error propagates as `Err(SecurityError::ProviderError(_))`.

**Acceptance**:
- [x] Four test bodies written
- [x] Tests reference `CredentialVerifier` trait and `BasicAuthenticationProvider::new(Arc<dyn CredentialVerifier>)` constructor shape

---

### TASK-014 — Implementation: `CredentialVerifier` trait + `BasicAuthenticationProvider`

**Type**: implementation
**Depends on**: TASK-013
**Files**:
- `crates/security-sdk/src/providers/basic/mod.rs` — modify — implement both types
- `crates/security-sdk/src/providers/mod.rs` — create — re-export `basic` module

**Description**:
Implement `CredentialVerifier` (object-safe async trait) and `BasicAuthenticationProvider` (delegating to it).

`CredentialVerifier`:
```rust
#[async_trait]
pub trait CredentialVerifier: Send + Sync {
    /// Verifies `secret` for `username`.
    ///
    /// Returns `Ok(Some(principal))` on success, `Ok(None)` when credentials
    /// don't match, and `Err(SecurityError::ProviderError)` for backend failure.
    async fn verify(
        &self,
        username: &str,
        secret: &str,
    ) -> Result<Option<Principal>, SecurityError>;
}
```

`BasicAuthenticationProvider` holds `Arc<dyn CredentialVerifier>` and delegates to it. The `authenticate` method:
1. Matches `Credential::Basic` — calls `verifier.verify`.
2. `Some(p)` → `Ok(p)`.
3. `None` → `Err(SecurityError::AuthenticationFailed("invalid username or password".into()))`.
4. Non-`Basic` variant → `Err(SecurityError::InvalidCredential("BasicAuthenticationProvider requires a Basic credential".into()))`.

Provide a constructor: `pub fn new(verifier: Arc<dyn CredentialVerifier>) -> Self`.

For tests, provide a simple `InMemoryVerifier { username: String, secret: String }` inside `#[cfg(test)]` that returns a hardcoded `Principal` on match and `None` on mismatch.

**Acceptance**:
- [x] All four tests from TASK-013 pass
- [x] Provider error from verifier backend propagates unchanged (verified by `verifier_backend_error_surfaces_provider_error` test)
- [x] `cargo build -p ego-security-sdk` exits 0

---

## Phase 5: `SecurityContext`

### TASK-021 — Tests: `SecurityContext` construction and no ambient state

**Type**: test
**Depends on**: TASK-008
**Files**:
- `crates/security-sdk/src/context/mod.rs` — create — `#[cfg(test)] mod tests` block written first

**Description**:
Create `context/mod.rs` with the test block before implementing the struct.

Tests to write (FR-011):
- `constructs_from_principal` — `SecurityContext::new(principal)` sets `ctx.principal()` to the same principal.
- `with_scope_adds_entry` — `SecurityContext::new(p).with_scope("tenant", "t1")`; `ctx.scope["tenant"] == "t1"`.
- `principal_is_non_optional` — assert `SecurityContext::new(p).principal().subject.as_str()` is accessible directly (no `Option` unwrap needed).
- `no_ambient_state_leak` — construct two `SecurityContext` values from different principals; assert each holds its own principal and neither references the other. This test implicitly verifies no thread-local/global is in play because neither context affects the other.
- `is_clone_and_send_sync` — compile-time: `fn assert_send_sync<T: Send + Sync + Clone>() {}; assert_send_sync::<SecurityContext>();`.

**Acceptance**:
- [x] Five test bodies written
- [x] `principal()` return type is `&Principal`, NOT `Option<&Principal>`

---

### TASK-022 — Implementation: `SecurityContext`

**Type**: implementation
**Depends on**: TASK-021
**Files**:
- `crates/security-sdk/src/context/mod.rs` — modify — implement `SecurityContext`

**Description**:
Implement `SecurityContext` exactly as specified in the design.

```rust
use std::collections::HashMap;
use crate::principal::Principal;

/// Carries the authenticated [`Principal`] plus decision-relevant scope.
///
/// Propagated **explicitly** through `ServiceContext` — no thread-local,
/// no task-local, no global, no implicit ambient state.
///
/// **Invariant**: if a `SecurityContext` exists, a [`Principal`] is guaranteed.
/// `principal` is non-optional by design.
#[derive(Debug, Clone)]
pub struct SecurityContext {
    /// The authenticated principal — always present.
    pub principal: Principal,
    /// Decision-relevant scope key/values (e.g. tenant, environment).
    pub scope: HashMap<String, String>,
}

impl SecurityContext {
    /// Creates a context for the given authenticated principal.
    pub fn new(principal: Principal) -> Self {
        Self { principal, scope: HashMap::new() }
    }
    /// Builder: adds a scope entry.
    pub fn with_scope(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.scope.insert(key.into(), value.into());
        self
    }
    /// Returns the authenticated principal.
    pub fn principal(&self) -> &Principal {
        &self.principal
    }
}
```

No `thread_local!`, no `task_local!`, no `once_cell`, no `lazy_static`.

**Acceptance**:
- [x] All five tests from TASK-021 pass
- [x] `grep -rn "thread_local\|task_local\|lazy_static\|once_cell" crates/security-sdk/src/context/` returns no results
- [x] `SecurityContext: Send + Sync + Clone` confirmed by compile-time test
- [x] `cargo build -p ego-security-sdk` exits 0

---

## Phase 6: Authorization Types

### TASK-015 — Tests: `AccessRequest`, `Resource`, `Action`, and `AuthorizationDecision`

**Type**: test
**Depends on**: TASK-004
**Files**:
- `crates/security-sdk/src/authorization/access_request.rs` — create — `#[cfg(test)] mod tests` block written first
- `crates/security-sdk/src/authorization/decision.rs` — create — `#[cfg(test)] mod tests` block written first

**Description**:
Create both files with test blocks before implementation.

Tests for `access_request.rs`:
- `constructs_from_resource_and_action` — `AccessRequest::new(Resource { kind: "orders".into(), id: None }, Action("read".into()))` succeeds; assert `request.resource.kind == "orders"` and `request.action.0 == "read"`.
- `resource_with_instance_id` — construct with `id: Some("order-42".into())`; assert `resource.id.as_deref() == Some("order-42")`.
- `from_permission_parses_valid_descriptor` — `AccessRequest::from_permission("orders:read")` returns `Ok(req)` where `req.resource.kind == "orders"` and `req.action.0 == "read"`.
- `from_permission_rejects_missing_colon` — `AccessRequest::from_permission("bad")` returns `Err(SecurityError::InvalidAccessRequest(_))`.
- `from_permission_rejects_empty_resource` — `AccessRequest::from_permission(":action")` returns `Err(SecurityError::InvalidAccessRequest(_))`.
- `from_permission_rejects_empty_action` — `AccessRequest::from_permission("resource:")` returns `Err(SecurityError::InvalidAccessRequest(_))`.

Tests for `decision.rs`:
- `allow_variant_is_allowed` — `AuthorizationDecision::Allow.is_allowed()` returns `true`.
- `deny_variant_is_not_allowed` — `AuthorizationDecision::Deny { reason: "x".into() }.is_allowed()` returns `false`.
- `deny_reason_accessible` — pattern-match `Deny { reason }` and assert content.

**Acceptance**:
- [x] Nine test bodies written across the two files
- [x] No transport types imported

---

### TASK-016 — Implementation: `AccessRequest`, `Resource`, `Action`, `AuthorizationDecision`

**Type**: implementation
**Depends on**: TASK-015
**Files**:
- `crates/security-sdk/src/authorization/access_request.rs` — modify — implement types
- `crates/security-sdk/src/authorization/decision.rs` — modify — implement `AuthorizationDecision`
- `crates/security-sdk/src/authorization/mod.rs` — create — `AuthorizationProvider` trait stub + re-exports + `authorize_in_context` stub

**Description**:
Implement the authorization model types.

`Resource`, `Action`, `AccessRequest` exactly as in the design. `AccessRequest::new(resource, action)` constructor. Also implement `AccessRequest::from_permission(descriptor)` as specified in the design — parses `"<resource>:<action>"` using `split_once(':')`, rejects missing colon or empty segments, returns `SecurityError::InvalidAccessRequest` on failure.

`AuthorizationDecision`:
```rust
/// Outcome of an authorization evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationDecision {
    /// Access is granted.
    Allow,
    /// Access is denied with a human-readable reason.
    Deny {
        /// Why access was denied.
        reason: String,
    },
}

impl AuthorizationDecision {
    /// Returns `true` for [`Allow`][AuthorizationDecision::Allow].
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }
}
```

`authorization/mod.rs` re-exports all authorization types and will receive `AuthorizationProvider` (TASK-017) and `authorize_in_context` (TASK-025). Add module-level doc comment now.

**Acceptance**:
- [x] All nine tests from TASK-015 pass
- [x] `AccessRequest::from_permission` implemented; parses `"resource:action"` correctly
- [x] `cargo build -p ego-security-sdk` exits 0

---

### TASK-017 — Tests: `AuthorizationProvider` object-safety and decision variants

**Type**: test
**Depends on**: TASK-016, TASK-022
**Files**:
- `crates/security-sdk/src/authorization/mod.rs` — modify — add `#[cfg(test)] mod tests` block

**Description**:
Add authorization provider tests before the trait is fully wired.

Tests to write:
- `provider_is_object_safe` — define stub `struct AlwaysAllow;` implementing `AuthorizationProvider` in the test module returning `Ok(AuthorizationDecision::Allow)`; store as `Arc<dyn AuthorizationProvider>`. Compile-time check.
- `allow_and_deny_are_matchable` — using `AlwaysAllow` and a `struct AlwaysDeny` stub, call `authorize` on each; assert `Allow` matches allow arm and `Deny { reason }` matches deny arm (FR-009).
- `external_provider_impl_compiles` — same as `provider_is_object_safe` but explicitly named to cover FR-013/TS-010.

**Acceptance**:
- [x] Three test bodies written
- [x] Tests reference `AuthorizationProvider::authorize` with all four parameters: `&Principal`, `&AccessRequest`, `&SecurityContext`

---

### TASK-018 — Implementation: `AuthorizationProvider` trait

**Type**: implementation
**Depends on**: TASK-017
**Files**:
- `crates/security-sdk/src/authorization/mod.rs` — modify — add `AuthorizationProvider` trait definition

**Description**:
Implement the `AuthorizationProvider` async trait.

```rust
use async_trait::async_trait;
use crate::{
    principal::Principal,
    authorization::{AccessRequest, AuthorizationDecision},
    context::SecurityContext,
    error::SecurityError,
};

/// Decides whether a [`Principal`] may perform the action named by an
/// [`AccessRequest`].
///
/// Object-safe; invoked as `Arc<dyn AuthorizationProvider>`. A clean Deny
/// is returned as `Ok(Deny { .. })`, NOT an error — only backend failures
/// return `Err(SecurityError)`.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait AuthorizationProvider: Send + Sync {
    /// Evaluates the request and returns an [`AuthorizationDecision`].
    async fn authorize(
        &self,
        principal: &Principal,
        request: &AccessRequest,
        ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError>;
}
```

**Acceptance**:
- [x] All three tests from TASK-017 pass
- [x] `#[cfg_attr(test, mockall::automock)]` present
- [x] No generic methods; no `Self`-returning methods

---

## Phase 7: Policy — `Permission` and `RoleStore`

### TASK-019 — Tests: `Permission` equality/hash and `RoleStore` contract

**Type**: test
**Depends on**: TASK-004
**Files**:
- `crates/security-sdk/src/policy/mod.rs` — create — `#[cfg(test)] mod tests` block written first

**Description**:
Create `policy/mod.rs` with tests before the types are implemented.

Tests to write:
- `permission_equality` — two `Permission { resource: "orders".into(), action: "read".into() }` instances are equal.
- `permission_hashes_consistently` — same permission hashes to the same value when inserted into `HashSet`.
- `role_store_is_object_safe` — compile-time test: define stub `struct StubStore;` implementing `RoleStore` (returning `Ok(vec![])`) and store as `Arc<dyn RoleStore>`.
- `unknown_role_returns_empty_vec` — using `InMemoryRoleStore` (forward reference): a role not in the store returns `Ok(vec![])` not an error.
- `known_role_returns_permissions` — `InMemoryRoleStore` with `Role("admin") → [Permission { resource: "orders", action: "read" }]`; assert `permissions_for_role` returns the vec.

**Acceptance**:
- [x] Five test bodies written
- [x] `permissions_for_role` method name exact (not `get_permissions` or similar)

---

### TASK-020 — Implementation: `Permission`, `RoleStore` trait, `InMemoryRoleStore`

**Type**: implementation
**Depends on**: TASK-019
**Files**:
- `crates/security-sdk/src/policy/mod.rs` — modify — implement `Permission`, `RoleStore` trait, and re-export `InMemoryRoleStore`
- `crates/security-sdk/src/policy/in_memory_role_store.rs` — create — implement `InMemoryRoleStore` (concrete `RoleStore` backend)

**Description**:
Implement policy types exactly as specified.

`Permission`:
```rust
/// A grant of `action` on a resource kind, mapped to roles by the [`RoleStore`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Permission {
    /// Resource kind this permission applies to (e.g. `"orders"`).
    pub resource: String,
    /// Action granted (e.g. `"read"`). `"*"` matches any action.
    pub action: String,
}
```

`RoleStore` — object-safe async trait with `#[cfg_attr(test, mockall::automock)]`:
```rust
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait RoleStore: Send + Sync {
    /// Returns the permissions granted by `role`.
    ///
    /// An unknown role returns `Ok(Vec::new())`, not an error.
    async fn permissions_for_role(
        &self,
        role: &Role,
    ) -> Result<Vec<Permission>, SecurityError>;
}
```

`InMemoryRoleStore`:
```rust
pub struct InMemoryRoleStore {
    grants: std::collections::HashMap<Role, Vec<Permission>>,
}

impl InMemoryRoleStore {
    /// Creates an empty store.
    pub fn new() -> Self { Self { grants: HashMap::new() } }
    /// Builder: grants `permissions` to `role`.
    pub fn with_role(mut self, role: Role, permissions: Vec<Permission>) -> Self {
        self.grants.insert(role, permissions);
        self
    }
}
```

Unknown role → `Ok(vec![])`, never an error.

**Acceptance**:
- [x] All five tests from TASK-019 pass
- [x] `#[cfg_attr(test, mockall::automock)]` on `RoleStore`
- [x] `InMemoryRoleStore::new()` starts empty; `with_role` is a builder
- [x] `cargo build -p ego-security-sdk` exits 0

---

## Phase 8: `RbacProvider`

### TASK-023 — Tests: `RbacProvider` allow/deny paths

**Type**: test
**Depends on**: TASK-020, TASK-022
**Files**:
- `crates/security-sdk/src/providers/rbac/mod.rs` — create — `#[cfg(test)] mod tests` block written first

**Description**:
Create `providers/rbac/mod.rs` with tests before implementation.

Tests to write (FR-010, TS-006, TS-007):
- `role_grants_allow` — `InMemoryRoleStore` with `Role("editor") → [Permission { resource: "posts", action: "write" }]`; `Principal` with role `Role("editor")`; `authorize` with `AccessRequest { resource: "posts", action: "write" }` returns `Ok(Allow)`.
- `missing_role_returns_deny` — same store; `Principal` with role `Role("viewer")` (not mapped); same request returns `Ok(Deny { reason: _ })`.
- `wildcard_action_grants_allow` — `Permission { resource: "data", action: "*" }`; request for `"data"/"delete"` returns `Ok(Allow)`.
- `unknown_role_contributes_nothing` — principal with unknown role (not in store) returns `Ok(Deny { .. })` (store returns empty vec, not error).
- `custom_role_store_compiles` — use `MockRoleStore` (from `mockall::automock` on `RoleStore`) instead of `InMemoryRoleStore`; construct `RbacProvider::new(Arc::new(mock_store))`; assert compilation (no `InMemoryRoleStore` import needed).
- `provider_error_propagates` — `MockRoleStore` configured to return `Err(SecurityError::ProviderError("db down".into()))`; `authorize` returns `Err(SecurityError::ProviderError(_))`.

**Acceptance**:
- [x] Six test bodies written
- [x] `custom_role_store_compiles` test does NOT import `InMemoryRoleStore` — proving `RbacProvider` depends only on the trait

---

### TASK-024 — Implementation: `RbacProvider`

**Type**: implementation
**Depends on**: TASK-023
**Files**:
- `crates/security-sdk/src/providers/rbac/mod.rs` — modify — implement `RbacProvider`
- `crates/security-sdk/src/providers/mod.rs` — modify — add `pub mod rbac; pub use rbac::RbacProvider;`

**Description**:
Implement `RbacProvider` backed by `Arc<dyn RoleStore>`.

Evaluation algorithm:
1. For each role on the principal, call `store.permissions_for_role(role).await?`.
2. A permission grants the request iff `perm.resource == request.resource.kind && (perm.action == request.action.0 || perm.action == "*")`.
3. First matching permission → `Allow`. No match across all roles → `Deny { reason: format!("no permission for {}:{}", resource, action) }`.

```rust
pub struct RbacProvider {
    store: std::sync::Arc<dyn crate::policy::RoleStore>,
}

impl RbacProvider {
    /// Creates a provider backed by the given [`RoleStore`].
    pub fn new(store: std::sync::Arc<dyn crate::policy::RoleStore>) -> Self {
        Self { store }
    }
}
```

`RbacProvider` module must NOT import `InMemoryRoleStore` — only `Arc<dyn RoleStore>`.

`InMemoryRoleStore` lives in `policy/in_memory_role_store.rs` (implemented in TASK-020). `providers/rbac/mod.rs` must NOT reference it — `RbacProvider` depends only on `Arc<dyn RoleStore>`. `InMemoryRoleStore` is made public from the crate root via `policy`'s re-export.

**Acceptance**:
- [x] All six tests from TASK-023 pass
- [x] `RbacProvider`'s source file has no direct `use ... InMemoryRoleStore` import
- [x] `cargo build -p ego-security-sdk` exits 0

---

## Phase 9: `authorize_in_context` helper

### TASK-025 — Tests: `authorize_in_context` happy path, Deny mapping, missing context

**Type**: test
**Depends on**: TASK-018, TASK-022
**Files**:
- `crates/security-sdk/src/authorization/mod.rs` — modify — add tests for the free function

**Description**:
Add tests for `authorize_in_context` before implementing it.

Tests to write:
- `allow_returns_ok_unit` — `MockAuthorizationProvider` returns `Allow`; call `authorize_in_context(Some(&ctx), resource, action, &*mock)`; assert `Ok(())`.
- `deny_maps_to_authorization_denied` — `MockAuthorizationProvider` returns `Deny { reason: "no role".into() }`; assert `Err(SecurityError::AuthorizationDenied { reason })` where `reason == "no role"`.
- `none_security_returns_missing_context` — call with `None`; assert `Err(SecurityError::MissingContext)`.
- `provider_error_propagates` — `MockAuthorizationProvider` returns `Err(SecurityError::ProviderError("x".into()))`; assert error propagates.

**Acceptance**:
- [x] Four test bodies written using `MockAuthorizationProvider` from `mockall`

---

### TASK-026 — Implementation: `authorize_in_context` free function

**Type**: implementation
**Depends on**: TASK-025
**Files**:
- `crates/security-sdk/src/authorization/mod.rs` — modify — add `pub async fn authorize_in_context`

**Description**:
Implement the stable seam for the future `#[authorize(...)]` macro.

```rust
/// Resolves the security context, builds an [`AccessRequest`], calls the
/// authorization provider, and maps a [`Deny`] decision to a
/// [`SecurityError::AuthorizationDenied`].
///
/// This is the stable seam a future `#[authorize("resource:action")]` macro
/// targets. The caller passes the already-resolved `Option<&SecurityContext>`
/// (extracted from `ctx.security`) so this function remains ego-dep-free.
///
/// # Errors
/// - [`SecurityError::MissingContext`] if `security` is `None`.
/// - [`SecurityError::AuthorizationDenied`] if the decision is `Deny`.
/// - Propagates any provider error.
pub async fn authorize_in_context(
    security: Option<&SecurityContext>,
    resource: Resource,
    action: Action,
    provider: &dyn AuthorizationProvider,
) -> Result<(), SecurityError> {
    let sec = security.ok_or(SecurityError::MissingContext)?;
    let principal = sec.principal();
    let request = AccessRequest::new(resource, action);
    match provider.authorize(principal, &request, sec).await? {
        AuthorizationDecision::Allow => Ok(()),
        AuthorizationDecision::Deny { reason } => {
            Err(SecurityError::AuthorizationDenied { reason })
        }
    }
}
```

Location: `crates/security-sdk/src/authorization/mod.rs` (per design decision).

**Acceptance**:
- [x] All four tests from TASK-025 pass
- [x] Function signature matches exactly: `(Option<&SecurityContext>, Resource, Action, &dyn AuthorizationProvider) -> Result<(), SecurityError>`
- [x] `cargo build -p ego-security-sdk` exits 0

---

## Phase 10: `lib.rs` Public Re-exports and `missing_docs` Gate

### TASK-027 — Tests: `#![deny(missing_docs)]` enforcement

**Type**: test
**Depends on**: TASK-026
**Files**:
- `crates/security-sdk/src/lib.rs` — modify — finalize all re-exports and run doc completeness check

**Description**:
This is a verification task. Wire all submodule re-exports into `lib.rs` and confirm every public item is documented.

Re-exports to add (if not already present):
```rust
pub use error::SecurityError;
pub use principal::{SubjectId, PrincipalKind, Principal, Role, Claim};
pub use credential::Credential;
pub use authentication::AuthenticationProvider;
pub use authorization::{
    AuthorizationProvider, AccessRequest, Resource, Action,
    AuthorizationDecision, authorize_in_context,
};
pub use policy::{Permission, RoleStore};
pub use context::SecurityContext;
pub use providers::{
    basic::{BasicAuthenticationProvider, CredentialVerifier},
    rbac::RbacProvider,
};
pub use policy::InMemoryRoleStore;
```

Run `cargo doc -p ego-security-sdk --no-deps` and confirm it exits 0 (fails if any public item lacks a doc comment). Fix any missing doc comments found.

**Acceptance**:
- [x] `cargo doc -p ego-security-sdk --no-deps` exits 0
- [x] `cargo build -p ego-security-sdk` exits 0 (which enforces `#![deny(missing_docs)]`)
- [x] All public items re-exported from `lib.rs`

---

## Phase 11: `service-sdk` Integration

### TASK-028 — Tests: `ServiceContext` backward compatibility before field addition

**Type**: test
**Depends on**: TASK-022
**Files**:
- `crates/service-sdk/tests/security_integration.rs` — create — test file written before the field is added

**Description**:
Create a new integration test file for the security integration. Write tests that will exercise the field BEFORE it is added, so the first test (`security_field_defaults_to_none`) must pass before TASK-029 makes any code change.

Tests to write (FR-012, INT-001):
- `security_field_defaults_to_none` — construct `ServiceContext::new()` (without specifying `security`); assert `.security.is_none()`. This test must compile and pass BEFORE TASK-029 — it will fail to compile until the field exists (that compile failure is the "red" state for TDD).
- `security_field_set_via_builder` — construct `ServiceContext::new().with_security(Arc::new(ctx))`; assert `service_ctx.security.is_some()`.
- `security_propagates_through_chain` — construct a `ServiceContext` with `security: Some(arc_ctx)`, pass it to a closure simulating a handler, and assert `service_ctx.security.as_ref().unwrap().principal().subject.as_str()` equals the original subject (TS-008).
- `existing_construction_sites_compile` — construct `ServiceContext` using every existing builder call in the codebase; this test acts as a compile-time regression check that adding the field does not break old code.

**Acceptance**:
- [x] Four test bodies written
- [x] `security_field_defaults_to_none` and `security_field_set_via_builder` are the primary red-state tests that gate TASK-029

---

### TASK-029 — Implementation: add `security: Option<Arc<SecurityContext>>` to `ServiceContext`

**Type**: integration
**Depends on**: TASK-028
**Files**:
- `crates/service-sdk/Cargo.toml` — modify — add `ego-security-sdk = { path = "../security-sdk" }` to `[dependencies]`
- `crates/service-sdk/src/context/mod.rs` — modify — add the `security` field, `with_security` builder, and `security()` accessor

**Description**:
Add the `ego-security-sdk` dependency to `service-sdk` and wire the new field.

`Cargo.toml` diff:
```toml
[dependencies]
# ... existing deps ...
ego-security-sdk = { path = "../security-sdk" }
```

`context/mod.rs` changes:
1. Add import: `use std::sync::Arc; use ego_security_sdk::context::SecurityContext;`
2. Add field to struct: `/// Attached security context carrying the authenticated principal, if any.\n pub security: Option<Arc<SecurityContext>>,`
3. Add `security: None` to the `Self { ... }` literal in `ServiceContext::new()`.
4. Add builder method:
   ```rust
   /// Attaches a security context (authenticated identity + scope).
   pub fn with_security(mut self, security: Arc<SecurityContext>) -> Self {
       self.security = Some(security);
       self
   }
   ```
5. Add accessor:
   ```rust
   /// Returns the attached security context, if any.
   pub fn security(&self) -> Option<&Arc<SecurityContext>> {
       self.security.as_ref()
   }
   ```

No other existing field is renamed, removed, or retyped. No `Default` impl change needed (it delegates to `new()`).

**Acceptance**:
- [x] All four tests from TASK-028 pass
- [x] `cargo test --workspace` exits 0 (all existing tests still green, `security: None` default keeps them compiling)
- [x] `grep -rn "thread_local\|task_local\|lazy_static\|once_cell" crates/service-sdk/src/context/` returns no security-context-related hits
- [x] INV-007: `ServiceContext::clone()` preserves the `security` field unchanged; verified by test

---

## Phase 12: Integration Tests (security-sdk tests/ directory)

### TASK-030 — Integration test: `tests/basic_auth.rs`

**Type**: test
**Depends on**: TASK-014
**Files**:
- `crates/security-sdk/tests/basic_auth.rs` — create — full integration test file

**Description**:
Write the integration test covering `BasicAuthenticationProvider` end-to-end.

Tests (from design):
- `injected_verifier_returns_principal` — verifier returns `Some(p)`; `authenticate(Basic{...})` returns `Ok(p)`.
- `injected_verifier_returns_none_gives_auth_failed` — verifier returns `None`; authenticate returns `Err(AuthenticationFailed(_))`.
- `non_basic_credential_returns_invalid_credential` — `Credential::Bearer` gives `Err(InvalidCredential(_))`.
- `verifier_backend_error_gives_provider_error` — verifier returns `Err(ProviderError(...))` ; authenticate returns `Err(ProviderError(_))`.

Each test must use `#[tokio::test]` runtime.

**Acceptance**:
- [x] All four tests pass under `cargo test --workspace`
- [x] No `InMemoryVerifier` is exported from the production code — test-only helpers are in `#[cfg(test)]` blocks

---

### TASK-031 — Integration test: `tests/rbac.rs`

**Type**: test
**Depends on**: TASK-024
**Files**:
- `crates/security-sdk/tests/rbac.rs` — create — full RBAC integration test file

**Description**:
Write the integration test for RBAC allow/deny paths.

Tests (from design, TS-006 + TS-007):
- `principal_role_grants_allow` — `InMemoryRoleStore` with `Role("admin") → [Permission { resource: "orders", action: "read" }]`; `Principal` with role `"admin"`; `authorize` returns `Ok(Allow)`.
- `missing_role_returns_deny` — principal with `Role("viewer")` (not in store); same request returns `Ok(Deny { reason: _ })`.
- `wildcard_action_grants_any_action` — store with `Permission { resource: "data", action: "*" }`; request for `"data"/"delete"` returns `Ok(Allow)`.
- `unknown_role_empty_perms_deny` — principal with unknown role; store returns `Ok([])` (not error); `authorize` returns `Ok(Deny { .. })`.
- `mock_role_store_injectable` — `MockRoleStore` (no `InMemoryRoleStore` import); `RbacProvider::new(Arc::new(mock))` compiles and evaluates correctly.

**Acceptance**:
- [x] All five tests pass under `cargo test --workspace`

---

### TASK-032 — Integration test: `tests/context_propagation.rs`

**Type**: test
**Depends on**: TASK-029
**Files**:
- `crates/security-sdk/tests/context_propagation.rs` — create — context propagation integration test

**Description**:
Write the test verifying `SecurityContext` propagates through a `ServiceContext`-shaped call chain without ambient state (from design).

Tests:
- `security_context_carried_via_explicit_passing` — construct a `SecurityContext` from a principal; set it on `ServiceContext`; pass `ServiceContext` to a mock handler closure; inside the handler, assert `sec_ctx.principal().subject.as_str()` equals the original subject.
- `two_independent_contexts_do_not_share_state` — construct two `SecurityContext` values from different principals; wrap each in a `ServiceContext`; pass each to a handler; assert neither context observes the other's principal. This confirms no thread-local leakage.
- `inv_007_clone_preserves_security_field` — construct `ServiceContext::new().with_security(Arc::clone(&arc))`; clone the context; assert `cloned.security.is_some()` and `Arc::ptr_eq(original.security.as_ref().unwrap(), cloned.security.as_ref().unwrap())` — same `Arc` pointer, not a new allocation. Verifies INV-007.

Import `ego_security_sdk` and `ego_service_sdk` directly.

**Acceptance**:
- [x] All three tests pass under `cargo test --workspace`
- [x] Test file contains no `thread_local`, `task_local`, or ambient access pattern

---

### TASK-033 — Integration test: `tests/declarative_authz.rs`

**Type**: test
**Depends on**: TASK-026, TASK-029
**Files**:
- `crates/security-sdk/tests/declarative_authz.rs` — create — `authorize_in_context` integration test

**Description**:
Write the declarative authorization integration tests (TS-011, from design).

Tests:
- `allow_returns_ok_unit` — `MockAuthorizationProvider` returns `Allow`; `authorize_in_context(Some(&ctx), resource, action, &*mock)` returns `Ok(())`.
- `deny_returns_authorization_denied` — `MockAuthorizationProvider` returns `Deny { reason: "no role".into() }`; assert `Err(SecurityError::AuthorizationDenied { reason })` where `reason == "no role"`.
- `none_security_returns_missing_context` — `authorize_in_context(None, ...)` returns `Err(SecurityError::MissingContext)`.
- `full_path_allow` — wire `RbacProvider` with `InMemoryRoleStore`; principal has role granting access; `authorize_in_context` returns `Ok(())`.
- `full_path_deny` — same wiring; principal lacks required role; `authorize_in_context` returns `Err(SecurityError::AuthorizationDenied { .. })`.

**Acceptance**:
- [x] All five tests pass under `cargo test --workspace`

---

### TASK-034 — Integration test: `tests/error_mapping.rs`

**Type**: test
**Depends on**: TASK-024
**Files**:
- `crates/security-sdk/tests/error_mapping.rs` — create — error neutrality integration test

**Description**:
Write the test verifying that provider/store failures surface as neutral `SecurityError` with no vendor type leakage (NFR-004, from design).

Tests:
- `role_store_failure_gives_provider_error` — `MockRoleStore` returns `Err(SecurityError::ProviderError("db down".into()))`; `RbacProvider::authorize` returns `Err(SecurityError::ProviderError(_))`.
- `provider_error_display_contains_no_vendor_name` — assert `.to_string()` on the returned error does NOT contain `"jsonwebtoken"`, `"ldap"`, or `"openfga"`.
- `authentication_provider_error_is_neutral` — `MockAuthenticationProvider` returns `Err(SecurityError::ProviderError("internal".into()))`; the caller receives `SecurityError::ProviderError` with no external type.

**Acceptance**:
- [x] All three tests pass under `cargo test --workspace`
- [x] No `jsonwebtoken` import in `crates/security-sdk/src/`

---

## Phase 13: Final Verification

### TASK-035 — Workspace compile and docs verification

**Type**: docs
**Depends on**: TASK-027, TASK-029, TASK-030, TASK-031, TASK-032, TASK-033, TASK-034
**Files**:
- Any file with a missing doc comment (identified during this task's run)

**Description**:
Final verification sweep.

1. Run `cargo test --workspace` — must exit 0 with all tests green.
2. Run `cargo build --workspace` — must exit 0 (enforces `#![deny(missing_docs)]` on security-sdk).
3. Run `cargo doc -p ego-security-sdk --no-deps` — must exit 0.
4. Run `grep -r "use http\|use hyper\|use tonic\|use axum\|use actix" crates/security-sdk/src/` — must return no results (NFR-003).
5. Run `grep -rn "thread_local\|task_local\|lazy_static\|once_cell::sync::Lazy" crates/security-sdk/src/ crates/service-sdk/src/context/` — must return no security-context-related hits (NFR-005).

Fix any missing doc comment or import violation found — this task is cleanup only, no new production logic.

**Acceptance**:
- [x] `cargo test --workspace` exits 0
- [x] `cargo build --workspace` exits 0
- [x] `cargo doc -p ego-security-sdk --no-deps` exits 0
- [x] NFR-003 grep returns no results
- [x] NFR-005 grep returns no security-related hits

---

## Parallelism Notes

| Task | Depends on | Can run in parallel with |
|---|---|---|
| TASK-001 | — | — |
| TASK-002 | TASK-001 | — |
| TASK-003 | TASK-002 | — |
| TASK-004 | TASK-003 | — |
| TASK-005 | TASK-004 | TASK-009, TASK-015, TASK-019 |
| TASK-006 | TASK-005 | — |
| TASK-007 | TASK-006 | — |
| TASK-008 | TASK-007 | TASK-009, TASK-015, TASK-019 |
| TASK-009 | TASK-008 | TASK-007 (post TASK-004) |
| TASK-010 | TASK-009 | — |
| TASK-011 | TASK-010 | — |
| TASK-012 | TASK-011 | — |
| TASK-013 | TASK-012 | — |
| TASK-014 | TASK-013 | — |
| TASK-015 | TASK-004 | TASK-021, TASK-019 |
| TASK-016 | TASK-015 | TASK-021, TASK-022 |
| TASK-017 | TASK-016, TASK-022 | — |
| TASK-018 | TASK-017 | — |
| TASK-019 | TASK-004 | TASK-005, TASK-015, TASK-021 |
| TASK-020 | TASK-019 | — |
| TASK-021 | TASK-008 | TASK-015, TASK-019 |
| TASK-022 | TASK-021 | TASK-015, TASK-016 |
| TASK-023 | TASK-020, TASK-022 | TASK-017 |
| TASK-024 | TASK-023 | — |
| TASK-025 | TASK-018, TASK-022 | TASK-023 |
| TASK-026 | TASK-025 | TASK-024 |
| TASK-027 | TASK-026 | — |
| TASK-028 | TASK-022 | TASK-020, TASK-024 |
| TASK-029 | TASK-028 | TASK-027 |
| TASK-030 | TASK-014 | TASK-031, TASK-032 |
| TASK-031 | TASK-024 | TASK-030, TASK-032 |
| TASK-032 | TASK-029 | TASK-030, TASK-031 |
| TASK-033 | TASK-026, TASK-029 | TASK-030, TASK-031, TASK-034 |
| TASK-034 | TASK-024 | TASK-030, TASK-031, TASK-032, TASK-033 |
| TASK-035 | TASK-027, TASK-029–034 | — |

## Invariant Coverage Map

| Invariant / NFR | Covered by task(s) |
|---|---|
| INV-001: Provider object safety | TASK-011, TASK-012, TASK-017, TASK-018, TASK-019, TASK-020 |
| INV-002: Credential not stored on Principal | TASK-007, TASK-008 (no `credential` field on `Principal`) |
| INV-003: SecurityError neutrality (no vendor types) | TASK-003, TASK-004, TASK-034 |
| INV-004: JWT deferred to CORE-009A | TASK-002 (no `jsonwebtoken` dep in Cargo.toml) |
| INV-005: SecurityContext explicit-origin-only | TASK-021, TASK-022 |
| INV-006: ServiceContext additive-only | TASK-028, TASK-029 |
| INV-007: SecurityContext propagation preserved on clone | TASK-029, TASK-032 |
| NFR-001: `#![deny(missing_docs)]` | TASK-002, TASK-027, TASK-035 |
| NFR-002: Workspace test gate | TASK-035 |
| NFR-003: No transport types | TASK-009, TASK-010, TASK-035 |
| NFR-004: SecurityError neutrality | TASK-003, TASK-004, TASK-034 |
| NFR-005: No ambient security state | TASK-022, TASK-032, TASK-035 |
| INT-001: ServiceContext backward compat | TASK-028, TASK-029 |

## FR / Test Scenario Coverage Map

| FR / TS | Covered by task(s) |
|---|---|
| FR-001 (Principal construction) | TASK-007, TASK-008 |
| FR-002 (PrincipalKind variants) | TASK-007, TASK-008 |
| FR-003 (SubjectId opaque + attributes) | TASK-005, TASK-006, TASK-007, TASK-008 |
| FR-004 (Credential variants) | TASK-009, TASK-010 |
| FR-005 (AuthenticationProvider object-safe) | TASK-011, TASK-012 |
| FR-006 (BasicAuthenticationProvider paths) | TASK-013, TASK-014, TASK-030 |
| FR-008 (AuthorizationProvider object-safe) | TASK-017, TASK-018 |
| FR-009 (AuthorizationDecision Allow/Deny) | TASK-015, TASK-016 |
| FR-010 (RbacProvider + RoleStore) | TASK-019, TASK-020, TASK-023, TASK-024, TASK-031 |
| FR-011 (SecurityContext explicit) | TASK-021, TASK-022 |
| FR-012 (SecurityContext via ServiceContext) | TASK-028, TASK-029, TASK-032 |
| FR-013 (Extensibility — new providers) | TASK-017, TASK-023 |
| TS-001 (Principal full construction) | TASK-007, TASK-008 |
| TS-002 (Credential variants) | TASK-009, TASK-010 |
| TS-003 (Basic auth valid/invalid) | TASK-013, TASK-014, TASK-030 |
| TS-006 (RBAC allow) | TASK-023, TASK-024, TASK-031 |
| TS-007 (RBAC deny) | TASK-023, TASK-024, TASK-031 |
| TS-008 (SecurityContext propagation) | TASK-028, TASK-029, TASK-032 |
| TS-009 (ServiceContext backward compat) | TASK-028, TASK-029 |
| TS-010 (Extensibility compile) | TASK-017, TASK-023 |
| TS-011 (Declarative authz path) | TASK-025, TASK-026, TASK-033 |

---

## Review Workload Forecast

- Estimated tasks: 35
- Estimated changed lines: ~2,360 (recalculated — original ~900 estimate omitted test lines and doc comments)
  - **PR-1 — Core Models** (Phases 1–3, 5; TASK-001–010, TASK-021–022): ~735 lines
    - Phase 1 (Scaffolding — workspace Cargo.toml + new crate): ~65 lines
    - Phase 2 (SecurityError — production + unit tests): ~110 lines
    - Phase 3 (SubjectId, Principal, Credential — production + unit tests): ~430 lines
    - Phase 5 (SecurityContext — production + unit tests): ~130 lines
  - **PR-2 — Providers** (Phases 4, 6–10; TASK-011–020, TASK-023–027): ~1,190 lines
    - Phase 4 (AuthenticationProvider + BasicAuthenticationProvider — production + unit tests): ~305 lines
    - Phase 6 (Authorization types + AuthorizationProvider — production + unit tests): ~335 lines
    - Phase 7 (Policy + RoleStore + InMemoryRoleStore — production + unit tests): ~235 lines
    - Phase 8 (RbacProvider — production + unit tests): ~185 lines
    - Phase 9 (authorize_in_context — production + unit tests): ~130 lines
    - Phase 10 (lib.rs re-exports — updates to existing file): ~0 net lines
  - **PR-3 — ServiceContext Integration** (Phases 11–13; TASK-028–035): ~440 lines
    - Phase 11 (service-sdk Cargo.toml + context/mod.rs): ~60 lines
    - Phase 12 (Integration tests — 5 files): ~380 lines
    - Phase 13 (Final verification): ~0 new production lines
- New files: ~22 (Cargo.toml + src modules + 5 integration test files)
- Modified files: ~3 (`Cargo.toml` workspace root + `crates/service-sdk/Cargo.toml` + `crates/service-sdk/src/context/mod.rs`)
- Chained PRs recommended: Yes — 3 PRs, stacked-to-main.
  - PR-1 (Core Models, ~735 lines): new crate + pure value types + SecurityContext; no consumer-facing contracts yet
  - PR-2 (Providers, ~1,190 lines): all authentication + authorization contracts + providers; size:exception justified — no logical split point within the auth/authz surface
  - PR-3 (ServiceContext Integration, ~440 lines): additive service-sdk changes + all 5 integration tests
- 400-line budget risk: Critical — PR-2 exceeds budget (~1,190 lines); size:exception required before sdd-apply PR-2. PR-1 (~735 lines) also exceeds budget — justified by strict TDD coupling of production types with their inline unit tests. PR-3 (~440 lines) marginal.
- Decision needed before apply: Yes — size:exception must be confirmed for PR-1 and PR-2 before sdd-apply begins.
