# Design: Security SDK

## Architecture Overview

`security-sdk` is a **cross-cutting SDK crate** (`crates/security-sdk`, package name `ego-security-sdk`) that holds canonical security primitives. It is a sibling of `service-sdk` and is NOT a member of any layer (domain/application/infrastructure/transport). It depends on **no ego crate** — only on third-party libraries. This keeps the strict layer rules intact: any layer may import `security-sdk` without risking a cycle, because `security-sdk` imports nobody.

`service-sdk` becomes the **first consumer**: it gains a dependency on `security-sdk` and wires `SecurityContext` into `ServiceContext` as an additive optional field.

```
            ┌─────────────────────────────────────────────┐
            │              security-sdk                     │
            │  (no ego deps — only async-trait, thiserror,  │
            │   serde)                                      │
            │                                               │
            │  contracts:  AuthenticationProvider           │
            │              AuthorizationProvider            │
            │              RoleStore                         │
            │  models:     Principal / Credential /          │
            │              SecurityContext / AccessRequest  │
            │  providers:  Basic /                          │
            │              Rbac + InMemoryRoleStore         │
            └───────────────────────▲───────────────────────┘
                                     │ depends on (one-way)
            ┌───────────────────────┴───────────────────────┐
            │                service-sdk                     │
            │  ServiceContext { …, security:                 │
            │                    Option<Arc<SecurityContext>>}│
            │  RuntimeBuilder propagates `security` unchanged │
            └────────────────────────────────────────────────┘
```

**Dependency direction is one-way and strict**: `service-sdk → security-sdk`. Never the reverse. Transports (HTTP/gRPC, future) translate their wire auth into `Credential` and call providers; the Security SDK ships none of that.

### Call shape (what the future macro targets)

```
transport edge ──translate──▶ Credential
        │
        ▼
AuthenticationProvider::authenticate(&Credential) ──▶ Principal
        │  (Principal placed into SecurityContext, attached to ServiceContext)
        ▼
ServiceContext { security: Some(Arc<SecurityContext>) } ──flows through graph──▶
        │
        ▼
authorize_in_context(&ServiceContext, Resource, Action, &dyn AuthorizationProvider)
        │  resolve ctx.security → build AccessRequest → provider.authorize → map Deny
        ▼
Result<(), SecurityError>
```

## Module Structure

```
crates/security-sdk/
  Cargo.toml
  src/
    lib.rs                       #![deny(missing_docs)] + module tree + public re-exports
    error/
      mod.rs                     SecurityError (thiserror) — no provider types leak
    principal/
      mod.rs                     re-exports principal types
      subject_id.rs              SubjectId newtype + format validation
      principal.rs               Principal, PrincipalKind, Role, Claim, Attribute
    credential/
      mod.rs                     Credential (Basic / Bearer / Custom)
    authentication/
      mod.rs                     AuthenticationProvider trait (object-safe async)
    authorization/
      mod.rs                     AuthorizationProvider trait (object-safe async)
      access_request.rs          AccessRequest, Resource, Action
      decision.rs                AuthorizationDecision (Allow / Deny { reason })
    policy/
      mod.rs                     Permission, RoleStore trait (object-safe async), InMemoryRoleStore re-export
      in_memory_role_store.rs    InMemoryRoleStore — concrete RoleStore backend
    context/
      mod.rs                     SecurityContext (explicit propagation, no ambient state)
    providers/
      mod.rs                     re-exports the two providers
      basic/
        mod.rs                   BasicAuthenticationProvider + CredentialVerifier trait
      rbac/
        mod.rs                   RbacProvider (Arc<dyn RoleStore> only, no backend coupling)
  tests/
    basic_auth.rs                Basic auth happy path + rejection
    rbac.rs                      allow / deny over InMemoryRoleStore
    context_propagation.rs       SecurityContext carried via ServiceContext-shaped call
    declarative_authz.rs         authorize_in_context happy + Deny mapping
    error_mapping.rs             provider error → neutral SecurityError, no leakage
```

## Trait Signatures

All three traits are **object-safe** (no generic methods, no `Self`-returning methods, no associated types in the dispatched methods) and are invoked behind `Arc<dyn _>`. They use `#[async_trait]`, consistent with the existing `service-sdk` provider style.

### `AuthenticationProvider`

```rust
use async_trait::async_trait;
use crate::{credential::Credential, principal::Principal, error::SecurityError};

/// Resolves a presented [`Credential`] into an authenticated [`Principal`].
///
/// Object-safe and async: stored and invoked as `Arc<dyn AuthenticationProvider>`.
/// No transport types (HTTP/gRPC headers, metadata) appear in this contract.
/// Providers that need tenant or environment context receive it at construction
/// time via dependency injection, not at call time.
#[async_trait]
pub trait AuthenticationProvider: Send + Sync {
    /// Authenticates `credential` and returns the resolved [`Principal`].
    ///
    /// # Errors
    /// Returns [`SecurityError::AuthenticationFailed`] or
    /// [`SecurityError::InvalidCredential`] on rejection, and
    /// [`SecurityError::ProviderError`] for backend failures.
    async fn authenticate(
        &self,
        credential: &Credential,
    ) -> Result<Principal, SecurityError>;
}
```

### `AuthorizationProvider`

```rust
use async_trait::async_trait;
use crate::{
    principal::Principal,
    authorization::{AccessRequest, AuthorizationDecision},
    context::SecurityContext,
    error::SecurityError,
};

/// Decides whether a [`Principal`] may perform the action named by an
/// [`AccessRequest`]. Object-safe; invoked as `Arc<dyn AuthorizationProvider>`.
#[async_trait]
pub trait AuthorizationProvider: Send + Sync {
    /// Evaluates the request and returns an [`AuthorizationDecision`].
    ///
    /// A backend failure returns `Err(SecurityError)`; a clean Deny is
    /// returned as `Ok(AuthorizationDecision::Deny { .. })`, NOT an error.
    async fn authorize(
        &self,
        principal: &Principal,
        request: &AccessRequest,
        ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError>;
}
```

### `RoleStore`

```rust
use async_trait::async_trait;
use crate::{principal::Role, policy::Permission, error::SecurityError};

/// Backing store that maps a [`Role`] to the [`Permission`]s it grants.
///
/// Object-safe async trait so `RbacProvider` can hold `Arc<dyn RoleStore>`
/// and future backends (PostgreSQL, Redis, LDAP, OpenFGA, …) plug in without
/// touching the provider. Returned errors are already neutral [`SecurityError`].
#[async_trait]
pub trait RoleStore: Send + Sync {
    /// Returns the permissions granted by `role`.
    ///
    /// An unknown role returns `Ok(Vec::new())`, not an error.
    async fn permissions_for_role(&self, role: &Role) -> Result<Vec<Permission>, SecurityError>;
}
```

## Core Types

### Identity model

```rust
use std::collections::{HashMap, HashSet};

/// What kind of actor a [`Principal`] represents.
///
/// Marked `#[non_exhaustive]` so future actor categories can be added
/// without a breaking change.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrincipalKind {
    /// A human end user.
    User,
    /// A service / workload identity.
    Service,
    /// An OS-level or runtime process.
    Process,
    /// An autonomous agent.
    Agent,
}

/// Opaque subject identifier — a non-empty string chosen by the
/// [`AuthenticationProvider`]. No format is enforced at the core level.
/// Examples: `"user:123"`, `"service:billing"`, `"machine:agent"` are
/// illustrative only; the provider decides the actual structure.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubjectId(String);

impl SubjectId {
    /// Creates a `SubjectId` from a non-empty string.
    ///
    /// # Errors
    /// Returns [`SecurityError::InvalidSubjectId`] if the value is empty.
    pub fn new(value: impl Into<String>) -> Result<Self, SecurityError>;

    /// Returns the full subject id string.
    pub fn as_str(&self) -> &str;
}

/// A named role assigned to a principal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Role(pub String);

/// A typed assertion about the principal (name + value), provider-neutral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// Claim name (e.g. `"email"`, `"iss"`).
    pub name: String,
    /// Claim value as a string (JSON-encoded for structured claims).
    pub value: String,
}

/// An arbitrary key/value attribute attached to a principal.
pub type Attribute = (String, String);

/// The authenticated actor flowing through the system.
///
/// A `Principal` never stores credentials — credentials are inputs to
/// authentication only.
#[derive(Debug, Clone)]
pub struct Principal {
    /// Kind of actor.
    pub kind: PrincipalKind,
    /// Canonical, validated subject id.
    pub subject: SubjectId,
    /// Roles assigned to this principal.
    pub roles: HashSet<Role>,
    /// Claims asserted about this principal.
    pub claims: Vec<Claim>,
    /// Free-form attributes.
    pub attributes: HashMap<String, String>,
}

impl Principal {
    /// Creates a principal with the given kind and subject; empty roles/claims/attributes.
    pub fn new(kind: PrincipalKind, subject: SubjectId) -> Self;
    /// Builder: adds a role.
    pub fn with_role(self, role: Role) -> Self;
    /// Builder: adds a claim.
    pub fn with_claim(self, claim: Claim) -> Self;
    /// Builder: adds an attribute.
    pub fn with_attribute(self, key: impl Into<String>, value: impl Into<String>) -> Self;
    /// Returns true if the principal holds `role`.
    pub fn has_role(&self, role: &Role) -> bool;
}
```

### Credential

```rust
/// What a caller presents before authentication. Inputs only — never stored
/// on a `Principal`. Holds no transport types.
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
    /// Any other scheme, with a free-form payload.
    Custom {
        /// Scheme name (e.g. `"api-key"`).
        scheme: String,
        /// Opaque raw bytes payload for that scheme.
        payload: Vec<u8>,
    },
}
```

### SecurityContext

```rust
use std::collections::HashMap;

/// Carries the authenticated [`Principal`] plus decision-relevant scope.
/// Propagated **explicitly** through `ServiceContext` — no thread-local,
/// no task-local, no global, no implicit ambient state.
///
/// **Invariant**: if a `SecurityContext` exists, a `Principal` is guaranteed.
/// `principal` is non-optional by design: `SecurityContext` cannot be constructed
/// without a `Principal`.
#[derive(Debug, Clone)]
pub struct SecurityContext {
    /// The authenticated principal — always present.
    pub principal: Principal,
    /// Decision-relevant scope key/values (e.g. requested tenant, environment).
    pub scope: HashMap<String, String>,
}

impl SecurityContext {
    /// Creates a context for the given authenticated principal.
    pub fn new(principal: Principal) -> Self;
    /// Builder: adds a scope entry.
    pub fn with_scope(self, key: impl Into<String>, value: impl Into<String>) -> Self;
    /// Returns the principal.
    pub fn principal(&self) -> &Principal;
}
```

### Authorization model

```rust
/// The thing being acted upon (e.g. type `"orders"`, optional instance id).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Resource {
    /// Resource type (e.g. `"orders"`).
    pub kind: String,
    /// Optional concrete instance id.
    pub id: Option<String>,
}

/// The verb being attempted (e.g. `"read"`, `"write"`, `"delete"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Action(pub String);

/// A request to perform `action` on `resource`.
#[derive(Debug, Clone)]
pub struct AccessRequest {
    /// Target resource.
    pub resource: Resource,
    /// Attempted action.
    pub action: Action,
}

impl AccessRequest {
    /// Constructs a request.
    pub fn new(resource: Resource, action: Action) -> Self;

    /// Parses a `"resource:action"` descriptor into an [`AccessRequest`].
    ///
    /// # Format
    /// `"<resource_kind>:<action>"` — exactly one colon separator.
    ///
    /// # Errors
    /// Returns [`SecurityError::InvalidAccessRequest`] if the string does not
    /// contain exactly one `:` separator or if either segment is empty.
    ///
    /// # Example
    /// ```rust
    /// let req = AccessRequest::from_permission("orders:read").unwrap();
    /// assert_eq!(req.resource.kind, "orders");
    /// assert_eq!(req.action.0, "read");
    /// ```
    pub fn from_permission(descriptor: &str) -> Result<Self, SecurityError> {
        let (resource_kind, action) = descriptor
            .split_once(':')
            .filter(|(r, a)| !r.is_empty() && !a.is_empty())
            .ok_or_else(|| SecurityError::InvalidAccessRequest(
                format!("expected \"<resource>:<action>\", got {:?}", descriptor)
            ))?;
        Ok(Self {
            resource: Resource { kind: resource_kind.to_owned(), id: None },
            action: Action(action.to_owned()),
        })
    }
}

/// Outcome of an authorization evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationDecision {
    /// Access is granted.
    Allow,
    /// Access is denied, with a human-readable reason.
    Deny {
        /// Why access was denied.
        reason: String,
    },
}

impl AuthorizationDecision {
    /// True for `Allow`.
    pub fn is_allowed(&self) -> bool;
}
```

### Policy

```rust
/// A grant of `action` on a `resource` kind. Mapped to roles by the `RoleStore`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Permission {
    /// Resource kind this permission applies to (e.g. `"orders"`).
    pub resource: String,
    /// Action granted (e.g. `"read"`). `"*"` matches any action.
    pub action: String,
}
```

### SecurityError

```rust
use thiserror::Error;

/// Unified, provider-neutral security error. No third-party error type
/// (e.g. `jsonwebtoken::Error`) appears in this public surface.
#[derive(Debug, Error)]
pub enum SecurityError {
    /// Authentication ran but the credential was rejected.
    #[error("authentication failed: {0}")]
    AuthenticationFailed(String),

    /// The presented credential was malformed or of an unsupported scheme.
    #[error("invalid credential: {0}")]
    InvalidCredential(String),

    /// Subject identifier is invalid according to SubjectId validation rules (must be non-empty).
    #[error("invalid subject id: {0}")]
    InvalidSubjectId(String),

    /// Authorization denied access.
    #[error("authorization denied: {reason}")]
    AuthorizationDenied {
        /// Why access was denied.
        reason: String,
    },

    /// No security context / principal was present where one was required.
    #[error("missing security context")]
    MissingContext,

    /// A provider or backing store failed for a non-policy reason. The
    /// underlying cause is flattened to a string so no vendor type leaks.
    #[error("provider error: {0}")]
    ProviderError(String),

    /// An access request descriptor was malformed (e.g. bad `"resource:action"` format).
    ///
    /// Distinct from [`InvalidCredential`]: credentials and access requests are
    /// different validation domains; conflating them would obscure the failure site.
    #[error("invalid access request: {0}")]
    InvalidAccessRequest(String),
}
```

The crate **never** writes `#[from] jsonwebtoken::Error`. JWT and store failures are mapped into `ProviderError(String)` / `AuthenticationFailed(String)` at the provider boundary (see below).

## Provider Designs

### `BasicAuthenticationProvider` — injected verifier

The provider owns no credential database. It delegates secret verification to an injected, object-safe `CredentialVerifier`, so tests and real deployments swap the backend without touching the provider.

```rust
#[async_trait]
pub trait CredentialVerifier: Send + Sync {
    /// Verifies `secret` for `username`; returns the resolved principal on success.
    ///
    /// Returns `Ok(None)` for "not authenticated" (bad user/secret),
    /// `Err(SecurityError::ProviderError)` for backend failure.
    async fn verify(&self, username: &str, secret: &str)
        -> Result<Option<Principal>, SecurityError>;
}

pub struct BasicAuthenticationProvider {
    verifier: Arc<dyn CredentialVerifier>,
}

#[async_trait]
impl AuthenticationProvider for BasicAuthenticationProvider {
    async fn authenticate(&self, credential: &Credential)
        -> Result<Principal, SecurityError>
    {
        match credential {
            Credential::Basic { username, secret } => {
                match self.verifier.verify(username, secret).await? {
                    Some(p) => Ok(p),
                    None => Err(SecurityError::AuthenticationFailed(
                        "invalid username or password".into())),
                }
            }
            _ => Err(SecurityError::InvalidCredential(
                "BasicAuthenticationProvider requires a Basic credential".into())),
        }
    }
}
```

> **Note:** JWT provider design (`JwtAuthenticationProvider`, `LocalKeyStore`) is deferred to CORE-009A. The `AuthenticationProvider` trait is stable and will be implemented there.

### `RbacProvider` + `InMemoryRoleStore`

`InMemoryRoleStore` lives in `policy/in_memory_role_store.rs`, alongside `RoleStore` and `Permission`. Tests and callers import it from `policy::InMemoryRoleStore` (or via the crate-root re-export), NOT from `providers::rbac`. `providers/rbac/` contains only `RbacProvider` with no backend coupling.

`RbacProvider` is **trait-object over `RoleStore`** — it never names a concrete store:

```rust
pub struct RbacProvider {
    store: Arc<dyn RoleStore>,
}

impl RbacProvider {
    pub fn new(store: Arc<dyn RoleStore>) -> Self { Self { store } }
}

#[async_trait]
impl AuthorizationProvider for RbacProvider {
    async fn authorize(&self, principal: &Principal, request: &AccessRequest, _ctx: &SecurityContext)
        -> Result<AuthorizationDecision, SecurityError>
    {
        // Evaluation algorithm:
        // 1. For each role on the principal, fetch its permissions from the store.
        // 2. A permission grants the request iff
        //      perm.resource == request.resource.kind
        //      && (perm.action == request.action.0 || perm.action == "*").
        // 3. First matching permission → Allow. No match across all roles → Deny.
        for role in &principal.roles {
            for perm in self.store.permissions_for_role(role).await? {
                let resource_ok = perm.resource == request.resource.kind;
                let action_ok = perm.action == request.action.0 || perm.action == "*";
                if resource_ok && action_ok {
                    return Ok(AuthorizationDecision::Allow);
                }
            }
        }
        Ok(AuthorizationDecision::Deny {
            reason: format!(
                "no permission for {}:{}",
                request.resource.kind, request.action.0
            ),
        })
    }
}
```

`InMemoryRoleStore` implements `RoleStore` **exactly** (so it is interchangeable with a `mockall` mock):

```rust
pub struct InMemoryRoleStore {
    // role name -> permissions
    grants: HashMap<Role, Vec<Permission>>,
}

impl InMemoryRoleStore {
    pub fn new() -> Self;
    /// Builder: grants `permissions` to `role`.
    pub fn with_role(self, role: Role, permissions: Vec<Permission>) -> Self;
}

#[async_trait]
impl RoleStore for InMemoryRoleStore {
    async fn permissions_for_role(&self, role: &Role) -> Result<Vec<Permission>, SecurityError> {
        Ok(self.grants.get(role).cloned().unwrap_or_default())
    }
}
```

## ServiceContext Integration

`ServiceContext` is currently a flat struct (verified in `crates/service-sdk/src/context/mod.rs`). The Security SDK ships **only the additive field** — no nesting refactor, no `TelemetryContext`.

### Field addition (diff)

```rust
// crates/service-sdk/src/context/mod.rs
use std::sync::Arc;
use ego_security_sdk::context::SecurityContext;   // NEW import

#[derive(Debug, Clone)]
pub struct ServiceContext {
    pub tenant_id: Option<String>,
    pub correlation_id: Option<String>,
    pub trace_id: Option<String>,
    pub deadline: Option<SystemTime>,
    pub timeout: Option<Duration>,
    pub additional_context: HashMap<String, String>,
    pub allow_cross_tenant: bool,
    pub cancellation_token: Option<CancellationToken>,
    pub security: Option<Arc<SecurityContext>>,   // NEW — additive, optional
}
```

`SecurityContext` derives `Debug + Clone`, so `ServiceContext`'s existing `#[derive(Debug, Clone)]` still holds. `Arc` makes propagation cheap and clone-safe across the graph.

### Constructor + builder

```rust
impl ServiceContext {
    pub fn new() -> Self {
        Self {
            // ...existing fields...
            security: None,            // NEW — default None keeps all callers compiling
        }
    }

    /// Attaches a security context (authenticated identity + scope).
    pub fn with_security(mut self, security: Arc<SecurityContext>) -> Self {
        self.security = Some(security);
        self
    }

    /// Returns the attached security context, if any.
    pub fn security(&self) -> Option<&Arc<SecurityContext>> {
        self.security.as_ref()
    }
}
```

`ServiceContext::new()` is the only construction site that sets every field literally, so adding `security: None` there is the single required edit; every `with_*` builder call still works. `Default` delegates to `new()`, unchanged.

### RuntimeBuilder propagation

`ServiceContext` is cloned (cheaply, `Arc`) into each `ctx.scope(...)` re-entry the proxy already performs. Because `security` is an ordinary field on the cloned struct, it travels unchanged through `Runtime::resolve` → proxy → `enforce_tenant` → `ctx.scope` → impl, and onward to nested service-to-service calls (which inherit the scoped `ServiceContext`). **No new propagation code is required** beyond carrying the field; the existing clone-on-scope path moves it across Services, Command Handlers, Persistent Entities, Projections, and the Scheduler. The verification target is: a `SecurityContext` set on an outer `ServiceContext` is observable, identical, on an inner call.

**INV-007 — Propagation invariant**: `ServiceContext::clone()` and any `scope()`/builder derivation MUST preserve the `security` field unchanged. The `with_security` builder method is the only sanctioned way to attach a `SecurityContext`; no component may silently drop it. This invariant is verified by the integration tests in `context_propagation.rs`.

### Caller extraction

```rust
if let Some(sec) = ctx.security() {
    // principal is guaranteed non-optional when SecurityContext exists
    let principal = sec.principal();
    // use principal.subject, principal.roles, ...
}
```

Unauthenticated/internal paths carry `None` in `ctx.security` and the caller handles it explicitly. When `security` is `Some`, `principal()` returns `&Principal` directly — no further `Option` unwrap needed.

## Declarative Authorization Integration Point

The Security SDK defines (and tests) the **stable callable path** the future `#[authorize(...)]` macro will generate a call into. The macro itself is out of scope. The function lives in `security-sdk` so any consumer (starting with `service-sdk`) can call it.

Location: `crates/security-sdk/src/authorization/mod.rs`.

```rust
/// Resolves the security context attached to a `ServiceContext`-like carrier,
/// builds an `AccessRequest`, calls the authorization provider, and maps a
/// `Deny` decision to a `SecurityError`.
///
/// This is the stable seam a future `#[authorize("orders:read")]` macro targets:
/// the macro only emits a call to this function. To keep `security-sdk` free of
/// any ego dependency, the caller passes the already-resolved
/// `Option<&SecurityContext>` (extracted from `ctx.security`), not `ServiceContext`.
///
/// # Errors
/// - [`SecurityError::MissingContext`] if `security` is `None` or has no principal.
/// - [`SecurityError::AuthorizationDenied`] if the decision is `Deny`.
/// - Propagates any provider error.
pub async fn authorize_in_context(
    security: Option<&SecurityContext>,
    resource: Resource,
    action: Action,
    provider: &dyn AuthorizationProvider,
) -> Result<(), SecurityError> {
    let sec = security.ok_or(SecurityError::MissingContext)?;
    // principal is non-optional: SecurityContext guarantees a Principal exists
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

**Signature contract (stable):**

| Element | Value |
|---|---|
| Inputs | resolved `Option<&SecurityContext>` (from `ctx.security`), `Resource`, `Action`, `&dyn AuthorizationProvider` |
| Output | `Result<(), SecurityError>` |
| Deny mapping | `Deny { reason }` → `SecurityError::AuthorizationDenied { reason }` |
| Missing identity | `None` security → `SecurityError::MissingContext`; no per-principal `Option` check needed (invariant) |

A future macro on a `#[service]` operation expands to: extract `ctx.security()`, call `authorize_in_context(...)` with the macro-literal resource/action, `?`-propagate. `service-sdk` can optionally expose a thin wrapper that accepts `&ServiceContext` and forwards `ctx.security()`, keeping `security-sdk` ego-free.

`AccessRequest::from_permission(descriptor)` is the stable parsing entry point that the future `#[authorize("resource:action")]` macro calls when converting the macro literal into a typed `AccessRequest`. The format is `"<resource>:<action>"` — exactly one colon, neither segment empty. Parse failures return `SecurityError::InvalidAccessRequest`.

## Error Design

Single `thiserror` enum (full definition in [Core Types → SecurityError](#securityerror)). Design rules enforced:

| Rule | How |
|---|---|
| No vendor type in public surface | No `#[from] jsonwebtoken::Error`; JWT/store errors mapped to `ProviderError(String)` / `AuthenticationFailed(String)` at the provider boundary. |
| Deny is not an error at the provider | `AuthorizationProvider::authorize` returns `Ok(Deny { .. })`; only `authorize_in_context` converts a `Deny` into `AuthorizationDenied`. |
| Distinguish reject vs. malformed | `AuthenticationFailed` (ran, rejected) vs. `InvalidCredential` (wrong scheme / malformed). |
| Format validation is typed | `InvalidSubjectId` for `SubjectId::new` failures (empty string rejected). `InvalidAccessRequest` for `AccessRequest::from_permission` parse failures (bad `"resource:action"` format). Each domain has its own variant — they are never conflated. |
| Missing identity is explicit | `MissingContext` where a principal was required but absent. |

`SecurityError: Send + Sync + 'static` (all variants wrap `String`), so it composes with async traits and `ServiceError` mapping in `service-sdk` later.

## Testing Strategy

**Strict TDD — tests first, then implementation. Gate: `cargo test --workspace`.** `#![deny(missing_docs)]` means every public item ships with docs from the first commit.

### Unit tests (per module, in `#[cfg(test)]` blocks)

| Module | Covers |
|---|---|
| `principal/subject_id.rs` | `new` accepts `"user:123"`, `"service:billing"`, any non-empty string; rejects empty string → `InvalidSubjectId`. |
| `principal/principal.rs` | builder adds roles/claims/attributes; `has_role`. |
| `credential` | each variant constructs and matches. |
| `authorization/decision.rs` | `is_allowed` for Allow vs Deny. |
| `policy` | `Permission` equality/hash; `"*"` action semantics consumed in rbac tests. |
| `context` | `new(principal)`, `with_scope`, `principal()` returns `&Principal` directly. |
| `error` | `Display` strings; assert no variant carries a non-String payload. |

### Integration tests (`tests/`)

| File | Scenario |
|---|---|
| `basic_auth.rs` | injected verifier returns principal → Ok; returns None → `AuthenticationFailed`; non-Basic credential → `InvalidCredential`; verifier backend error → `ProviderError`. |
| `rbac.rs` | principal whose role grants `orders:read` → `Allow`; principal without the grant → `Deny { reason }`; wildcard `"*"` action grants; unknown role → empty perms → contributes nothing. |
| `context_propagation.rs` | a `SecurityContext` with a principal set on an outer carrier is observed identically on an inner (nested) call; assert no thread-local/task-local/global is read (the value arrives only via the explicitly-passed context). |
| `declarative_authz.rs` | `authorize_in_context` with Allow → `Ok(())`; with Deny → `AuthorizationDenied { reason }`; `None` security → `MissingContext`; authenticated-but-unauthorized → `AuthorizationDenied`. |
| `error_mapping.rs` | force a provider/store failure and assert the surfaced error is a neutral `SecurityError` whose `Debug`/`Display` does not contain `jsonwebtoken` type names. |

### Mock strategy (`mockall`)

Because all three contracts are object-safe async traits, annotate each with `#[cfg_attr(test, mockall::automock)]` (or a dedicated `mock!` block) to generate `MockAuthenticationProvider`, `MockAuthorizationProvider`, `MockRoleStore`. Tests inject the mock as `Arc::new(mock) as Arc<dyn _>`:

- `MockRoleStore` lets `rbac.rs` drive `permissions_for_role` return values without `InMemoryRoleStore`, proving `RbacProvider` depends only on the trait.
- `MockAuthorizationProvider` lets `declarative_authz.rs` exercise Allow/Deny/provider-error mapping without `RbacProvider`.
- `MockAuthenticationProvider` lets `context_propagation.rs` produce a principal deterministically.

`mockall` is already a workspace/dev dependency (version `0.12` in `service-sdk`); pin the same in `security-sdk` dev-deps.

## Cargo.toml Changes

### New crate `crates/security-sdk/Cargo.toml`

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

`lib.rs` starts with `#![deny(missing_docs)]`.

### Workspace root `Cargo.toml`

```toml
[workspace]
members = [
    # ...existing members...
    "crates/service-sdk",
    "crates/service-sdk-macros",
    "crates/security-sdk",          # NEW
]

[workspace.dependencies]
mockall = "0.14.0"
async-trait = "0.1"
```

(The repo currently declares most deps inline per-crate rather than via `workspace = true`; `security-sdk` follows that inline style for self-containment.)

### `crates/service-sdk/Cargo.toml`

```toml
[dependencies]
async-trait = "0.1"
tokio = { version = "1.0", features = ["full"] }
tokio-util = { version = "0.7" }
semver = "1"
thiserror = "1.0"
kitlogger = { git = "https://github.com/pablogore/kitlogger.git", branch = "develop" }
ego-security-sdk = { path = "../security-sdk" }   # NEW
```

## Design Constraints Honored (checklist)

- [x] All provider traits object-safe: `#[async_trait]`, no generic methods, no `Self`-returning methods, invoked as `Arc<dyn _>`.
- [x] `SecurityError` exposes no external vendor error type — provider failures are mapped to `ProviderError`/`AuthenticationFailed` strings.
- [x] `SubjectId` is an opaque newtype over `String` with non-empty validation only — no format enforced at the core level; provider interprets the value.
- [x] `SecurityContext` has `principal: Principal` (non-optional); invariant: if `SecurityContext` exists, a `Principal` is guaranteed.
- [x] `JwtAuthenticationProvider` and `LocalKeyStore` deferred to CORE-009A; no `jsonwebtoken` dependency in the Security SDK.
- [x] `InMemoryRoleStore` lives in `policy/` (alongside `RoleStore` and `Permission`); `providers/rbac/` contains only `RbacProvider` with no backend coupling. `InMemoryRoleStore` implements `RoleStore` exactly, so it is swappable with a `mockall` mock.
- [x] Tokio is a `[dev-dependencies]` only — production code uses `std::sync` + `async_trait`; no tokio runtime dependency on consuming crates.
- [x] `ServiceContext` gains only `security: Option<Arc<SecurityContext>>` — additive, no nesting refactor, all existing callers keep compiling with `None`.
- [x] Propagation is explicit only — value travels via the passed `ServiceContext`; no thread/task-local for security identity.