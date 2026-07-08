# Design: CORE-022 — TestKit

## Technical Approach

A new library crate `crates/testkit` (package `ego-testkit`) that **composes and
re-exports** the existing production contracts and hands test authors ready-made,
contract-identical building blocks. TestKit adds **no parallel type** for any
production contract: every value it produces is the real production type
(`ServiceContext`, `SecurityContext`, `Principal`, `Arc<dyn AuthorizationProvider>`,
`Arc<KITLogger>`, `ConfigValue<C>`) or a thin ergonomic wrapper around a real
`RuntimeBuilder`/`Runtime`. Where TestKit needs behavior (a scriptable authorizer,
a capturable logger) it implements the **same public trait** production consumes,
so a test exercises real dispatch and real validation, not a look-alike.

TestKit does **not** redesign or fork Runtime, Service SDK, Security,
Configuration, or Logger (proposal Non Goals). It has three jobs only:

1. **Construct** real contract values with sensible defaults (identity, security,
   context, config, logger).
2. **Compose** them into a pre-wired `Runtime` + `ServiceContext` via the real
   `RuntimeBuilder` (fixtures).
3. **Assert** outcomes against the real production error/decision types.

### Grounding findings (drive several decisions below)

Verified against `develop` source, not assumed:

1. **`KITLogger::with_exporter_and_format` takes a concrete
   `Arc<console_exporter::ConsoleExporterImpl>`, not `Arc<dyn Exporter>`**
   (confirmed by `service-sdk/Cargo.toml` dev-dep comment and
   `service-sdk/src/runtime/logger.rs` tests). kitlogger exposes **no**
   pluggable structured-exporter trait at that entry point. The only injection
   seam is `ConsoleExporterImpl::set_writers(Box<dyn Write>, Box<dyn Write>)`.
   Therefore the capturable logger must redirect the real formatter's output
   into an in-memory buffer and parse it back — it cannot register a native
   record-capturing exporter (see AD-6).
2. **`AllowAllAuthorizationProvider` is gated behind `ego-security-sdk`'s
   `dev-providers` / `test-helpers` feature.** `reference-app/src/lib.rs`
   documents that enabling it from a permanent workspace member's
   `[dependencies]` unifies the feature across every workspace member's
   `ego-security-sdk` link. TestKit avoids that leak by shipping its own
   `ScriptedAuthorizationProvider` (see AD-3). `DenyAllAuthorizationProvider`
   is **not** feature-gated and is reused directly.
3. **`SecurityContext` always carries a `Principal`** — there is no
   "unauthenticated `SecurityContext`". Production models *no authenticated
   principal* as `ServiceContext.security == None`
   (`require_security()` → `SecurityError::CapabilityNotEnabled`). TestKit's
   unauthenticated helper therefore operates at the `ServiceContext` level
   (see AD-4).
4. **There is no public `Runtime::resolve` execution engine and no
   `RuntimeBuilder` service-registration API yet.** Production "execution" of a
   service today = build a `Runtime` via `RuntimeBuilder`, then invoke the
   service's real async trait method with a `ServiceContext`, returning the
   service's own `Result<T, ServiceError>`. TestKit must **not** invent an
   execution engine (see AD-1).
5. **The config contract a service reads is
   `RuntimeInner::resolve_config::<C>() -> Result<ConfigValue<C>, RuntimeError>`;
   an unset key returns `Err(DependencyNotFound)`, never a panic.** Registration
   is `RuntimeBuilder::with_config::<C>(Arc<C>)`. `ConfigurationProvider::from_value`
   covers the JSON-subtree view. TestKit reuses both (see AD-5).

## Requirement → Component Map

| Spec requirement | TestKit component | Production contract reused |
|------------------|-------------------|----------------------------|
| Consistent Service Execution | `ServiceTestFixture` + `ServiceContext` + `fixture.service::<S>()` | `RuntimeBuilder`/`Runtime`, `Injectable::build`, service trait `Result<T, ServiceError>` |
| Testing `ServiceContext` | `TestContextBuilder`, `test_context()` | `ego_service_sdk::ServiceContext` (real, per-test instance) |
| Testing `AuthorizationProvider` | `ScriptedAuthorizationProvider` (+ reused `DenyAllAuthorizationProvider`) | `ego_security_sdk::authorization::AuthorizationProvider` (async trait) |
| `SecurityContext` Helpers | `security::authenticated*`, `TestContextBuilder::unauthenticated` | `ego_security_sdk::context::SecurityContext` |
| Identity Builders | `PrincipalBuilder`, `principal()` | `ego_security_sdk::principal::{Principal, SubjectId, Role, PrincipalKind}` |
| Test Configuration | `TestConfig`, `FixtureBuilder::config`, `fixture.service::<S>()` | `RuntimeBuilder::with_config` → `Injectable::build` → `resolve_config`, `ConfigurationProvider` |
| Capturable Logger | `CapturingLogger`, `CapturedRecord` | real `Arc<KITLogger>` via `with_exporter_and_format` |
| Reusable Fixtures | `ServiceTestFixture`, `FixtureBuilder` | all of the above, composed via `RuntimeBuilder` |
| Assertion Helpers | `assert_authorized`/`assert_denied`, `assert_service_error!` | `authorize_in_context`, `SecurityError`, `ServiceError` |

## Architecture Decisions

### AD-1: No execution engine — the fixture wires a real `Runtime`, the test calls the real trait method

| Option | Tradeoff | Decision |
|--------|----------|----------|
| TestKit builds a new "test executor" that dispatches services | Reinvents/forks Runtime dispatch that does not even exist publicly yet; guaranteed divergence | rejected |
| Fixture builds a real `Runtime` via `RuntimeBuilder`; test invokes the service's own async trait method with a TestKit `ServiceContext` | Zero new dispatch; success/error types are the service's own `Result<T, ServiceError>` by construction | **chosen** |

Rationale: The spec's "returned success and error types are identical to what a
production caller receives" is satisfied *automatically* when the test calls the
real trait method — there is nothing to keep in sync. "MUST NOT require a test
author to hand-wire Runtime internals" is met by `ServiceTestFixture`, which
performs the `RuntimeBuilder` wiring once. Independence ("two tests execute
independently") is structural: each fixture owns its own `Runtime` and issues
fresh `ServiceContext` values — TestKit holds **no** statics, thread-locals, or
globals. When a public `Runtime::resolve` lands later, the fixture exposes
`runtime()` so tests can resolve proxies through it unchanged.

### AD-2: `ServiceContext` is produced by a builder over the real type, one instance per test

| Option | Tradeoff | Decision |
|--------|----------|----------|
| New `TestServiceContext` type | Parallel type — violates the same-contract constraint | rejected |
| `TestContextBuilder` that returns `ego_service_sdk::ServiceContext` | Real type; each `build()` is a fresh value, no shared state | **chosen** |

`ServiceContext` is already a concrete, cheaply-cloned value type constructed at a
request boundary (`ServiceContext::new().with_*`). The builder is sugar over its
existing `with_security`/`with_logger`/`with_tenant_id`/`with_correlation_id`
methods. Isolation ("state set in one context MUST NOT leak into the other") is
inherent: distinct `ServiceContext` values share nothing except `Arc`-counted
immutable inners.

> The `ServiceContext` carries *request-scoped* state (security, logger, tenant).
> The service instance that reads it is obtained separately, through the real
> DI construction path — see AD-9. AD-2 covers the context; AD-9 covers the
> service.

### AD-3: Ship `ScriptedAuthorizationProvider` rather than depend on `test-helpers`

| Option | Tradeoff | Decision |
|--------|----------|----------|
| Depend on `ego-security-sdk/test-helpers` and re-export `AllowAllAuthorizationProvider` | Feature unification leaks allow-all into every workspace member's `ego-security-sdk` on `--workspace` builds (documented in reference-app) | rejected as the default |
| Implement `ScriptedAuthorizationProvider` in TestKit against the real async trait | No feature leak; supports per-action allow **and** deny; denial flows through real `authorize_in_context` → `SecurityError::AuthorizationDenied` | **chosen** |

`ScriptedAuthorizationProvider` implements
`ego_security_sdk::authorization::AuthorizationProvider` — the same async trait a
policy engine implements — so it is invoked via real dispatch and its `Deny`
maps, through the production `authorize_in_context` seam, to the same
`SecurityError::AuthorizationDenied` production uses. It matches on
`(resource.kind, action)` with a configurable default, covering both spec
scenarios (allow a specific action / deny a specific action) deterministically,
with no policy engine or external service. `DenyAllAuthorizationProvider`
(ungated) is re-exported for the coarse case; `AllowAllAuthorizationProvider`
re-export is offered **only** behind an opt-in `ego-testkit` passthrough feature
so the leak is a deliberate choice, never the default.

### AD-4: The unauthenticated helper operates on `ServiceContext`, not `SecurityContext`

`SecurityContext`'s invariant is that a `Principal` is always present — there is
no unauthenticated `SecurityContext`. Production represents "no authenticated
principal" as `ServiceContext.security == None`, which makes `require_security()`
return `SecurityError::CapabilityNotEnabled`. TestKit therefore:

- `security::authenticated(principal)` / `authenticated_with_claims(principal, claims)`
  → a real `SecurityContext` (via `SecurityContext::empty` / `::new`).
- `TestContextBuilder::unauthenticated()` (and `FixtureBuilder::unauthenticated()`)
  → a `ServiceContext` whose `security` field is left `None`.

This reproduces exactly how production code observes the unauthenticated case
("the same way production code represents it"), rather than fabricating a
sentinel `SecurityContext`.

**Scopes.** The spec's "SecurityContext Helpers" requirement mentions scopes, but
`SecurityContext` has **no** `scopes` field — it carries only `principal` and
`claims`. Per CORE-021 (AD-7), scopes are request-scoped auth assertions that live
in `Claims.custom["scopes"]` as a JSON string array, not on the principal. TestKit
follows that convention: `authenticated_with_claims(principal, claims)` is how a
test supplies scopes (by putting a `"scopes"` entry in `Claims.custom`); TestKit
adds no `scopes` accessor of its own, because none exists on the production
contract to mirror. A `.scopes([..])` convenience on the claims side is a possible
future ergonomic helper, but it would still write into `Claims.custom["scopes"]` —
never a new field.

### AD-5: Test configuration reuses `with_config`/`resolve_config` and `ConfigurationProvider`

| Option | Tradeoff | Decision |
|--------|----------|----------|
| New in-memory config type with its own getter API | Parallel config contract; "unset key" semantics could drift from production | rejected |
| `TestConfig` registers typed values through `RuntimeBuilder::with_config`; JSON-subtree via `ConfigurationProvider::from_value` | Service reads through the real `resolve_config::<C>()`; unset = `Err(DependencyNotFound)`, never a panic — identical to production | **chosen** |

The service under test reads configuration through the production contract
(`resolve_config::<C>()` on the runtime, yielding `ConfigValue<C>`) — and it does
so at **construction** time, not at call time: the `#[service]` macro's generated
`Injectable::build(rt)` resolves each `ConfigValue<C>` field via
`rt.resolve_config::<C>()`. So "provided value is observed by the service" is only
meaningful once the service is constructed through that path — which AD-9 makes
the fixture's responsibility. `TestConfig::with_value::<C>` is a thin collector the
fixture drains into `RuntimeBuilder::with_config::<C>` calls; the constructed
service then observes it. "Unset key behaves like production (default or explicit
not-found, never a panic)" holds because the read path is the unmodified
production path (`resolve_config` → `Err(DependencyNotFound)` for an unset type).

**Two distinct config contracts — do not conflate them:**

| Path | Written by | Read by | Observable via `resolve_config::<C>()`? |
|------|-----------|---------|------------------------------------------|
| Typed DI value | `TestConfig::with_value::<C>` → `RuntimeBuilder::with_config` | service field `ConfigValue<C>` (AD-9) | **Yes** |
| JSON subtree view | `TestConfig::set(key, value)` | `ConfigurationProvider::from_value(..).<view>()` (host-boundary, e.g. logging) | **No** |

`TestConfig::set(..)` populates only the `serde_json::Value` tree that
`ConfigurationProvider` reads at the host boundary. It is **not** registered on
the runtime and is therefore **not** observable through `resolve_config::<C>()` —
a service's typed config fields will not see values set via `.set()`. Use
`.with_value::<C>` for anything a service resolves through DI; use `.set()` only
for the JSON-subtree/host-boundary contract (e.g. logging settings).

**Open item found during Phase 6 review, to resolve at the start of Phase 8:**
`TestConfig.typed` (`HashMap<TypeId, Arc<dyn Any + Send + Sync>>`) is type-erased
at rest, matching `RuntimeInner`'s `DependencyTable` *container shape* — but
container-shape match is not itself a draining path. `RuntimeBuilder::with_config`
is generic (`fn with_config<C: Send + Sync + 'static>(self, value: Arc<C>)`) and
needs a concrete `C` at the call site; there is no way to iterate a type-erased
map and call a generic method per entry without already knowing `C` per key, and
`DependencyTable`/`with_registrations` are `pub(super)` in `service-sdk`, not
reachable from `testkit`. Phase 8 must capture a per-value closure at
`with_value::<C>()` call time (e.g. `Box<dyn FnOnce(RuntimeBuilder) -> RuntimeBuilder>`)
instead of relying on the type-erased map to drain itself — decide this before
starting `fixtures.rs`, not mid-implementation.

### AD-6: Capturable logger = real `KITLogger` + writer-side capture, not a fake logger

| Option | Tradeoff | Decision |
|--------|----------|----------|
| A `TestLogger` implementing a logging trait | kitlogger's construction entry (`with_exporter_and_format`) takes the **concrete** `ConsoleExporterImpl`; there is no `dyn Exporter` seam to implement, so a fake would be a parallel type | rejected |
| Build a real `Arc<KITLogger>` via `with_exporter_and_format`, redirect its `ConsoleExporterImpl` writers into an in-memory buffer, parse the `LogFormat::Json` output into `CapturedRecord` | Logger handed to the service is the real `KITLogger`; capture is purely on the serialized output side | **chosen** |

Rationale: the service receives an `Arc<KITLogger>` — byte-for-byte the
production logging contract (`ServiceContext::with_logger(Arc<KITLogger>)`), so
it logs through the real pipeline and real formatter. `CapturingLogger` owns a
`ConsoleExporterImpl` whose writers point at a shared `Arc<Mutex<Vec<u8>>>`;
`records()` parses each JSON line the real `LogFormat::Json` formatter emitted
back into a structured `CapturedRecord { level, message, fields }`. Because
`level`/`message`/`fields` are recovered from what the real formatter wrote, the
captured record has the same level, message, and structured fields the service
logged. No leakage between instances is structural: each `CapturingLogger` owns
its own exporter and buffer. The one honest ceiling — capture depends on the
JSON formatter's field names — is called out in Risks and Open Questions; it is
forced by kitlogger's concrete-exporter API, not a TestKit shortcut.

**Supported logging entry points.** `KITLogger` exposes more than one way to
emit a log. Capture works for **any** path that routes through the exporter and
JSON formatter this logger is built with — that is the entire supported surface,
because capture is writer-side:

- The structured path (the record-carrying entry point, `log_record`-style):
  captured with **full** `level`, `message`, and `fields`.
- The simple path `KITLogger::log(Severity, &str)`: also captured (it still
  serializes through the same formatter), but it carries **no structured
  fields** — so the resulting `CapturedRecord.fields` is empty. This is not a
  TestKit limitation; the field-less record is exactly what the service emitted.

A hypothetical logging call that bypasses this logger's exporter/formatter
entirely (e.g. writing to a global `tracing` subscriber instead of the injected
`KITLogger`) produces **no** captured records — `CapturingLogger` observes only
what flows through the `Arc<KITLogger>` handed to the service. The exact
field-carrying entry point name and the JSON shape each path emits are confirmed
in the Open Questions against kitlogger source at apply time.

### AD-7: Identity builder uses `SubjectId::new` and defaults to a valid, non-empty subject

`PrincipalBuilder` builds a real `Principal` via `Principal::new(kind, SubjectId::new(..)?)`
plus the existing `with_tenant_id`/`with_role`/`with_attribute` methods. The
default subject is a fixed non-empty value (`"test:subject"`), so a no-override
`build()` satisfies every production invariant `SubjectId`/`Principal` enforce
("Default identity satisfies production invariants"). Overriding one field
(`.role("admin")`) leaves all others at their default via builder semantics
("Overriding a single field leaves others at default"). The builder returns the
real `Principal`; there is no `TestPrincipal`.

### AD-8: Assertion helpers wrap the real seams, not reimplemented comparison logic

- `assert_authorized` / `assert_denied` call the real
  `ego_security_sdk::authorization::authorize_in_context` and assert on its
  `Result<(), SecurityError>` — the same seam production authorization flows
  through. `assert_denied` asserts specifically on
  `SecurityError::AuthorizationDenied`.
- `assert_service_error!(result, ServiceError::NotFound { .. })` is a
  `matches!`-based macro: it matches the **variant** and ignores the message
  string, satisfying "passes only when the actual result matches that variant,
  regardless of error message text". Using a pattern (not a discriminant helper)
  keeps it exhaustive and readable at the call site.

### AD-9: The fixture constructs the service through the real `Injectable::build` DI path

The spec's "Consistent Service Execution" and "Test Configuration" requirements
both hinge on the service instance being wired to the runtime — a bare
`ServiceContext` cannot carry DI-resolved config. Production obtains a service
instance through the `#[service]` macro's generated
`Injectable::build(rt: &RuntimeInner)`, which resolves each `ConfigValue<C>` /
`AdapterRef<A>` / `ProjectionRef<P>` field via `rt.resolve_config::<C>()` etc.

| Option | Tradeoff | Decision |
|--------|----------|----------|
| Test constructs the service by hand (`MyService { .. }`) | Bypasses DI — the instance never resolves runtime-registered config; "provided value is observed by the service" cannot be shown | rejected |
| Fixture exposes `service::<S: Injectable>() -> Result<S, RuntimeError>` calling `S::build(self.runtime().inner())` | Uses the **exact** production construction path; the returned instance resolves config through `resolve_config` just as production does | **chosen** |

`ServiceTestFixture::service::<S>()` calls the real
`Injectable::build(self.runtime.inner())`. The returned `S` is the real service
struct; the test then invokes `S`'s own async trait methods with
`fixture.context()`, returning the service's own `Result<T, ServiceError>`. This
is what closes the loop the review flagged: the `svc` in the Data Flow comes from
`fixture.service::<S>()`, and a value registered via `TestConfig::with_value::<C>`
is observed by the service *because* it was resolved through `resolve_config::<C>()`
during `build`. `S: Injectable` is the only bound required; no macro is needed for
the fixture itself (a hand-written `Injectable` impl works, which is how TestKit's
own tests exercise the path without depending on the macro crate).

### AD-10: Runtime security providers are all-or-nothing; the test authn is a pairing stub, not dispatched

`RuntimeBuilder::with_security(authn, authz)` takes **both** providers together,
and `build()` keeps the pair only when both are present
(`(Some, Some) => Some(..)`, else `None`). There is no "authz-only" registration.
The authz provider **is** load-bearing: a service annotated with `#[authorize(..)]`
resolves it from the runtime via `RuntimeInner::authorization_provider()` and runs
it through the real `authorize_in_context` seam. To register that authz provider,
the pairing invariant forces an authn provider to be supplied alongside it.

Because AD-1 does not dispatch authentication (the test calls the service's trait
method directly rather than driving a credential through Runtime), the authn
provider is never invoked. The fixture therefore supplies a **minimal internal
stub** `AuthenticationProvider` whose only job is to satisfy the pairing invariant
so the authz provider is retained. It is a real `AuthenticationProvider` impl (not
a parallel type), private to TestKit, and returns a fixed authenticated context if
ever called. It is intentionally *not* part of the public surface: exposing it
would imply an authentication-driven execution model TestKit does not offer.

> If a later change makes `RuntimeBuilder` accept an authz-only registration, the
> stub disappears and `FixtureBuilder` drops the authn dependency — no other part
> of this design changes. Until then the stub is the minimum needed to keep the
> `#[authorize]` seam working for services under test.

## Data Flow

```
Test author
  │
  ├─ PrincipalBuilder::new().role("admin").build()      ─► real Principal
  │        └─ SubjectId::new("test:subject")            (default, valid)
  │
  ├─ security::authenticated(principal)                 ─► real SecurityContext
  │
  ├─ ScriptedAuthorizationProvider::allow_all()          ─► Arc<dyn AuthorizationProvider>
  │     .deny("orders", "delete", "no role")
  │
  ├─ CapturingLogger::new()                              ─► real Arc<KITLogger>
  │     └─ ConsoleExporterImpl::set_writers(buffer,..)      (LogFormat::Json → in-memory buffer)
  │
  └─ ServiceTestFixture::builder()
         .principal(principal)          (or .unauthenticated())
         .authorization(authz)
         .config(TestConfig::new().set("k", v))
         .build()
             │  RuntimeBuilder::new()
             │      .with_security(test_authn, authz)
             │      .with_logger(capturing.logger())
             │      .with_config::<C>(Arc<C>)  (per TestConfig entry)
             │      .build()                    ─► real Runtime
             ▼
        ServiceTestFixture { runtime, context: ServiceContext(security+logger attached), logger }
             │
   test:    let svc = fixture.service::<MyServiceImpl>()?; // Injectable::build(runtime.inner())
            │        └─ resolves ConfigValue<C> via rt.resolve_config::<C>()  (AD-9)
            let ctx = fixture.context();                 // real ServiceContext, fresh per call
            let out: Result<T, ServiceError> = svc.handle(&ctx, req).await;   // REAL trait method
            assert_service_error!(out, ServiceError::NotFound { .. });
            assert_denied(&*authz, &sec, res, act).await; // → SecurityError::AuthorizationDenied
            let records = fixture.captured_records();      // Vec<CapturedRecord>
```

Every arrow crosses only production contracts. Two fixtures share nothing —
independence and isolation are guaranteed by construction, not by reset logic.

## Crate Layout / File Changes

| File | Action | Description |
|------|--------|-------------|
| `Cargo.toml` (workspace) | Modify | Add member `crates/testkit` |
| `crates/testkit/Cargo.toml` | Create | Package `ego-testkit`; deps below; optional `dev-providers` passthrough feature |
| `crates/testkit/src/lib.rs` | Create | `#![deny(missing_docs)]`, crate-level same-contract note, `pub use` re-exports of production types + TestKit surface |
| `crates/testkit/src/identity.rs` | Create | `PrincipalBuilder`, `principal()` (AD-7) |
| `crates/testkit/src/security.rs` | Create | `authenticated`, `authenticated_with_claims` (AD-4) |
| `crates/testkit/src/context.rs` | Create | `TestContextBuilder`, `test_context()`, `unauthenticated()` (AD-2, AD-4) |
| `crates/testkit/src/authz.rs` | Create | `ScriptedAuthorizationProvider`; re-export `DenyAllAuthorizationProvider` (AD-3) |
| `crates/testkit/src/config.rs` | Create | `TestConfig` (AD-5) |
| `crates/testkit/src/logger.rs` | Create | `CapturingLogger`, `CapturedRecord` (AD-6) |
| `crates/testkit/src/fixtures.rs` | Create | `ServiceTestFixture` (incl. `service::<S: Injectable>()`, AD-9), `FixtureBuilder`, private pairing authn stub (AD-10) |
| `crates/testkit/src/assertions.rs` | Create | `assert_authorized`/`assert_denied`, `assert_service_error!` (AD-8) |

TestKit touches **no** existing crate source (additive only). It does not modify
Runtime, Service SDK, Security, Configuration, or Logger.

### `crates/testkit/Cargo.toml` (dependency shape)

```toml
[package]
name = "ego-testkit"
version = "0.1.0"
edition = "2021"

[features]
# Opt-in ONLY: re-exports ego-security-sdk's AllowAllAuthorizationProvider.
# Off by default so the dev-providers feature is never silently unified across
# the workspace (see AD-3). ScriptedAuthorizationProvider::allow_all() is the
# default allow path and needs no feature.
dev-providers = ["ego-security-sdk/test-helpers"]

[dependencies]
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
ego-domain = { path = "../domain" }
ego-security-sdk = { path = "../security-sdk" }
ego-service-sdk = { path = "../service-sdk" }
kitlogger = { git = "https://github.com/pablogore/kitlogger.git", branch = "develop" }
kitlogger-formatter = { git = "https://github.com/pablogore/kitlogger.git", branch = "develop" }
console-exporter = { git = "https://github.com/pablogore/kitlogger.git", branch = "develop" }
kitlogger-log-domain = { git = "https://github.com/pablogore/kitlogger.git", branch = "develop" }
```

## Interfaces / Contracts

```rust
// ─── identity.rs ─────────────────────────────────────────────────────────────
use ego_security_sdk::principal::{Principal, PrincipalKind, Role, SubjectId};

/// Builds a real `Principal` with valid defaults; override only what a test needs.
pub struct PrincipalBuilder { /* kind, subject, tenant, roles, attributes */ }
impl PrincipalBuilder {
    pub fn new() -> Self;                                   // User, subject "test:subject"
    pub fn kind(self, kind: PrincipalKind) -> Self;
    pub fn subject(self, subject: impl Into<String>) -> Self;
    pub fn tenant(self, tenant: impl Into<String>) -> Self;
    pub fn role(self, role: impl Into<String>) -> Self;     // wraps Role(..)
    pub fn attribute(self, key: impl Into<String>, value: impl Into<String>) -> Self;
    /// Panics only on an explicitly-set empty subject; the default is always valid.
    pub fn build(self) -> Principal;                        // Principal::new(kind, SubjectId::new(..))
}
/// Convenience: `PrincipalBuilder::new().build()`.
pub fn principal() -> Principal;

// ─── security.rs ─────────────────────────────────────────────────────────────
use ego_domain::auth::Claims;
use ego_security_sdk::context::SecurityContext;

/// Real `SecurityContext` for `principal`, empty claims. Indistinguishable to
/// consuming code from what a real AuthenticationProvider produces.
pub fn authenticated(principal: Principal) -> SecurityContext;         // SecurityContext::empty
pub fn authenticated_with_claims(principal: Principal, claims: Claims) -> SecurityContext; // ::new
// Unauthenticated is represented at the ServiceContext level — see context.rs (AD-4).

// ─── context.rs ──────────────────────────────────────────────────────────────
use std::sync::Arc;
use ego_service_sdk::ServiceContext;
use kitlogger::KITLogger;

/// Builds a real `ego_service_sdk::ServiceContext`; each `build()` is independent.
pub struct TestContextBuilder { /* security, logger, tenant, correlation, trace */ }
impl TestContextBuilder {
    pub fn new() -> Self;
    pub fn security(self, sec: SecurityContext) -> Self;    // -> with_security(Arc::new(sec))
    pub fn unauthenticated(self) -> Self;                   // leaves security = None (AD-4)
    pub fn logger(self, logger: Arc<KITLogger>) -> Self;
    pub fn tenant(self, tenant: impl Into<String>) -> Self;
    pub fn correlation(self, id: impl Into<String>) -> Self;
    pub fn build(self) -> ServiceContext;                   // real ServiceContext, fresh state
}
/// Convenience: authenticated context for `principal()` with no logger.
pub fn test_context() -> ServiceContext;

// ─── authz.rs ────────────────────────────────────────────────────────────────
use async_trait::async_trait;
use ego_security_sdk::authorization::{
    AccessRequest, AuthorizationDecision, AuthorizationProvider,
};
use ego_security_sdk::{context::SecurityContext, error::SecurityError, principal::Principal};
pub use ego_security_sdk::DenyAllAuthorizationProvider;   // ungated, reused as-is
#[cfg(feature = "dev-providers")]
pub use ego_security_sdk::AllowAllAuthorizationProvider;  // opt-in only (AD-3)

/// Deterministic per-action authorizer implementing the REAL async trait.
/// Denials flow through `authorize_in_context` to `SecurityError::AuthorizationDenied`.
pub struct ScriptedAuthorizationProvider { /* default: Decision, rules: (kind,action)->Decision */ }
impl ScriptedAuthorizationProvider {
    pub fn allow_all() -> Self;                             // default Allow
    pub fn deny_all() -> Self;                              // default Deny{reason}
    pub fn allow(self, resource_kind: impl Into<String>, action: impl Into<String>) -> Self;
    pub fn deny(self, resource_kind: impl Into<String>, action: impl Into<String>,
                reason: impl Into<String>) -> Self;
}
#[async_trait]
impl AuthorizationProvider for ScriptedAuthorizationProvider {
    async fn authorize(&self, _p: &Principal, req: &AccessRequest, _c: &SecurityContext)
        -> Result<AuthorizationDecision, SecurityError>;   // looks up (req.resource.kind, req.action)
}

// ─── config.rs ───────────────────────────────────────────────────────────────
use ego_service_sdk::ConfigurationProvider;

/// Collects test configuration. Typed values are drained into
/// `RuntimeBuilder::with_config` by the fixture; the JSON view is exposed via
/// `ConfigurationProvider` (AD-5). Unset keys read through the real
/// `resolve_config::<C>()` → `Err(DependencyNotFound)`, never a panic.
pub struct TestConfig { /* root: serde_json::Value, typed: Vec<(TypeId, Arc<dyn Any+Send+Sync>)> */ }
impl TestConfig {
    pub fn new() -> Self;
    /// Register a typed config value resolvable via `resolve_config::<C>()`.
    pub fn with_value<C: Send + Sync + 'static>(self, value: C) -> Self;
    /// Set a key in the JSON-subtree view.
    pub fn set(self, key: impl Into<String>, value: impl serde::Serialize) -> Self;
    /// Real `ConfigurationProvider` over the accumulated JSON root.
    pub fn provider(&self) -> ConfigurationProvider;        // ConfigurationProvider::from_value
}

// ─── logger.rs ───────────────────────────────────────────────────────────────
use std::collections::BTreeMap;
use kitlogger_log_domain::Severity;                        // production level type

/// A parsed view of one emitted log line (recovered from the real JSON formatter).
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedRecord {
    pub level: Severity,
    pub message: String,
    pub fields: BTreeMap<String, serde_json::Value>,
}

/// Real `KITLogger` whose output is redirected into an in-memory buffer (AD-6).
pub struct CapturingLogger { /* logger: Arc<KITLogger>, buffer: Arc<Mutex<Vec<u8>>> */ }
impl CapturingLogger {
    pub fn new() -> Self;                                   // ConsoleExporterImpl + set_writers + init;
                                                            // KITLogger::with_exporter_and_format(_, Json)
    /// The real logger to hand to `ServiceContext::with_logger` / `RuntimeBuilder::with_logger`.
    pub fn logger(&self) -> Arc<KITLogger>;
    /// Parse the buffered JSON output into structured records.
    pub fn records(&self) -> Vec<CapturedRecord>;
}

// ─── fixtures.rs ─────────────────────────────────────────────────────────────
use ego_service_sdk::{Runtime, RuntimeBuilder};
use ego_service_sdk::di::Injectable;
use ego_service_sdk::runtime::RuntimeError;

/// Fully-wired, immediately-usable test setup. Each fixture owns its own
/// `Runtime` — two fixtures share no state (execution independence, isolation).
pub struct ServiceTestFixture { /* runtime: Runtime, context: ServiceContext, logger: CapturingLogger */ }
impl ServiceTestFixture {
    pub fn builder() -> FixtureBuilder;
    /// Default: authenticated `principal()`, `ScriptedAuthorizationProvider::allow_all()`,
    /// `CapturingLogger`, empty `TestConfig` — no further assembly required.
    pub fn new() -> Self;                                   // == builder().build()
    /// A ready `ServiceContext` (security + logger attached), fresh per call.
    pub fn context(&self) -> ServiceContext;
    /// Constructs a service instance through the REAL DI path (AD-9), so its
    /// `ConfigValue<C>` fields resolve runtime-registered config via
    /// `resolve_config::<C>()`. `S` is the service impl struct (macro- or
    /// hand-generated `Injectable`).
    pub fn service<S: Injectable>(&self) -> Result<S, RuntimeError>; // S::build(self.runtime.inner())
    pub fn runtime(&self) -> &Runtime;
    pub fn captured_records(&self) -> Vec<CapturedRecord>;
}

pub struct FixtureBuilder { /* principal, security_mode, authz, config, logger */ }
impl FixtureBuilder {
    pub fn principal(self, principal: Principal) -> Self;
    pub fn unauthenticated(self) -> Self;                   // context security = None (AD-4)
    pub fn authorization(self, authz: Arc<dyn AuthorizationProvider>) -> Self;
    pub fn config(self, config: TestConfig) -> Self;
    pub fn build(self) -> ServiceTestFixture;               // real RuntimeBuilder wiring
}

// ─── assertions.rs ───────────────────────────────────────────────────────────
use ego_service_sdk::ServiceError;
use ego_security_sdk::authorization::{Action, Resource, authorize_in_context};

/// Passes iff `authorize_in_context(..)` returns Ok; else panics with a clear message.
pub async fn assert_authorized(
    provider: &dyn AuthorizationProvider, ctx: &SecurityContext, resource: Resource, action: Action);
/// Passes iff the outcome is `Err(SecurityError::AuthorizationDenied { .. })`.
pub async fn assert_denied(
    provider: &dyn AuthorizationProvider, ctx: &SecurityContext, resource: Resource, action: Action);

/// Asserts a `Result<_, ServiceError>` matches a specific variant, ignoring message text.
/// Usage: `assert_service_error!(result, ServiceError::NotFound { .. });`
#[macro_export]
macro_rules! assert_service_error { /* matches!(result, Err($variant)) or panic with actual */ }
```

## Testing Strategy (strict TDD — `cargo test -p ego-testkit`)

TestKit is a library of test doubles; its own tests are unit tests with no
external resources (testing skill Rule 1). No testcontainers (out of scope).

| Layer | What to test | Approach |
|-------|--------------|----------|
| Unit | `PrincipalBuilder` default builds a valid `Principal`; single-field override leaves others default | build + field assertions |
| Unit | `PrincipalBuilder` yields a real `Principal` usable where `Principal` is required | type-level use + `has_role` |
| Unit | `authenticated()` produces a `SecurityContext` with the given principal/claims | `principal()`/`claims()` assertions |
| Unit | `TestContextBuilder::unauthenticated()` → `ServiceContext.security` is `None`; `require_security()` → `CapabilityNotEnabled` | assert `Err(CapabilityNotEnabled)` |
| Unit | Two independently built `ServiceContext`s don't share state (isolation) | set differing fields, assert no leak |
| Unit | `ScriptedAuthorizationProvider::allow(res, act)` → `authorize_in_context` Ok for that action | real seam, assert Ok |
| Unit | `ScriptedAuthorizationProvider::deny(res, act, reason)` → `Err(AuthorizationDenied { reason })` | assert variant + reason |
| Unit | `TestConfig::with_value::<C>` is observed by a service constructed via `fixture.service::<S>()` (S's `ConfigValue<C>` field resolves it); unset `C` → `Err(DependencyNotFound)` (no panic) | hand-rolled `Injectable` stub resolving `ConfigValue<C>`, build via fixture |
| Unit | `TestConfig::set(key, ..)` is NOT observable via `service::<S>()`/`resolve_config` (AD-5); only via `TestConfig::provider()` | set a key, assert `resolve_config::<C>()` still `DependencyNotFound` |
| Unit | `fixture.service::<S>()` returns a real instance whose trait method yields `Result<T, ServiceError>` | hand-rolled `Injectable` service, invoke method |
| Unit | `TestConfig::provider().logging()` round-trips a set logging view | `ConfigurationProvider` assertion |
| Unit | `CapturingLogger`: structured-path log at a level with fields → `records()` yields a `CapturedRecord` with same level/message/fields | log via `logger()`, parse, assert |
| Unit | `CapturingLogger`: simple `log(Severity, &str)` path → captured record has level+message, `fields` empty (AD-6) | log via `logger()`, assert empty fields |
| Unit | Two `CapturingLogger` instances don't cross-capture | log to each, assert disjoint |
| Unit | `ServiceTestFixture::new()` is immediately usable (context + runtime present, no further setup) | construct, use context |
| Unit | `FixtureBuilder` overriding only `authorization` leaves principal/config/logger at default | build, inspect |
| Unit | `assert_service_error!` passes on matching variant regardless of message; fails on wrong variant | positive + `#[should_panic]` |
| Contract | `ScriptedAuthorizationProvider` is `Arc<dyn AuthorizationProvider>` object-safe & `Send + Sync` | compile assertion |

## Migration / Rollout

Purely additive. New crate, new workspace member; no existing source changes and
no existing call site touched. `ego-security-sdk`, `ego-service-sdk`,
`ego-domain`, and kitlogger are consumed unmodified. The `dev-providers`
passthrough feature is off by default, so a plain `cargo build --workspace` does
not enable `ego-security-sdk/test-helpers` (no feature-unification leak). Downstream
projects add `ego-testkit` as a `[dev-dependencies]` entry.

## Integration Points

- **`RuntimeBuilder`/`Runtime`** — `ServiceTestFixture` composes via
  `RuntimeBuilder::{new, with_security, with_logger, with_config, build}` exactly
  as the reference-app host does. `with_security` is all-or-nothing, so the
  fixture pairs the host's authz provider with a private stub authn (AD-10). It
  exposes `runtime()` and constructs services via the real
  `Injectable::build(runtime().inner())` path (`service::<S>()`, AD-9); `runtime()`
  also gives forward compatibility with a future public `Runtime::resolve`.
- **`ego_security_sdk::authorization`** — `ScriptedAuthorizationProvider`
  implements the async `AuthorizationProvider` trait; assertions call the real
  `authorize_in_context` seam.
- **`ego_service_sdk::ServiceContext`** — builders emit the real type; the fixture
  attaches `Arc<SecurityContext>` and `Arc<KITLogger>` via its existing `with_*`.
- **kitlogger** — `CapturingLogger` builds a real `Arc<KITLogger>` through
  `with_exporter_and_format` + `ConsoleExporterImpl::set_writers`.

## Open Questions — for the Tasks/Apply phase

- [ ] **Capturing logger field mapping.** Confirm, against kitlogger `develop`,
  the exact JSON keys the `LogFormat::Json` formatter emits for level, message,
  and structured fields, so `CapturedRecord` parsing maps them faithfully. If the
  JSON shape is unstable, `CapturedRecord::fields` falls back to the raw parsed
  object under a single top-level key. This is the one place capture depends on
  formatter output rather than a structured seam (forced by AD-6's concrete-exporter
  finding).
- [ ] **Logger entry points (AD-6).** Confirm the exact name/signature of
  `KITLogger`'s structured, field-carrying entry point (the `log_record`-style
  method) and that both it and `log(Severity, &str)` serialize through the
  logger's configured formatter (so both are captured, the latter with empty
  `fields`). If any field-carrying path bypasses the exporter, AD-6's supported
  surface must be narrowed to name only the paths that route through it.
- [ ] **`Injectable::build` across the crate boundary (AD-9).** Confirm
  `ego_service_sdk::di::Injectable` and `RuntimeInner` are reachable from
  `ego-testkit` and that `S::build(fixture.runtime().inner())` type-checks
  (`inner()` yields `&Arc<RuntimeInner>`, deref to `&RuntimeInner`). `Injectable`
  is a `pub` trait and `build` takes `&RuntimeInner` (public), so this is expected
  to compile; verify at apply and adjust the `service::<S>()` signature if a
  visibility gap appears.
- [ ] **`Severity` parse surface.** Confirm `kitlogger_log_domain::Severity`
  round-trips from the formatter's level string (Deserialize or a `FromStr`); if
  not, map the level string explicitly at the boundary (mirrors CORE-017's
  "map by Debug at the boundary" precedent).
- [ ] **`with_value` config typing.** `resolve_config::<C>()` keys by `TypeId`,
  so `TestConfig::with_value` supports one value per concrete type `C`
  (last-write-wins, matching `RuntimeBuilder::with_config`). Confirm no test needs
  two distinct values of the same `C` (would require a newtype, as in production).

## Future Considerations (not decisions pending)

- **Public `Runtime::resolve` execution.** When a public resolve/registration API
  lands, `ServiceTestFixture` can grow a `resolve::<Tag>()` passthrough with no
  change to its construction model. Deferred until that API exists (YAGNI).
- **HTTP/gRPC/database/testcontainer/snapshot/property/bench/perf/chaos helpers.**
  Explicit proposal Non Goals — reserved for future changes; TestKit's crate
  boundary is drawn so any of these could be layered as a separate crate or
  feature without reworking the core.
