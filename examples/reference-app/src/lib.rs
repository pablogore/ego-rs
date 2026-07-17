//! Reference example for CORE-016 — Application Configuration Model.
//!
//! ego-rs is a library workspace: it ships no host/binary crate of its own,
//! so the root `AppConfig` type does not live in ego-rs (see spec.md
//! "Root Configuration" — the name and ownership are application-defined).
//! This crate demonstrates the pattern a downstream application follows:
//!
//! Host -> AppConfig::validate() -> typed service construction -> RuntimeBuilder
//!
//! `build_runtime` also proves the CORE-016 frozen constraint
//! ("RuntimeBuilder MUST NOT receive raw configuration values") with a real
//! config source: the logging subtree is materialized through the real
//! `kit_config::ConfigLoader` (see `config.toml`), converted to a
//! `ConfigurationProvider`, and turned into a logger via `build_logger`
//! before `RuntimeBuilder` ever sees it. `AppConfig` itself is still
//! constructed directly in-process — the two config models are scoped to
//! different subtrees (`AppConfig`: security/runtime/scheduler/database;
//! kit-config: logging only), not a redundancy.
//!
//! # Module map (hexagonal layout)
//!
//! - [`domain`] — the `User`/`TenantOrganization` `PersistentEntity`
//!   aggregates (design.md AD-4/AD-6): pure Command/Event/State, no
//!   framework or transport concerns.
//! - [`application`] — the `RegisterUser` service (design.md AD-4/AD-5/AD-7):
//!   orchestrates the two domain entities behind a guarded, resolvable
//!   operation. This is the hexagon's core; `domain` is inside it,
//!   everything below is outside it.
//! - [`ports::http`] — the inbound HTTP adapter (design.md AD-2): axum
//!   routes, request/response DTOs shared with OpenAPI, and the Swagger UI.
//!   A future adapter (e.g. gRPC) would sit alongside it, never inside
//!   `application`/`domain`.
//! - [`read_side`] — the `UsersByTenant` read-side projection: a
//!   CORE-005-engine-backed query model fed by real events emitted from
//!   `application`'s write path, queried by `ports::http`.
//!
//! `build_runtime` below is the composition root that wires all four layers
//! together; `main.rs` (this crate's bin target) is the only caller.

use std::sync::Arc;

pub mod application;
pub mod domain;
pub mod ports;
pub mod providers;
pub mod read_side;

use ego_domain::{Clock, ConfigError, SystemClock, Validate};
use ego_persistence::DatabaseConfig;
use ego_scheduler::event_bus::EventBusConfig;
use ego_security_sdk::{
    AccessRequest, AuthenticationProvider, AuthorizationDecision, AuthorizationProvider, Principal, SecurityContext, SecurityError,
};
use ego_service_sdk::{build_logger, App, ConfigurationProvider};
use ego_transport::GrpcServerConfig;
use kit_config::ConfigLoader;
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::runtime::RuntimeConfig;
use security_jwt::{Hs256AuthenticationProvider, JwtAlgorithm, JwtProviderConfig, KeyResolver, LocalKeyResolver, VerificationKey};

use crate::application::{RegisterUserImpl, RegisterUserTag};
use crate::read_side::{ReadSideHandles, ReadSideSink, SharedReadSideStore};

/// Signing key `build_runtime`'s `Hs256AuthenticationProvider` verifies
/// against — `pub` so tests (e.g. `http_route.rs`, `e2e_register.rs`) can
/// mint tokens that authenticate against the exact same runtime, rather
/// than duplicating this literal.
///
/// CORE-018 ground-truth correction: the previous 25-byte literal
/// (`b"reference-app-signing-key"`) was never actually exercised before
/// this change (no HTTP layer existed to invoke `authenticate`), and falls
/// under `Hs256AuthenticationProvider`'s NIST SP 800-107 32-byte HMAC-key
/// minimum — every real authentication attempt against it fails closed
/// with `ProviderUnavailable`. Lengthened to well above that 32-byte floor.
pub const DEV_SIGNING_KEY: &[u8] = b"reference-app-development-signing-key-not-for-prod";

/// Cross-domain rule threshold (illustrative — see design.md "Validation").
/// A single subtree's own `validate()` cannot see this: it is a policy that
/// only makes sense once `runtime` and `database` are both known.
const MIN_MULTI_TENANT_CONNECTIONS: u32 = 5;

/// Illustrative application root config. Real applications define their own
/// type with their own name (spec.md: "The name is application-defined").
// `RuntimeConfig` implements neither `Clone` nor `Debug`, so `AppConfig`
// can't derive them either — Default is enough for this example.
#[derive(Default)]
pub struct AppConfig {
    pub runtime: RuntimeConfig,
    pub jwt: JwtProviderConfig,
    pub scheduler: EventBusConfig,
    pub database: DatabaseConfig,
    pub transport: GrpcServerConfig,
}

impl Validate for AppConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        self.runtime.validate()?;
        self.jwt.validate()?;
        self.scheduler.validate()?;
        self.database.validate()?;
        self.transport.validate()?;

        // Cross-domain rule: a multi-tenant runtime fans out load across
        // tenants, so it needs a database pool sized above the single-tenant
        // default. Neither `RuntimeConfig::validate` nor `DatabaseConfig::validate`
        // can express this alone — it only exists at the AppConfig level.
        if !self.runtime.single_tenant_mode && self.database.max_connections < MIN_MULTI_TENANT_CONNECTIONS {
            return Err(ConfigError::Invalid {
                field: "database.max_connections".to_string(),
                reason: format!(
                    "multi-tenant runtime requires at least {MIN_MULTI_TENANT_CONNECTIONS} database connections"
                ),
            });
        }

        Ok(())
    }
}

// ponytail: `ego-scheduler` / `ego-persistence` don't expose a
// config-consuming service constructor today (no `Scheduler::new(config)` /
// `Database::new(config)`) — these thin wrappers stand in for "a service
// that receives its typed subtree config" without inventing new public API
// in those crates. Add real constructors there if/when a real service needs
// one.
/// Stand-in "service that owns the scheduler subtree config" (see the
/// ponytail note above) — holds `EventBusConfig` only to prove it reached
/// the right owner, does nothing else.
pub struct SchedulerService<'a>(#[allow(dead_code)] pub &'a EventBusConfig);
/// Stand-in "service that owns the database subtree config" (see the
/// ponytail note above) — holds `DatabaseConfig` only to prove it reached
/// the right owner, does nothing else.
pub struct DatabaseService<'a>(#[allow(dead_code)] pub &'a DatabaseConfig);

// ponytail: local example-only stand-in for `ego-security-sdk`'s
// `AllowAllAuthorizationProvider`, which lives behind the `test-helpers`
// feature. Depending on that feature here would unify it into every other
// workspace member that links `ego-security-sdk` (e.g. `security-jwt`,
// `security-apikey`, `service-sdk`) for any `cargo build/test --workspace`
// run — a real feature-unification leak for a permanent workspace member.
// A real application supplies its own policy.
struct ReferenceAllowAllAuthorization;

#[async_trait::async_trait]
impl AuthorizationProvider for ReferenceAllowAllAuthorization {
    async fn authorize(
        &self,
        _principal: &Principal,
        _request: &AccessRequest,
        _ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Allow)
    }
}

/// `build_runtime`'s success payload: the constructed, not-yet-started
/// [`App`], the `authn` provider `ego_transport::AppState` needs, and the
/// not-yet-spawned `UsersByTenant` read-side wiring.
pub struct BuiltRuntime {
    pub app: App,
    pub authn: Arc<dyn AuthenticationProvider>,
    pub read_side: ReadSideHandles,
}

/// Host -> AppConfig -> service construction -> `App::builder()` pipeline
/// (CORE-028 Stage 1 PR2 — migrated from constructing `RuntimeBuilder`
/// directly; see design.md AD-1: `AppBuilder` delegates to `RuntimeBuilder`,
/// it does not replace it).
///
/// Mirrors design.md's Data Flow: validation runs before any service is
/// constructed, then each subtree's typed config goes only to the service
/// that owns it. `AppBuilder` (like the `RuntimeBuilder` it delegates to)
/// never receives raw configuration — it only ever receives
/// already-constructed security providers and, when present, an
/// already-constructed logger materialized through kit-config.
///
/// CORE-018 TASK-024: also builds the two `EntityRuntime`s (AD-4) and
/// registers `RegisterUser` via `.service_instance()` (see the FLAG comment
/// at that call site for why `.service::<S, Tag>()`'s `Injectable` path
/// isn't used), and returns the constructed `authn` alongside the built
/// `App` (previously discarded after `.with_security(authn, authz)`) so a
/// caller (e.g. `main.rs`) can build `ego_transport::AppState` via
/// [`App::runtime`].
///
/// Also wires the `UsersByTenant` read-side projection (new capability,
/// see `crate::read_side`): a `ReadSideSink` is attached to the
/// `RegisterUser` write path, and the not-yet-spawned `ReadSideHandles` are
/// returned alongside `App`/`authn` — `ReadSideHandles::new` is a plain
/// sync call (safe from `tests/pipeline.rs`'s non-Tokio tests); only
/// `ReadSideHandles::spawn` requires a running Tokio runtime, so the
/// caller (`main.rs`) decides when to start the background poller.
pub fn build_runtime(config: &AppConfig) -> Result<BuiltRuntime, Box<dyn std::error::Error>> {
    config.validate()?;

    let resolver: Arc<dyn KeyResolver> = Arc::new(LocalKeyResolver::new(
        JwtAlgorithm::Hs256,
        VerificationKey::Hmac(DEV_SIGNING_KEY.to_vec()),
    ));
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let authn: Arc<dyn AuthenticationProvider> =
        Arc::new(Hs256AuthenticationProvider::new(config.jwt.clone(), resolver, clock));
    let authz: Arc<dyn AuthorizationProvider> = Arc::new(ReferenceAllowAllAuthorization);

    let _scheduler = SchedulerService(&config.scheduler);
    let _database = DatabaseService(&config.database);

    // Materialize configuration through the real kit-config loader before any
    // AppBuilder/RuntimeBuilder construction begins. Only the resulting
    // materialized `LoggingSettings` (via `ConfigurationProvider`) reaches
    // it — never the loader or its raw sources (CORE-016 frozen constraint).
    let map = ConfigLoader::builder()
        .add_defaults()
        // ponytail: file sources (TomlFileSource, priority 200) outrank
        // environment sources (EnvironmentSource, priority 50) in kit-config
        // today, and `add_environment()` only ever produces flat top-level
        // `Value::String` keys — it can never populate or override the
        // nested `logging` object below. This is observed kit-config
        // behavior, not an ego-rs guarantee; kept here to demonstrate it,
        // not to be relied upon for precedence.
        .add_environment()
        .add_toml(concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml"))
        .build()?
        .load()?;
    let settings = ConfigurationProvider::from_value(serde_json::to_value(map)?).logging()?;
    let logger = build_logger(&settings)?;

    // AD-4: two independent EntityRuntimes, one per aggregate.
    let org_runtime = Arc::new(EntityRuntimeBuilder::new().build());
    let user_runtime = Arc::new(EntityRuntimeBuilder::new().build());

    // UsersByTenant read-side wiring (CORE-005's real engine, not
    // ego-service-sdk's resolve_projection DI mechanism): the sink and the
    // scheduler-facing handles share the same underlying store.
    let read_side_store = SharedReadSideStore::new();
    let read_side_sink = ReadSideSink::new(read_side_store.clone());
    let read_side_handles = ReadSideHandles::new(read_side_store).with_logger(logger.clone());

    let register_user = Arc::new(RegisterUserImpl::new(org_runtime, user_runtime, None).with_read_side_sink(read_side_sink));

    let mut builder = App::builder().security(authn.clone(), authz);
    // FLAG (design.md AD-3, task 5.1): `RegisterUserImpl` does not — and, as
    // built today, cannot cheaply — implement `Injectable`. Its two
    // `EntityRuntime`s aren't `AdapterRef`/`ConfigValue`-resolvable (no
    // generic DI mechanism resolves a per-aggregate `EntityRuntime`; see
    // explore.md #15), and its `read_side_sink` is a hand-wired collaborator
    // assembled above, not something `Injectable::build` could construct
    // from the registry. `.service::<S, Tag>()`'s `Injectable` path was
    // rejected for this reason, not overlooked — `.service_instance()` is
    // the correct, intended escape hatch here (AD-3), not a workaround.
    builder = builder.service_instance::<RegisterUserTag>(register_user);
    if let Some(logger) = logger {
        builder = builder.logger(logger);
    }

    Ok(BuiltRuntime { app: builder.build()?, authn, read_side: read_side_handles })
}
