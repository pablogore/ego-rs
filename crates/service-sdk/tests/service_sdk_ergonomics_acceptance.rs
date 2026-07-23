//! CORE-025 TASK-020 — end-to-end developer-journey acceptance walkthrough.
//!
//! One realistic narrative covering, in sequence, all 5 scenarios the
//! proposal names (not scattered unit assertions — those already live
//! per-task in `with_service_resolve.rs`, `runtime/builder.rs`'s test
//! module, and `testkit/src/fixtures.rs`):
//!
//! 1. Minimal service, no deps: define / `with_service` / `build()` /
//!    `resolve()` / invoke.
//! 2. Service with dependencies (adapter + typed config via `Injectable`):
//!    `with_adapter`/`with_config` / `with_injectable` / `try_build()` /
//!    `Injectable::build(rt.inner())` / invoke.
//! 3. The same DI service missing its adapter: `try_build()` fails fast with
//!    `DependencyNotFound { type_name, service_name }` naming both.
//! 4. TestKit: the same kind of trait-proxy service constructed through
//!    `ServiceTestFixture::builder().with_service(..)` + `.resolve(..)` —
//!    proving no parallel wiring.
//! 5. A `#[tenant_scoped]` service registered via `with_service` and
//!    resolved via `resolve`, invoked with a `ServiceContext` for which
//!    tenant resolution fails — same guard order/`SecurityError` as the
//!    hand-rolled path.
//!
//! Compile + runtime test only — no real DB/broker/HTTP I/O, only in-memory
//! runtime state. Same placement/style as `proxy_codegen.rs`/
//! `tenant_scoped_codegen.rs`/`with_service_resolve.rs`; this repo has no
//! `crates/integration-tests/` and none of these scenarios need one.
//!
//! Run with: cargo test -p ego-service-sdk --test service_sdk_ergonomics_acceptance

use std::any::TypeId;
use std::sync::Arc;

use async_trait::async_trait;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::di::{AdapterRef, ConfigValue, DepKey, Injectable};
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::{ServiceError, ServiceErrorTrait};
use ego_service_sdk::runtime::{RuntimeBuilder, RuntimeError, RuntimeInner};
use ego_service_sdk::security::SecurityError;
use ego_service_sdk_macros::service;
#[allow(unused_imports)]
use ego_service_sdk_macros::{operation, tenant_scoped};

// ---------------------------------------------------------------------------
// Scenario 1: minimal service, no deps
// ---------------------------------------------------------------------------

#[service(version = "1.0.0")]
pub trait MinimalGreeter {
    #[operation]
    async fn greet(&self, ctx: ServiceContext, name: String) -> Result<String, ServiceError>;
}

struct MinimalGreeterImpl;

#[async_trait]
impl MinimalGreeter for MinimalGreeterImpl {
    async fn greet(&self, _ctx: ServiceContext, name: String) -> Result<String, ServiceError> {
        Ok(format!("hello, {name}"))
    }
}

// ---------------------------------------------------------------------------
// Scenarios 2 & 3: DI service depending on an adapter + typed config
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
struct NotifierAdapter(String);

struct ConfiguredGreeter {
    adapter: AdapterRef<NotifierAdapter>,
    limit: ConfigValue<u32>,
}

impl Injectable for ConfiguredGreeter {
    fn dependencies() -> Vec<DepKey> {
        vec![
            DepKey::Adapter(
                TypeId::of::<NotifierAdapter>(),
                std::any::type_name::<NotifierAdapter>(),
            ),
            DepKey::Config(TypeId::of::<u32>(), std::any::type_name::<u32>()),
        ]
    }

    fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> {
        Ok(Self {
            adapter: rt.resolve_adapter::<NotifierAdapter>()?,
            limit: rt.resolve_config::<u32>()?,
        })
    }
}

impl ConfiguredGreeter {
    fn greet(&self, name: &str) -> String {
        format!(
            "{}: hello {name} (retry limit {})",
            self.adapter.0, *self.limit
        )
    }
}

// ---------------------------------------------------------------------------
// Scenario 5: tenant-scoped protected service
// ---------------------------------------------------------------------------

/// Domain error with `From<SecurityError>` — required for `#[tenant_scoped]`
/// codegen's fallible `enforce_tenant(..)?` call site (mirrors
/// `with_service_resolve.rs::TenantHelloError`). Preserves the originating
/// `SecurityError` variant (not just its `Display` text) so tests can
/// assert on the actual cause, not a string a different error could
/// coincidentally also produce.
#[derive(Debug)]
pub enum TenantGreeterError {
    Security(SecurityError),
}

impl From<SecurityError> for TenantGreeterError {
    fn from(e: SecurityError) -> Self {
        Self::Security(e)
    }
}

impl ServiceErrorTrait for TenantGreeterError {
    fn code(&self) -> &str {
        "TENANT_GREETER_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        match self {
            Self::Security(e) => e.to_string(),
        }
    }
}

#[service(version = "1.0.0")]
pub trait ProtectedGreeter {
    #[operation]
    #[tenant_scoped]
    async fn greet(&self, ctx: ServiceContext) -> Result<String, TenantGreeterError>;
}

struct ProtectedGreeterImpl;

#[async_trait]
impl ProtectedGreeter for ProtectedGreeterImpl {
    async fn greet(&self, _ctx: ServiceContext) -> Result<String, TenantGreeterError> {
        Ok("hello, tenant".to_string())
    }
}

// ---------------------------------------------------------------------------
// The narrative
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_developer_journey_from_minimal_service_to_protected_service() {
    // --- Scenario 1: minimal service, no deps ---------------------------
    // Define -> with_service -> build() -> resolve() -> invoke.
    let rt = RuntimeBuilder::new()
        .with_service::<MinimalGreeterTag>(Arc::new(MinimalGreeterImpl) as Arc<dyn MinimalGreeter>)
        .expect("first registration for a fresh tag succeeds")
        .build();

    let minimal = rt
        .resolve::<MinimalGreeterTag>()
        .expect("registered tag resolves to Ok(MinimalGreeterRef)");
    let out = minimal
        .greet(ServiceContext::new(), "world".to_string())
        .await
        .expect("invocation succeeds exactly as the hand-rolled path would");
    assert_eq!(out, "hello, world");

    // --- Scenario 2: service with dependencies (adapter + typed config) --
    // with_adapter/with_config -> with_injectable -> try_build() ->
    // Injectable::build(rt.inner()) -> invoke.
    let rt = RuntimeBuilder::new()
        .with_adapter(Arc::new(NotifierAdapter("notifier".to_string())))
        .with_config(Arc::new(10u32))
        .with_injectable::<ConfiguredGreeter>()
        .try_build()
        .expect("all recorded dependencies present, try_build must succeed");

    let configured = ConfiguredGreeter::build(rt.inner())
        .expect("build() succeeds using the same resolved adapter/config try_build validated");
    assert_eq!(
        configured.greet("world"),
        "notifier: hello world (retry limit 10)"
    );

    // --- Scenario 3: missing dependency ----------------------------------
    // The same DI service, this time with no adapter registered — try_build()
    // must fail fast, naming both the missing type and the requesting
    // service, never reaching Injectable::build.
    let err = match RuntimeBuilder::new()
        .with_config(Arc::new(10u32))
        .with_injectable::<ConfiguredGreeter>()
        .try_build()
    {
        Err(e) => e,
        Ok(_) => panic!("try_build must fail fast when the adapter dependency is missing"),
    };
    match err {
        RuntimeError::DependencyNotFound { type_name, service_name, .. } => {
            assert_eq!(
                type_name,
                std::any::type_name::<NotifierAdapter>(),
                "error must name the missing adapter type"
            );
            assert_eq!(
                service_name,
                Some(std::any::type_name::<ConfiguredGreeter>()),
                "error must name the requesting service"
            );
        }
        other => panic!(
            "expected DependencyNotFound naming both NotifierAdapter and ConfiguredGreeter, got {other:?}"
        ),
    }

    // --- Scenario 4: TestKit uses the same canonical path -----------------
    // The same kind of trait-proxy service (MinimalGreeter), constructed
    // through ServiceTestFixture instead of RuntimeBuilder directly — no
    // parallel wiring, same with_service/resolve calls under the hood.
    let fixture = ego_testkit::ServiceTestFixture::builder()
        .with_service::<MinimalGreeterTag>(Arc::new(MinimalGreeterImpl) as Arc<dyn MinimalGreeter>)
        .expect("fixture registration succeeds")
        .build();

    let fixture_proxy = fixture
        .resolve::<MinimalGreeterTag>()
        .expect("fixture resolves the registered tag through its real runtime");
    let fixture_out = fixture_proxy
        .greet(fixture.context(), "fixture".to_string())
        .await
        .expect("fixture invocation succeeds through the real generated proxy");
    assert_eq!(fixture_out, "hello, fixture");

    // --- Scenario 5: protected/tenant-scoped service ----------------------
    // Registered via with_service, resolved via resolve — must still fail
    // closed with the same guard order and SecurityError the hand-rolled
    // path enforces when no tenant can be resolved from the context.
    let rt = RuntimeBuilder::new()
        .with_service::<ProtectedGreeterTag>(
            Arc::new(ProtectedGreeterImpl) as Arc<dyn ProtectedGreeter>
        )
        .expect("registration succeeds")
        .build();

    let protected = rt
        .resolve::<ProtectedGreeterTag>()
        .expect("registered tenant-scoped tag resolves");
    let result = protected.greet(ServiceContext::new()).await;
    assert!(
        matches!(
            result,
            Err(TenantGreeterError::Security(SecurityError::MissingContext))
        ),
        "tenant-scoped op resolved via `resolve` must fail closed with the same \
         SecurityError::MissingContext the hand-rolled path (tenant_scoped_codegen.rs) reports — \
         resolution introduces no alternate, relaxed code path; got {result:?}"
    );
}
