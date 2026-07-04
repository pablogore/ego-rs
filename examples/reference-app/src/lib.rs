//! Reference example for CORE-016 — Application Configuration Model.
//!
//! ego-rs is a library workspace: it ships no host/binary crate of its own,
//! so the root `AppConfig` type does not live in ego-rs (see spec.md
//! "Root Configuration" — the name and ownership are application-defined).
//! This crate demonstrates the pattern a downstream application follows:
//!
//! Host -> AppConfig::validate() -> typed service construction -> RuntimeBuilder
//!
//! `kit-config` (materialization + secrets) is intentionally out of scope —
//! this example constructs `AppConfig` directly in-process to keep the
//! pipeline runnable without an external dependency that does not exist in
//! this workspace.

use std::sync::Arc;

use ego_domain::{Clock, ConfigError, SystemClock, Validate};
use ego_persistence::DatabaseConfig;
use ego_scheduler::event_bus::EventBusConfig;
use ego_security_sdk::{
    AccessRequest, AuthenticationProvider, AuthorizationDecision, AuthorizationProvider, Principal, SecurityContext, SecurityError,
};
use ego_service_sdk::{Runtime, RuntimeBuilder};
use ego_transport::GrpcServerConfig;
use persistent_entity::runtime::RuntimeConfig;
use security_jwt::{Hs256AuthenticationProvider, JwtAlgorithm, JwtProviderConfig, KeyResolver, LocalKeyResolver, VerificationKey};

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
pub struct SchedulerService<'a>(#[allow(dead_code)] pub &'a EventBusConfig);
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

/// Host -> AppConfig -> service construction -> RuntimeBuilder pipeline.
///
/// Mirrors design.md's Data Flow: validation runs before any service is
/// constructed, then each subtree's typed config goes only to the service
/// that owns it. `RuntimeBuilder` never receives raw configuration — it
/// only ever receives already-constructed security providers.
pub fn build_runtime(config: &AppConfig) -> Result<Runtime, ConfigError> {
    config.validate()?;

    let resolver: Arc<dyn KeyResolver> = Arc::new(LocalKeyResolver::new(
        JwtAlgorithm::Hs256,
        VerificationKey::Hmac(b"reference-app-signing-key".to_vec()),
    ));
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let authn: Arc<dyn AuthenticationProvider> =
        Arc::new(Hs256AuthenticationProvider::new(config.jwt.clone(), resolver, clock));
    let authz: Arc<dyn AuthorizationProvider> = Arc::new(ReferenceAllowAllAuthorization);

    let _scheduler = SchedulerService(&config.scheduler);
    let _database = DatabaseService(&config.database);

    Ok(RuntimeBuilder::new().with_security(authn, authz).build())
}
