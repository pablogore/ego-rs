//! Service test fixture (CORE-022 Phase 8, design.md AD-9, AD-10).

use std::sync::Arc;

use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::AuthorizationProvider;
use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::credential::Credential;
use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};
use ego_security_sdk::AuthenticationError;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::di::Injectable;
use ego_service_sdk::runtime::RuntimeError;
use ego_service_sdk::{Runtime, RuntimeBuilder};

use crate::authz::ScriptedAuthorizationProvider;
use crate::config::TestConfig;
use crate::identity::principal;
use crate::logger::{CapturedRecord, CapturingLogger};
use crate::security::authenticated;

/// Satisfies `RuntimeBuilder::with_security`'s all-or-nothing pairing so the
/// real authz provider is retained (design.md AD-10). A real
/// `AuthenticationProvider` impl, not a parallel type — but it is never
/// actually invoked: AD-1 means a test calls the service's own trait method
/// directly rather than driving a credential through `Runtime`. Deliberately
/// **not** `pub` — see design.md AD-10 and task 8.4's privacy check; exposing
/// it would imply an authentication-driven execution model TestKit does not
/// offer.
pub(crate) struct PairingAuthnStub;

impl AuthenticationProvider for PairingAuthnStub {
    fn authenticate(
        &self,
        _credential: &Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        let subject =
            SubjectId::new("testkit:pairing-stub").expect("fixed subject is always valid");
        let principal = Principal::new(PrincipalKind::User, subject);
        Ok(SecurityContext::empty(principal))
    }
}

/// Fully-wired, immediately-usable test setup. Each fixture owns its own
/// `Runtime` — two fixtures share no state (execution independence,
/// isolation; design.md AD-1).
pub struct ServiceTestFixture {
    runtime: Runtime,
    context: ServiceContext,
    logger: CapturingLogger,
}

impl ServiceTestFixture {
    /// Starts a [`FixtureBuilder`] for customizing the fixture before build.
    pub fn builder() -> FixtureBuilder {
        FixtureBuilder::new()
    }

    /// Default: authenticated `principal()`,
    /// `ScriptedAuthorizationProvider::allow_all()`, a fresh `CapturingLogger`,
    /// and an empty `TestConfig` — no further assembly required.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// A ready `ServiceContext` (security + logger attached), fresh per call
    /// (`ServiceContext` is a cheaply-cloned value type).
    pub fn context(&self) -> ServiceContext {
        self.context.clone()
    }

    /// Constructs a service instance through the REAL `Injectable::build` DI
    /// path (design.md AD-9), so its `ConfigValue<C>`/`AdapterRef<A>`/
    /// `ProjectionRef<P>` fields resolve runtime-registered dependencies
    /// exactly as production does. `S` may be a hand-rolled `Injectable` impl
    /// or a `#[service]`-macro-generated one — both route through this same
    /// call.
    pub fn service<S: Injectable>(&self) -> Result<S, RuntimeError> {
        S::build(self.runtime.inner())
    }

    /// The underlying real `Runtime`, for forward compatibility with a future
    /// public `Runtime::resolve`.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Records captured by this fixture's `CapturingLogger` so far.
    pub fn captured_records(&self) -> Vec<CapturedRecord> {
        self.logger.records()
    }
}

impl Default for ServiceTestFixture {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds a [`ServiceTestFixture`]; overriding one building block leaves
/// every other one at its default.
pub struct FixtureBuilder {
    principal: Option<Principal>,
    unauthenticated: bool,
    authorization: Arc<dyn AuthorizationProvider>,
    config: TestConfig,
}

impl FixtureBuilder {
    fn new() -> Self {
        Self {
            principal: None,
            unauthenticated: false,
            authorization: Arc::new(ScriptedAuthorizationProvider::allow_all()),
            config: TestConfig::new(),
        }
    }

    /// Overrides the authenticated principal (default: `principal()`).
    pub fn principal(mut self, principal: Principal) -> Self {
        self.principal = Some(principal);
        self.unauthenticated = false;
        self
    }

    /// Leaves the fixture's `ServiceContext.security` as `None` — represents
    /// "no authenticated principal" the same way production code does
    /// (design.md AD-4).
    pub fn unauthenticated(mut self) -> Self {
        self.unauthenticated = true;
        self
    }

    /// Overrides the authorization provider (default:
    /// `ScriptedAuthorizationProvider::allow_all()`).
    pub fn authorization(mut self, authz: Arc<dyn AuthorizationProvider>) -> Self {
        self.authorization = authz;
        self
    }

    /// Overrides the test configuration (default: empty `TestConfig`).
    pub fn config(mut self, config: TestConfig) -> Self {
        self.config = config;
        self
    }

    /// Builds the fixture, wiring a real `RuntimeBuilder`.
    pub fn build(self) -> ServiceTestFixture {
        let logger = CapturingLogger::new();

        let security = if self.unauthenticated {
            None
        } else {
            Some(authenticated(self.principal.unwrap_or_else(principal)))
        };

        let runtime = self
            .config
            .drain_into(RuntimeBuilder::new())
            .with_security(Arc::new(PairingAuthnStub), self.authorization)
            .with_logger(logger.logger())
            .build();

        let mut context = ServiceContext::new().with_logger(logger.logger());
        if let Some(sec) = security {
            context = context.with_security(Arc::new(sec));
        }

        ServiceTestFixture {
            runtime,
            context,
            logger,
        }
    }
}

impl Default for FixtureBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ego_security_sdk::authorization::AuthorizationProvider;
    use ego_service_sdk::di::{ConfigValue, DepKey, Injectable};
    use ego_service_sdk::runtime::RuntimeError;
    use ego_service_sdk::ServiceError;
    use ego_service_sdk_macros::service;

    use crate::config::TestConfig;
    use crate::fixtures::ServiceTestFixture;

    /// Hand-rolled `Injectable` service resolving a `ConfigValue<u32>` field —
    /// exercises AD-5's deferred "observed via `resolve_config` by a real
    /// service" scenario without depending on the `#[service]` macro.
    struct HandRolledService {
        limit: ConfigValue<u32>,
    }

    impl Injectable for HandRolledService {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Config(std::any::TypeId::of::<u32>())]
        }

        fn build(rt: &ego_service_sdk::runtime::RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(Self {
                limit: rt.resolve_config::<u32>()?,
            })
        }
    }

    impl HandRolledService {
        async fn check(&self, value: u32) -> Result<u32, ServiceError> {
            if value > *self.limit {
                return Err(ServiceError::validation("value exceeds configured limit"));
            }
            Ok(value)
        }
    }

    /// Real `#[service]`-macro-generated struct — proves `fixture.service::<S>()`
    /// drives the SAME `Injectable::build` path the macro generates, not a
    /// shortcut that only works for hand-rolled impls (task 8.2).
    #[service]
    struct MacroService {
        limit: ConfigValue<u32>,
    }

    #[tokio::test]
    async fn hand_rolled_service_observes_registered_config_value() {
        let fixture = ServiceTestFixture::builder()
            .config(TestConfig::new().with_value(10u32))
            .build();

        let svc = fixture
            .service::<HandRolledService>()
            .expect("service constructs through the real Injectable::build path");

        assert_eq!(svc.check(5).await, Ok(5));
        assert_eq!(
            svc.check(50).await,
            Err(ServiceError::validation("value exceeds configured limit"))
        );
    }

    #[tokio::test]
    async fn hand_rolled_service_unset_config_is_dependency_not_found_never_a_panic() {
        let fixture = ServiceTestFixture::new();

        let result = fixture.service::<HandRolledService>();

        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    #[test]
    fn macro_generated_service_resolves_config_identically_to_hand_rolled_case() {
        let fixture = ServiceTestFixture::builder()
            .config(TestConfig::new().with_value(10u32))
            .build();

        // Proves `fixture.service::<S>()` is driving the exact same
        // `Injectable::build` path the `#[service]` macro's generated impl
        // uses — not a hand-rolled-only shortcut (task 8.2, concern #1).
        let svc = fixture
            .service::<MacroService>()
            .expect("macro-generated Injectable::build resolves config identically");
        assert_eq!(*svc.limit, 10u32);
    }

    #[test]
    fn new_is_immediately_usable_with_defaults() {
        let fixture = ServiceTestFixture::new();

        // Immediately usable: a fresh context is available, no further
        // assembly required.
        let ctx = fixture.context();
        assert!(ctx.security().is_some());
        assert!(fixture.captured_records().is_empty());
    }

    #[tokio::test]
    async fn builder_overriding_only_authorization_leaves_rest_at_default() {
        use ego_security_sdk::{authorize_in_context, Action, Resource, SecurityError};

        let deny_all: Arc<dyn AuthorizationProvider> =
            Arc::new(crate::authz::ScriptedAuthorizationProvider::deny_all());

        let fixture = ServiceTestFixture::builder()
            .authorization(deny_all)
            .build();

        // The override actually took effect: default is allow_all, so a
        // deny_all provider registered here must really deny, through the
        // real `authorize_in_context` seam.
        let authz = fixture
            .runtime()
            .inner()
            .authorization_provider()
            .expect("authorization provider registered");
        let ctx = fixture.context();
        let result = authorize_in_context(
            ctx.security(),
            Resource {
                kind: "any".into(),
                id: None,
            },
            Action("any".into()),
            authz.as_ref(),
        )
        .await;
        assert!(matches!(
            result,
            Err(SecurityError::AuthorizationDenied { .. })
        ));

        // Principal stays at default: same subject as the crate's default
        // `principal()`, not just "some" security context.
        let sec = ctx.security().expect("still authenticated by default");
        assert_eq!(
            sec.principal().subject_id.as_str(),
            crate::identity::principal().subject_id.as_str()
        );

        // Config stays at default (empty): unset `u32` still resolves to
        // DependencyNotFound, never a panic.
        assert!(matches!(
            fixture.service::<HandRolledService>(),
            Err(RuntimeError::DependencyNotFound)
        ));

        // Logger stays at default: fresh, nothing captured yet.
        assert!(fixture.captured_records().is_empty());
    }

    #[test]
    fn builder_principal_overrides_the_default_subject() {
        let custom = crate::identity::PrincipalBuilder::new()
            .subject("custom:subject")
            .build();

        let fixture = ServiceTestFixture::builder().principal(custom).build();

        let ctx = fixture.context();
        let sec = ctx.security().expect("authenticated by explicit principal");
        assert_eq!(sec.principal().subject_id.as_str(), "custom:subject");
    }

    #[test]
    fn builder_unauthenticated_leaves_security_none() {
        let fixture = ServiceTestFixture::builder().unauthenticated().build();

        assert!(fixture.context().security().is_none());
    }
}
