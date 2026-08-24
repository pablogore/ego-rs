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
// Task 2.2/2.4: `.service_with_tag::<S, Tag>()` (CORE-028 Stage 2B rename) —
// real Injectable + Tag + Resolvable
// ---------------------------------------------------------------------------

#[service(version = "1.0.0")]
pub trait GreetingService {
    #[operation]
    async fn greet(&self, ctx: ServiceContext, name: String) -> Result<String, ServiceError>;
}

/// A concrete `GreetingService` impl that is ALSO `Injectable`, hand-rolled
/// (mirrors testkit's `HandRolledService` pattern) so `.service_with_tag::<S,
/// Tag>()` can construct it through the real `Injectable::build` path rather
/// than requiring a pre-built instance.
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
        .idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .adapter(Arc::new(GreeterAdapter("hello".to_string())))
        .service_with_tag::<GreetingServiceImpl, GreetingServiceTag>(|arc| arc)
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
        .idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .service_with_tag::<GreetingServiceImpl, GreetingServiceTag>(|arc| arc)
        .build();

    match result {
        Ok(_) => panic!("expected build to fail on a missing adapter dependency"),
        Err(ego_service_sdk::app::CompositionError::Validation(
            RuntimeError::DependencyNotFound {
                type_name,
                service_name,
                ..
            },
        )) => {
            assert_eq!(type_name, std::any::type_name::<GreeterAdapter>());
            assert_eq!(
                service_name,
                Some(std::any::type_name::<GreetingServiceImpl>())
            );
        }
        Err(other) => {
            panic!("expected Validation(DependencyNotFound) naming type+requester, got {other:?}")
        }
    }
}

/// Review F3: a hand-rolled `Injectable` whose `dependencies()` is
/// incomplete (doesn't declare the adapter it actually needs) — so
/// `Injectable::validate`'s presence check never catches the missing
/// dependency — but whose `build()` tries to resolve it anyway and fails.
/// Before the F3 fix, this `DependencyNotFound` reached `AppBuilder::service`
/// with `service_name: None` (only `validate()`'s error path was
/// attributed); this proves `build()`'s path is now attributed identically.
struct MisdeclaredGreetingServiceImpl {
    adapter: AdapterRef<GreeterAdapter>,
}

impl Injectable for MisdeclaredGreetingServiceImpl {
    fn dependencies() -> Vec<DepKey> {
        Vec::new() // intentionally incomplete — doesn't declare GreeterAdapter
    }

    fn build(rt: &ego_service_sdk::runtime::RuntimeInner) -> Result<Self, RuntimeError> {
        Ok(Self {
            adapter: rt.resolve_adapter::<GreeterAdapter>()?,
        })
    }
}

#[async_trait]
impl GreetingService for MisdeclaredGreetingServiceImpl {
    async fn greet(&self, _ctx: ServiceContext, name: String) -> Result<String, ServiceError> {
        Ok(format!("{}, {name}", self.adapter.0))
    }
}

#[test]
fn build_time_dependency_failure_is_attributed_even_when_dependencies_omit_it() {
    let result = App::builder()
        .idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .service_with_tag::<MisdeclaredGreetingServiceImpl, GreetingServiceTag>(|arc| arc)
        .build();

    match result {
        Ok(_) => panic!("expected build to fail — GreeterAdapter was never registered"),
        Err(ego_service_sdk::app::CompositionError::Validation(
            RuntimeError::DependencyNotFound {
                type_name,
                service_name,
                ..
            },
        )) => {
            assert_eq!(type_name, std::any::type_name::<GreeterAdapter>());
            assert_eq!(
                service_name,
                Some(std::any::type_name::<MisdeclaredGreetingServiceImpl>()),
                "a DependencyNotFound surfacing only from build() (not caught by validate()'s \
                 incomplete dependencies() list) must still name the requesting service"
            );
        }
        Err(other) => {
            panic!("expected Validation(DependencyNotFound) naming type+requester, got {other:?}")
        }
    }
}

// DX follow-up (Part A/C): resolving a service tag that was never registered
// surfaces `ServiceNotFound` naming the missing tag (not a bare fieldless
// variant), and its message points at the fix method — the type_name from
// Part A flowing through end to end.
#[test]
fn resolving_an_unregistered_service_names_the_missing_tag() {
    let app = App::builder()
        .idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .build()
        .expect("build succeeds");

    match app.resolve::<GreetingServiceTag>() {
        Err(RuntimeError::ServiceNotFound { type_name, .. }) => {
            assert!(
                type_name.contains("GreetingServiceTag"),
                "ServiceNotFound must name the missing tag, got {type_name}"
            );
        }
        Err(other) => panic!("expected ServiceNotFound naming the tag, got {other:?}"),
        Ok(_) => panic!("expected ServiceNotFound, but an unregistered tag resolved"),
    }
}

// ---------------------------------------------------------------------------
// CORE-028 Stage 2B (task 3.1): `.service::<S>()` — single-type-parameter
// registration for a macro-linked (`impl_of`) service struct. No Tag
// parameter, no coercion closure; must resolve identically to the
// two-generic form above (spec.md "A macro-linked service registers with a
// single type parameter and no closure").
// ---------------------------------------------------------------------------

#[service(impl_of = GreetingService)]
struct LinkedGreetingServiceImpl {
    adapter: AdapterRef<GreeterAdapter>,
}

#[async_trait]
impl GreetingService for LinkedGreetingServiceImpl {
    async fn greet(&self, _ctx: ServiceContext, name: String) -> Result<String, ServiceError> {
        Ok(format!("{}, {name}", self.adapter.0))
    }
}

#[tokio::test]
async fn macro_linked_service_registers_with_single_type_parameter_and_resolves_identically() {
    let app = App::builder()
        .idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .adapter(Arc::new(GreeterAdapter("hello".to_string())))
        .service::<LinkedGreetingServiceImpl>()
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
        .idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
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
        vec![DepKey::Config(
            std::any::TypeId::of::<u32>(),
            std::any::type_name::<u32>(),
        )]
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
        .idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .config(Arc::new(10u32))
        .service_with_tag::<LimitServiceImpl, LimitServiceTag>(|arc| arc)
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
        .with_idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
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
        .idempotency_enforcement_mode(ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility)
        .service_instance::<GreetingServiceTag>(app_instance)
        .build()
        .expect("App::builder()
            .idempotency_enforcement_mode(ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility) registration succeeds");
    let via_app_out = app
        .resolve::<GreetingServiceTag>()
        .expect("App resolution succeeds")
        .greet(ServiceContext::new(), "world".to_string())
        .await
        .expect("App-path invocation succeeds");

    assert_eq!(
        direct_out, via_app_out,
        "RuntimeBuilder-direct and App::builder()
            .idempotency_enforcement_mode(ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility) composition must resolve \
         identical services under the same Tag (AD-10 same-contract proof)"
    );
}

// ---------------------------------------------------------------------------
// Final API Consistency Cleanup: `pending_error` first-error-wins.
// ---------------------------------------------------------------------------

/// Once `AppBuilder` has latched an error, no later registration overrides
/// it — including the three service-registration methods, which previously
/// kept pushing registrars onto `service_registrars` even after an earlier
/// call had already condemned the build to fail.
#[test]
fn once_latched_a_pending_error_survives_every_service_registration_method() {
    let instance: Arc<dyn GreetingService> = Arc::new(PreBuiltGreeter);
    let result = App::builder()
        .idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .adapter(Arc::new(GreeterAdapter("hello".to_string())))
        .adapter(Arc::new(GreeterAdapter("world".to_string()))) // latches DuplicateAdapter
        .service_with_tag::<GreetingServiceImpl, GreetingServiceTag>(|arc| arc)
        .service::<LinkedGreetingServiceImpl>()
        .service_instance::<GreetingServiceTag>(instance)
        .build();

    match result {
        Err(ego_service_sdk::app::CompositionError::DuplicateAdapter { type_name }) => {
            assert_eq!(type_name, std::any::type_name::<GreeterAdapter>());
        }
        Ok(_) => panic!("expected the first-latched DuplicateAdapter, got Ok"),
        Err(other) => panic!("expected the first-latched DuplicateAdapter, got {other:?}"),
    }
}
