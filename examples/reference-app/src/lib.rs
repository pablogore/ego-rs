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
pub mod outbound;
pub mod ports;
pub mod providers;
pub mod read_side;

use ego_domain::persistence::EventStore;
use ego_domain::{Clock, ConfigError, SystemClock, Validate};
use ego_persistence::DatabaseConfig;
use ego_scheduler::event_bus::EventBusConfig;
use ego_security_sdk::{
    AccessRequest, AuthenticationProvider, AuthorizationDecision, AuthorizationProvider, Principal,
    SecurityContext, SecurityError,
};
use ego_service_sdk::{build_logger, App, ConfigurationProvider};
use ego_transport::GrpcServerConfig;
use kit_config::ConfigLoader;
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::runtime::RuntimeConfig;
use security_jwt::{
    Hs256AuthenticationProvider, JwtAlgorithm, JwtProviderConfig, KeyResolver, LocalKeyResolver,
    VerificationKey,
};

use crate::application::{RegisterUserImpl, RegisterUserTag};
use crate::read_side::{ReadSideHandles, ReadSideSink, SharedReadSideStore};

/// Signing key `build_runtime`'s `Hs256AuthenticationProvider` verifies
/// against — `pub` so tests (e.g. `http_route.rs`) can
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

/// Set to any value to make this process abort between the two halves of a
/// `register` operation. Unset installs nothing.
///
/// Only exists under the `crash-test-failpoint` feature, which is off by
/// default. An earlier version of this compiled unconditionally, and that was a
/// defect rather than a cautious default: any ordinary host that *inherited*
/// this variable — a CI environment, an exported shell, a container image built
/// from one — would have aborted mid-`register`. `None` by default does not help
/// when the environment can flip it, and describing that as "no new behaviour in
/// production" was simply wrong.
///
/// Read exactly once, at the composition root: a workflow that consulted the
/// environment itself would carry a second, invisible input.
#[cfg(feature = "crash-test-failpoint")]
pub const CRASH_FAILPOINT_VAR: &str = "EGO_IT_CRASH_AFTER_ORG_RECEIPT";

/// The failpoint this process was asked to install, if any.
#[cfg(feature = "crash-test-failpoint")]
fn aborting_failpoint() -> Option<Arc<dyn crate::application::DualAggregateFailpoint>> {
    if std::env::var(CRASH_FAILPOINT_VAR).is_err() {
        return None;
    }

    struct Abort;
    impl crate::application::DualAggregateFailpoint for Abort {
        /// `abort`, deliberately — not `panic!` and not `exit`.
        ///
        /// SIGABRT stops the process where it stands: nothing unwinds, no
        /// destructor runs, no pool closes and no lease is abandoned on the way
        /// out. A panic would unwind and leave a tidier partial state than any
        /// real crash leaves, so recovery would be tested against a situation
        /// that cannot happen.
        fn after_org_receipt_confirmed(&self) {
            std::process::abort();
        }
    }

    Some(Arc::new(Abort))
}

/// Always `None`, and it reads nothing to decide that.
///
/// The ordinary build: no environment is consulted, and no aborting
/// implementation is compiled for anything to construct. The seam in
/// `RegisterUser` stays exactly where it is — what disappears is any way to
/// reach it from outside the process.
#[cfg(not(feature = "crash-test-failpoint"))]
fn aborting_failpoint() -> Option<Arc<dyn crate::application::DualAggregateFailpoint>> {
    None
}

/// Cross-domain rule threshold (illustrative — see design.md "Validation").
/// A single subtree's own `validate()` cannot see this: it is a policy that
/// only makes sense once `runtime` and `database` are both known.
const MIN_MULTI_TENANT_CONNECTIONS: u32 = 5;

/// Audience this application binds its JWTs to — the value `AppConfig`'s
/// `jwt.expected_aud` requires and the exact `aud` claim tests must mint
/// (see `tests/support::make_token`). `pub` so tests reference this single
/// source of truth instead of duplicating the literal, mirroring
/// [`DEV_SIGNING_KEY`].
///
/// `security_jwt::JwtProviderConfig::validate()` rejects a `None`/empty
/// `expected_aud` (audience-confusion / token-reuse defense), so a real
/// audience is mandatory — the reference app names itself as its own audience.
pub const REFERENCE_APP_AUDIENCE: &str = "reference-app";

/// Illustrative application root config. Real applications define their own
/// type with their own name (spec.md: "The name is application-defined").
// `RuntimeConfig` implements neither `Clone` nor `Debug`, so `AppConfig`
// can't derive them either. `Default` is hand-written (not derived) because
// `JwtProviderConfig`'s own default has `expected_aud: None`, which its
// `validate()` now rejects — this app must pin a concrete audience.
pub struct AppConfig {
    pub runtime: RuntimeConfig,
    pub jwt: JwtProviderConfig,
    pub scheduler: EventBusConfig,
    pub database: DatabaseConfig,
    pub transport: GrpcServerConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            runtime: RuntimeConfig::default(),
            // Pin the audience so `jwt.validate()` passes and every minted
            // token must carry a matching `aud` claim. All other JWT
            // parameters keep their own defaults.
            jwt: JwtProviderConfig {
                expected_aud: Some(vec![REFERENCE_APP_AUDIENCE.to_string()]),
                ..JwtProviderConfig::default()
            },
            scheduler: EventBusConfig::default(),
            database: DatabaseConfig::default(),
            transport: GrpcServerConfig::default(),
        }
    }
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
        if !self.runtime.single_tenant_mode
            && self.database.max_connections < MIN_MULTI_TENANT_CONNECTIONS
        {
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
    /// The entity runtimes this build composed, as it composed them.
    ///
    /// A read-only view, not a second registration: these are the same `Arc`s
    /// the services were handed. It exists because only `UserEntity` is
    /// resolvable through DI — `RegisterUserImpl` holds the organization runtime
    /// by hand — so without it there is no way to observe what
    /// [`build_runtime_with`] actually gave each aggregate.
    ///
    /// That mattered: a durability test that reached for `compose_entity_runtimes`
    /// directly would still pass if `build_runtime_with` ignored the stores it
    /// was handed and composed in-memory ones. Reading them back from here is
    /// what closes that gap without changing DI to suit a test.
    pub entities: ObservedEntityRuntimes,
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
/// Builds the app over **in-memory** event stores.
///
/// The name says so because the alternative misled: this was `build_runtime`,
/// the most natural name in the module, and a host reaching for it got volatile
/// persistence without asking. Nothing it writes survives the process — every
/// event and every confirmed receipt is gone on restart — so a deployment that
/// arrived here would look durable and silently lose the progress receipts exist
/// to record.
///
/// For a real deployment use [`build_runtime_with`], which takes the stores it
/// will use and therefore cannot be reached by omission.
pub fn build_runtime_in_memory(
    config: &AppConfig,
) -> Result<BuiltRuntime, Box<dyn std::error::Error>> {
    build_runtime_observed_in_memory(config, None)
}

/// Every entity runtime this app owns, built from one observability sink.
///
/// This type exists so the sink cannot reach one aggregate and miss another: a
/// caller receives all of them or none, from the single function below.
pub struct ObservedEntityRuntimes {
    /// The `TenantOrganization` aggregate's runtime.
    pub org: Arc<
        persistent_entity::runtime::EntityRuntime<crate::domain::tenant_org::OrganizationEnsured>,
    >,
    /// The `User` aggregate's runtime.
    pub user: Arc<persistent_entity::runtime::EntityRuntime<crate::domain::user::UserRegistered>>,
}

/// The durable event stores this app's aggregates write through.
///
/// Two typed instances over **one shared pool**. They are distinct types because
/// each is parameterised by its aggregate's event, not because they are separate
/// backends: same pool, same tables, same transactions. A pool per aggregate
/// would double the connection budget and buy nothing, since the two never need
/// to be isolated from each other.
///
/// This type exists so the choice of backing store is **stated**, never defaulted.
/// Before it, `compose_entity_runtimes` silently accepted
/// `EntityRuntimeBuilder`'s in-memory default, which meant every event and every
/// receipt lived in process memory — and a restart lost the durable progress the
/// receipts exist to record.
pub struct EntityEventStores {
    /// Where `TenantOrganization` writes.
    pub org: Arc<dyn EventStore<crate::domain::tenant_org::OrganizationEnsured> + Send + Sync>,
    /// Where `User` writes.
    pub user: Arc<dyn EventStore<crate::domain::user::UserRegistered> + Send + Sync>,
}

impl EntityEventStores {
    /// Opens both stores against one shared pool.
    ///
    /// The host has already opened the pool and applied the migrations; this
    /// only binds the two typed views onto it. Opening is fallible on purpose —
    /// `PostgreSQLEventStore::open` refuses while any row is missing its
    /// aggregate type, and that refusal must stop startup rather than be
    /// swallowed into a memory-backed fallback.
    pub async fn open(
        pool: sqlx::PgPool,
    ) -> Result<Self, ego_domain::persistence::PersistenceError> {
        let org = ego_persistence::postgres::event_store::PostgreSQLEventStore::open(
            pool.clone(),
            |_aggregate_type: &str,
             value: serde_json::Value,
             occurred_at: chrono::DateTime<chrono::Utc>| {
                crate::domain::tenant_org::OrganizationEnsured::from_stored(value, occurred_at)
            },
        )
        .await?;
        let user = ego_persistence::postgres::event_store::PostgreSQLEventStore::open(
            pool,
            |_aggregate_type: &str,
             value: serde_json::Value,
             occurred_at: chrono::DateTime<chrono::Utc>| {
                crate::domain::user::UserRegistered::from_stored(value, occurred_at)
            },
        )
        .await?;
        Ok(Self {
            org: Arc::new(org),
            user: Arc::new(user),
        })
    }

    /// In-memory stores, for tests and local development that genuinely want them.
    ///
    /// Spelled out at every call site rather than reachable by omission. Nothing
    /// written through these survives the process, so a composition root that
    /// arrived here by default would look durable and lose every receipt on
    /// restart — which is the failure this whole slice exists to remove.
    pub fn in_memory() -> Self {
        Self {
            org: Arc::new(ego_infrastructure::persistence::in_memory::InMemoryEventStore::new()),
            user: Arc::new(ego_infrastructure::persistence::in_memory::InMemoryEventStore::new()),
        }
    }
}

/// Which idempotency posture this host runs under.
///
/// Named for the contract each carries, not for the decision's status. An earlier
/// draft called the first variant `Deferred`, which said something about the
/// roadmap and nothing about what the host receives — a reader had to go looking
/// to find out that requests without an operation key are admitted.
///
/// There is deliberately no third state. Enforcement declared with nothing behind
/// it is the configuration that looks guarded and guarantees nothing, and the
/// runtime already refuses it at build time; this type makes it unrepresentable
/// rather than merely rejected.
pub enum IdempotencyWiring {
    /// Requests with no operation key are **admitted**.
    ///
    /// The bounded compatibility posture, for a deployment still in transition.
    /// No reservation store is registered — deliberately, because registering an
    /// in-memory one would make the build succeed and the deployment look adopted
    /// while giving no durability at all.
    Compatibility,
    /// Enforcement is on, with everything a lease needs.
    ///
    /// The four travel together because they are not four settings: a store with
    /// no clock cannot compute a lease expiry, an owner with no store means
    /// nothing, and a lease length without a clock is unusable. Carried as one
    /// variant, a host cannot assemble `store` without `owner`, `owner` without
    /// `clock`, or enforcement without a lease.
    Enforced {
        /// Where reservations are held. Durable, or enforcement is decorative.
        store: Arc<dyn ego_domain::operation::OperationReservationStore>,
        /// Who this runtime reserves as. Two replicas must not share one, or
        /// neither can take the other's lease over.
        owner_id: ego_domain::operation::OwnerId,
        /// How long a lease is held before another owner may take it over. Must
        /// exceed the longest a legitimate execution can take.
        lease_duration: std::time::Duration,
        /// The clock expiry is measured against.
        clock: Arc<dyn Clock>,
    },
}

/// **The one place a sink is handed to the entity half of the system.**
///
/// Production calls this, and so does the acceptance test — deliberately the
/// same function, because the failure worth guarding is a *host* that forgets
/// one half, and a test that rebuilt the wiring itself could only ever catch a
/// mistake in its own fixture.
///
/// The handoff cannot be deferred. `EntityRuntime::with_observability` consumes
/// `self`, a host registers the result as an `Arc`, and
/// `Runtime::observability()` only exists once the SDK runtime is already built.
/// So the sink is passed here, before either runtime is finished, or the actors
/// these spawn never report at all — and nothing says so, because the SDK half
/// keeps working.
pub fn compose_entity_runtimes(
    stores: EntityEventStores,
    observability: Option<Arc<dyn ego_domain::Observability>>,
) -> ObservedEntityRuntimes {
    // AD-4: two independent EntityRuntimes, one per aggregate. Both get the
    // sink, or whichever misses it goes dark on its own — and both get their
    // store explicitly, so neither can fall back to memory by omission.
    ObservedEntityRuntimes {
        org: Arc::new(observed_entity_runtime(stores.org, observability.clone())),
        user: Arc::new(observed_entity_runtime(stores.user, observability)),
    }
}

/// One entity runtime, carrying the sink if there is one.
///
/// Generic rather than a closure: each aggregate has its own event type, so a
/// closure would be monomorphised to whichever it was first called with — and
/// the second aggregate would not compile, which is a better failure than the
/// alternative but not one worth arranging on purpose.
fn observed_entity_runtime<E>(
    event_store: Arc<dyn EventStore<E> + Send + Sync>,
    observability: Option<Arc<dyn ego_domain::Observability>>,
) -> persistent_entity::runtime::EntityRuntime<E>
where
    E: ego_domain::DomainEvent
        + Clone
        + serde::de::DeserializeOwned
        + serde::Serialize
        + Send
        + Sync
        + 'static,
{
    let builder = EntityRuntimeBuilder::<E>::new().with_event_store(event_store);
    match observability {
        Some(sink) => builder.with_observability(sink),
        None => builder,
    }
    .build()
}

/// [`build_runtime_in_memory`], with an observability sink threaded through
/// **both** halves.
///
/// The sink goes to `App::builder().observability(...)` for the SDK's own
/// signals and, through [`compose_entity_runtimes`], to every entity runtime.
/// Wiring only the first is the silent failure this signature exists to make
/// hard: the SDK's metrics keep flowing while `idempotency.receipt.outcome`
/// disappears.
/// [`build_runtime_in_memory`], with an observability sink threaded through both
/// halves. Volatile for the same reason, and named for it.
pub fn build_runtime_observed_in_memory(
    config: &AppConfig,
    observability: Option<Arc<dyn ego_domain::Observability>>,
) -> Result<BuiltRuntime, Box<dyn std::error::Error>> {
    build_runtime_with(
        config,
        EntityEventStores::in_memory(),
        IdempotencyWiring::Compatibility,
        observability,
    )
}

/// [`build_runtime_observed_in_memory`], over event stores the caller chose.
///
/// The production entry point. `main` opens a pool, applies the migrations, and
/// hands the durable stores here — so a Postgres that cannot be reached stops
/// startup instead of degrading to memory. Callers that genuinely want in-memory
/// stores say so by passing [`EntityEventStores::in_memory`].
pub fn build_runtime_with(
    config: &AppConfig,
    stores: EntityEventStores,
    idempotency: IdempotencyWiring,
    observability: Option<Arc<dyn ego_domain::Observability>>,
) -> Result<BuiltRuntime, Box<dyn std::error::Error>> {
    config.validate()?;

    let resolver: Arc<dyn KeyResolver> = Arc::new(LocalKeyResolver::new(
        JwtAlgorithm::Hs256,
        VerificationKey::Hmac(DEV_SIGNING_KEY.to_vec()),
    ));
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let authn: Arc<dyn AuthenticationProvider> = Arc::new(Hs256AuthenticationProvider::try_new(
        config.jwt.clone(),
        resolver,
        clock,
    )?);
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

    let ObservedEntityRuntimes {
        org: org_runtime,
        user: user_runtime,
    } = compose_entity_runtimes(stores, observability.clone());
    // Kept so the caller can see what this build composed; the services below
    // get the same `Arc`s.
    let composed = ObservedEntityRuntimes {
        org: org_runtime.clone(),
        user: user_runtime.clone(),
    };

    // UsersByTenant read-side wiring (CORE-005's real engine, not
    // ego-service-sdk's resolve_projection DI mechanism): the sink and the
    // scheduler-facing handles share the same underlying store.
    let read_side_store = SharedReadSideStore::new();
    let read_side_sink = ReadSideSink::new(read_side_store.clone());
    let read_side_handles = ReadSideHandles::new(read_side_store).with_logger(logger.clone());

    let register_user = RegisterUserImpl::new(org_runtime, user_runtime.clone(), None)
        .with_read_side_sink(read_side_sink);
    // Installed here, at the composition root, or not at all. The service itself
    // reads no environment: a workflow that decided mid-operation whether to
    // survive would be a different workflow under test than the one shipped.
    let register_user = Arc::new(match aborting_failpoint() {
        Some(failpoint) => register_user.with_failpoint(failpoint),
        None => register_user,
    });

    let mut builder = App::builder()
        // This service has not adopted operation-key enforcement yet, and says so.
        //
        // The builder's default is the enforcing variant, which refuses to start
        // without a registered `OperationReservationStore` — a runtime that promises
        // every mutating operation carries a client-supplied key and has nowhere to
        // reserve one cannot keep the promise. Declaring `Compatibility` is how a
        // deployment states it is still in transition.
        //
        .security(authn.clone(), authz);
    // Which posture this host runs under is the caller's, not this function's.
    //
    // What has not changed is why an in-memory reservation store is never a
    // stand-in for the durable one: it would make the build succeed and the
    // deployment look adopted while surviving nothing. `Compatibility` registers
    // no store at all, which is the honest shape of "not adopted yet".
    builder = match idempotency {
        IdempotencyWiring::Compatibility => builder.idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        ),
        IdempotencyWiring::Enforced {
            store,
            owner_id,
            lease_duration,
            clock,
        } => builder.enforced_idempotency(store, owner_id, lease_duration, clock),
    };
    // The same sink the entity runtimes were given, for the SDK's own signals.
    if let Some(sink) = observability {
        builder = builder.observability(sink);
    }
    // CORE-028 Stage 2 (AD-5): registers the DI *handle-access* path for the
    // query-side `UsersByTenantStore` — distinct from the untouched read-side
    // *engine* path above (`ReadSideHandles`/`TagSchedulerImpl::spawn`,
    // which keeps producing into it). `UsersByTenantStore` is `Clone` over an
    // internal `Arc<RwLock<_>>` (read_side/projection.rs), so this clone
    // shares live state with the engine-fed store, not a frozen snapshot.
    builder = builder.projection(Arc::new(read_side_handles.query.clone()));
    // CORE-028 Stage 2C (AD-7 item 2): registers the entity-runtime DI path
    // for `UserEntity` through the composition API, as production proof of
    // `.entity::<E>()`/`App::resolve_entity`. Deliberately does NOT migrate
    // `RegisterUserImpl` off its hand-threaded `.service_instance()` wiring
    // below (AD-9 non-goal) — that migration is still blocked by
    // `ReadSideSink`'s hand-wiring, not by entity resolution.
    builder = builder.entity::<crate::domain::user::UserEntity>(user_runtime.clone());
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

    Ok(BuiltRuntime {
        app: builder.build()?,
        authn,
        read_side: read_side_handles,
        entities: composed,
    })
}
