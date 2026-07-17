//! Integration tests for CORE-028 Stage 1 `App`/`AppBuilder` (design.md
//! CORE-028). Exercises the real `#[service]`/`#[operation]` macros — their
//! generated code references `ego_service_sdk::...` paths that don't resolve
//! from inside the crate's own unit tests, so these live here as a
//! crate-local `tests/` integration file, same placement as
//! `with_service_resolve.rs` (whose module doc explains the same
//! constraint).
//!
//! Run with: cargo test -p ego-service-sdk --test app_composition

use std::sync::Arc;

use async_trait::async_trait;
use ego_service_sdk::app::App;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::di::{AdapterRef, DepKey, Injectable};
use ego_service_sdk::error::ServiceError;
use ego_service_sdk::runtime::RuntimeError;
#[allow(unused_imports)]
use ego_service_sdk_macros::{operation, service};

// ---------------------------------------------------------------------------
// Task 2.2/2.4: `.service::<S, Tag>()` — real Injectable + Tag + Resolvable
// ---------------------------------------------------------------------------

#[service(version = "1.0.0")]
pub trait GreetingService {
    #[operation]
    async fn greet(&self, ctx: ServiceContext, name: String) -> Result<String, ServiceError>;
}

/// A concrete `GreetingService` impl that is ALSO `Injectable`, hand-rolled
/// (mirrors testkit's `HandRolledService` pattern) so `.service::<S, Tag>()`
/// can construct it through the real `Injectable::build` path rather than
/// requiring a pre-built instance.
struct GreetingServiceImpl {
    adapter: AdapterRef<GreeterAdapter>,
}

struct GreeterAdapter(String);

impl Injectable for GreetingServiceImpl {
    fn dependencies() -> Vec<DepKey> {
        vec![DepKey::Adapter(
            std::any::TypeId::of::<GreeterAdapter>(),
            std::any::type_name::<GreeterAdapter>(),
        )]
    }

    fn build(rt: &ego_service_sdk::runtime::RuntimeInner) -> Result<Self, RuntimeError> {
        Ok(Self {
            adapter: rt.resolve_adapter::<GreeterAdapter>()?,
        })
    }
}

#[async_trait]
impl GreetingService for GreetingServiceImpl {
    async fn greet(&self, _ctx: ServiceContext, name: String) -> Result<String, ServiceError> {
        Ok(format!("{}, {name}", self.adapter.0))
    }
}

#[tokio::test]
async fn registered_service_with_satisfied_dependencies_resolves() {
    let app = App::builder()
        .adapter(Arc::new(GreeterAdapter("hello".to_string())))
        .service::<GreetingServiceImpl, GreetingServiceTag>(|arc| arc)
        .build()
        .expect("all dependencies satisfied, build must succeed");

    let proxy = app
        .resolve::<GreetingServiceTag>()
        .expect("registered service must resolve via its Tag");
    let out = proxy
        .greet(ServiceContext::new(), "world".to_string())
        .await
        .expect("invocation succeeds");
    assert_eq!(out, "hello, world");
}

#[test]
fn missing_dependency_names_both_type_and_requester() {
    let result = App::builder()
        .service::<GreetingServiceImpl, GreetingServiceTag>(|arc| arc)
        .build();

    match result {
        Ok(_) => panic!("expected build to fail on a missing adapter dependency"),
        Err(ego_service_sdk::app::CompositionError::Validation(RuntimeError::DependencyNotFound {
            type_name,
            service_name,
        })) => {
            assert_eq!(type_name, std::any::type_name::<GreeterAdapter>());
            assert_eq!(service_name, Some(std::any::type_name::<GreetingServiceImpl>()));
        }
        Err(other) => panic!("expected Validation(DependencyNotFound) naming type+requester, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Task 2.6: `.service_instance::<Tag>()` — pre-built escape hatch (AD-3 flag)
// ---------------------------------------------------------------------------

struct PreBuiltGreeter;

#[async_trait]
impl GreetingService for PreBuiltGreeter {
    async fn greet(&self, _ctx: ServiceContext, name: String) -> Result<String, ServiceError> {
        Ok(format!("hi, {name}"))
    }
}

#[tokio::test]
async fn service_instance_registers_a_pre_built_instance_resolvable_under_its_tag() {
    let instance: Arc<dyn GreetingService> = Arc::new(PreBuiltGreeter);
    let app = App::builder()
        .service_instance::<GreetingServiceTag>(instance)
        .build()
        .expect("service_instance registration succeeds");

    let proxy = app
        .resolve::<GreetingServiceTag>()
        .expect("pre-built instance must resolve under its Tag");
    let out = proxy
        .greet(ServiceContext::new(), "world".to_string())
        .await
        .expect("invocation succeeds");
    assert_eq!(out, "hi, world");
}

// ---------------------------------------------------------------------------
// Task 4.2 (AD-9): App-constructed and FixtureBuilder-constructed instances
// of the same #[service] resolve identically — both sit over the same
// RuntimeBuilder/Injectable path, no second DI path.
// ---------------------------------------------------------------------------

use ego_service_sdk::di::ConfigValue;
use ego_testkit::{ServiceTestFixture, TestConfig};

#[service(version = "1.0.0")]
pub trait LimitService {
    #[operation]
    async fn limit(&self, ctx: ServiceContext) -> Result<u32, ServiceError>;
}

struct LimitServiceImpl {
    limit: ConfigValue<u32>,
}

impl Injectable for LimitServiceImpl {
    fn dependencies() -> Vec<DepKey> {
        vec![DepKey::Config(std::any::TypeId::of::<u32>(), std::any::type_name::<u32>())]
    }

    fn build(rt: &ego_service_sdk::runtime::RuntimeInner) -> Result<Self, RuntimeError> {
        Ok(Self {
            limit: rt.resolve_config::<u32>()?,
        })
    }
}

#[async_trait]
impl LimitService for LimitServiceImpl {
    async fn limit(&self, _ctx: ServiceContext) -> Result<u32, ServiceError> {
        Ok(*self.limit)
    }
}

#[tokio::test]
async fn app_and_fixture_builder_resolve_the_same_service_identically() {
    let app = App::builder()
        .config(Arc::new(10u32))
        .service::<LimitServiceImpl, LimitServiceTag>(|arc| arc)
        .build()
        .expect("build succeeds");
    let via_app = app
        .resolve::<LimitServiceTag>()
        .expect("resolves via App")
        .limit(ServiceContext::new())
        .await
        .expect("invocation succeeds");

    let fixture = ServiceTestFixture::builder()
        .config(TestConfig::new().with_value(10u32))
        .build();
    let via_fixture = fixture
        .service::<LimitServiceImpl>()
        .expect("constructs through the real Injectable::build path");

    assert_eq!(
        via_app, *via_fixture.limit,
        "App and FixtureBuilder must resolve the identical registered config value \
         through the same Injectable/RuntimeBuilder path (AD-9)"
    );
}

// ---------------------------------------------------------------------------
// Task 4.4 (AD-10, review G4): one equivalent application built two ways —
// directly via RuntimeBuilder, and via App::builder() — must resolve
// identical services under the same Tag ("optional migration, same
// contract" as a permanent, checkable test).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn runtime_builder_direct_and_app_builder_resolve_identical_services_under_the_same_tag() {
    let direct_instance: Arc<dyn GreetingService> = Arc::new(PreBuiltGreeter);
    let rt = ego_service_sdk::runtime::RuntimeBuilder::new()
        .with_service::<GreetingServiceTag>(direct_instance)
        .expect("direct registration succeeds")
        .build();
    let direct_out = rt
        .resolve::<GreetingServiceTag>()
        .expect("direct resolution succeeds")
        .greet(ServiceContext::new(), "world".to_string())
        .await
        .expect("direct invocation succeeds");

    let app_instance: Arc<dyn GreetingService> = Arc::new(PreBuiltGreeter);
    let app = App::builder()
        .service_instance::<GreetingServiceTag>(app_instance)
        .build()
        .expect("App::builder() registration succeeds");
    let via_app_out = app
        .resolve::<GreetingServiceTag>()
        .expect("App resolution succeeds")
        .greet(ServiceContext::new(), "world".to_string())
        .await
        .expect("App-path invocation succeeds");

    assert_eq!(
        direct_out, via_app_out,
        "RuntimeBuilder-direct and App::builder() composition must resolve \
         identical services under the same Tag (AD-10 same-contract proof)"
    );
}
