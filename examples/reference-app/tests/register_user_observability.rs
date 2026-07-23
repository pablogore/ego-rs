//! CORE-018 Phase 8 — observability test-double assertions.
//!
//! Satisfies reference-service spec "Success and failure are observed".
//! Guard denials (authorize/tenant_scoped) are already recorded for free by
//! CORE-012A's `RuntimeBuilder::with_observability` macro-guard wiring — only
//! the two business-outcome paths (success, partial-failure) need this
//! file's coverage of `RegisterUserImpl`'s own explicit `obs.trace()` calls.

mod support;

use std::sync::{Arc, Mutex};

use ego_domain::{Level, Observability, SemanticEvent};
use ego_testkit::{PrincipalBuilder, ServiceTestFixture};
use reference_app::application::{RegisterInput, RegisterUser, RegisterUserTag};
use support::make_register_user;

/// Test-double `Observability` implementor capturing every `trace()` call.
#[derive(Default)]
struct RecordingObservability {
    events: Mutex<Vec<SemanticEvent>>,
}

impl RecordingObservability {
    fn new() -> Self {
        Self::default()
    }

    fn event_names(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.event_name.clone())
            .collect()
    }
}

impl Observability for RecordingObservability {
    fn trace(&self, event: SemanticEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn metric(&self, _name: &str, _value: f64) {}
    fn log(&self, _level: Level, _message: &str) {}
}

fn make_service() -> Arc<dyn RegisterUser> {
    make_register_user(None)
}

fn make_service_with_obs(obs: Arc<dyn Observability>) -> Arc<dyn RegisterUser> {
    make_register_user(Some(obs))
}

// TASK-022: success records >= 1 event via RegisterUserImpl's explicit trace.
#[tokio::test]
async fn successful_registration_records_at_least_one_event() {
    let observability = Arc::new(RecordingObservability::new());
    let service = make_service_with_obs(observability.clone());

    let principal = PrincipalBuilder::new().tenant("tenant-a").build();
    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(service)
        .expect("registration succeeds")
        .principal(principal)
        .with_observability(observability.clone())
        .build();

    let proxy = fixture
        .resolve::<RegisterUserTag>()
        .expect("registered tag resolves");

    let ctx = fixture.context().with_tenant_id("tenant-a");
    let input = RegisterInput {
        user_id: "user-1".to_string(),
        email: "user@example.com".to_string(),
        tenant_id: "tenant-a".to_string(),
        org_name: "Acme".to_string(),
    };

    let result = proxy.register(ctx, input).await;
    assert!(
        result.is_ok(),
        "expected registration to succeed: {result:?}"
    );

    assert!(
        !observability.event_names().is_empty(),
        "expected at least one recorded event on success"
    );
    assert!(observability
        .event_names()
        .contains(&"register_user.success".to_string()));
}

// TASK-022: partial failure records >= 1 event via RegisterUserImpl's explicit trace.
#[tokio::test]
async fn partial_failure_records_at_least_one_event() {
    let observability = Arc::new(RecordingObservability::new());
    let service = make_service_with_obs(observability.clone());

    let principal = PrincipalBuilder::new().tenant("tenant-a").build();
    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(service)
        .expect("registration succeeds")
        .principal(principal)
        .with_observability(observability.clone())
        .build();

    let proxy = fixture
        .resolve::<RegisterUserTag>()
        .expect("registered tag resolves");

    let input = RegisterInput {
        user_id: "user-1".to_string(),
        email: String::new(), // triggers the User-write failure
        tenant_id: "tenant-a".to_string(),
        org_name: "Acme".to_string(),
    };

    let ctx = fixture.context().with_tenant_id("tenant-a");
    let result = proxy.register(ctx, input).await;
    assert!(result.is_err(), "expected partial failure");

    assert!(
        !observability.event_names().is_empty(),
        "expected at least one recorded event on partial failure"
    );
    assert!(observability
        .event_names()
        .contains(&"register_user.partial_failure".to_string()));
}

// TASK-022: guard denials are already recorded by CORE-012A's macro-guard
// wiring — no explicit obs.trace() call needed inside RegisterUserImpl for
// these two cases.
#[tokio::test]
async fn guard_denial_is_recorded_without_any_explicit_trace_call() {
    use ego_testkit::ScriptedAuthorizationProvider;

    let observability = Arc::new(RecordingObservability::new());
    let service = make_service();

    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(service)
        .expect("registration succeeds")
        .authorization(Arc::new(ScriptedAuthorizationProvider::deny_all()))
        .with_observability(observability.clone())
        .build();

    let proxy = fixture
        .resolve::<RegisterUserTag>()
        .expect("registered tag resolves");

    let input = RegisterInput {
        user_id: "user-1".to_string(),
        email: "user@example.com".to_string(),
        tenant_id: "tenant-a".to_string(),
        org_name: "Acme".to_string(),
    };

    let result = proxy.register(fixture.context(), input).await;
    assert!(result.is_err(), "expected authorization denial");

    assert!(
        !observability.event_names().is_empty(),
        "expected the macro-guard wiring to record the denial for free"
    );
}
