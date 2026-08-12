//! PROD-012 B6.5 — the HTTP end of the `OperationKey` extraction contract.
//!
//! Three pieces existed before this and nothing joined them: the header
//! carrier, the shared policy table, and the runtime's configured mode. So the
//! guarantee the SDK enforces internally could be disabled simply by deploying
//! a transport that never asked. These tests are about the join.
//!
//! Every case runs the real extractor against a real `AppState` built from a
//! real `RuntimeBuilder`, so what is exercised is the wiring rather than a
//! restatement of `resolve_operation_key`'s own unit tests.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::Request;
use ego_security_sdk::authentication::AuthenticationProvider;
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, RuntimeBuilder};
use ego_transport::state::AppState;
use ego_transport::{OperationKeyExtractor, TransportError};

/// The extractor never authenticates, so the provider only has to exist.
struct UnusedAuthn;
impl AuthenticationProvider for UnusedAuthn {
    fn authenticate(
        &self,
        _c: &ego_security_sdk::credential::Credential,
    ) -> Result<ego_security_sdk::context::SecurityContext, ego_security_sdk::AuthenticationError>
    {
        unimplemented!("the operation-key boundary does not authenticate");
    }
}

/// A state whose runtime was built under `mode` — and only under `mode`. The
/// extractor has no configuration of its own, so this is the single input that
/// decides its behaviour.
fn state_under(mode: IdempotencyEnforcementMode) -> AppState {
    let mut builder = RuntimeBuilder::new().with_idempotency_enforcement_mode(mode);
    if matches!(mode, IdempotencyEnforcementMode::MandatoryKey) {
        // The builder refuses `MandatoryKey` with nowhere to reserve — a runtime
        // that promises every mutating operation carries a key and cannot
        // reserve one could not keep the promise. Registering a store is what
        // makes the promise buildable, not what makes the header required.
        builder = builder
            .with_operation_reservation_store(Arc::new(
                ego_testkit::InMemoryOperationReservationStore::new(Arc::new(
                    ego_domain::time::SystemClock,
                )),
            ))
            .with_reservation_owner_id(ego_domain::operation::OwnerId::new("under-test"))
            .with_reservation_lease_duration(std::time::Duration::from_secs(30));
    }
    AppState::new(builder.build().resolver(), Arc::new(UnusedAuthn))
}

/// Runs the real extractor over a request carrying `header`, if any.
async fn extract(
    mode: IdempotencyEnforcementMode,
    header: Option<&str>,
) -> Result<OperationKeyExtractor, TransportError> {
    let mut request = Request::builder().uri("/register");
    if let Some(value) = header {
        request = request.header("Idempotency-Key", value);
    }
    let (mut parts, _) = request.body(()).expect("a valid request").into_parts();
    let state = state_under(mode);
    OperationKeyExtractor::from_request_parts(&mut parts, &state).await
}

// ---------------------------------------------------------------------------
// The mode the extractor applies is the one the runtime was built under
// ---------------------------------------------------------------------------

/// The runtime reports the policy it was built under, whichever it is. The
/// whole boundary reads this value, so a builder that validated one policy and
/// a runtime that reported another would let a deployment enforce something
/// nobody configured.
///
/// **The default is the fail-closed one.** `MandatoryKey`, not `Compatibility`
/// — a caller who never thought about idempotency must not silently get none.
/// Pinned here because the extractor's behaviour on an unconfigured deployment
/// follows from it, and because it is the opposite of what "default" usually
/// suggests.
#[test]
fn the_runtime_reports_the_mode_it_was_built_under() {
    assert_eq!(
        IdempotencyEnforcementMode::default(),
        IdempotencyEnforcementMode::MandatoryKey,
        "the default fails closed; a runtime that reported otherwise would admit \
         keyless requests nobody agreed to admit"
    );

    // Built under the default, which is only buildable with somewhere to
    // reserve — the builder refuses the combination that could not keep its
    // promise, and that refusal is why this state needs a store at all.
    assert_eq!(
        state_under(IdempotencyEnforcementMode::MandatoryKey)
            .runtime
            .idempotency_enforcement_mode(),
        IdempotencyEnforcementMode::MandatoryKey
    );

    assert_eq!(
        state_under(IdempotencyEnforcementMode::Compatibility)
            .runtime
            .idempotency_enforcement_mode(),
        IdempotencyEnforcementMode::Compatibility,
        "an explicitly permissive deployment is reported as permissive, not \
         normalised toward the default"
    );
}

// ---------------------------------------------------------------------------
// A key that arrives is carried, unchanged
// ---------------------------------------------------------------------------

/// The value the client sent is the value that reaches the context — parsed,
/// never regenerated, normalised or reconstructed. This is the assertion a
/// handler that minted its own key would fail.
#[tokio::test]
async fn a_valid_header_is_carried_through_unchanged() {
    for mode in [
        IdempotencyEnforcementMode::Compatibility,
        IdempotencyEnforcementMode::MandatoryKey,
    ] {
        let OperationKeyExtractor(key) = extract(mode, Some("op-from-the-client"))
            .await
            .expect("a well-formed key is admitted under every mode");

        assert_eq!(
            key.expect("a present key resolves to Some").as_str(),
            "op-from-the-client",
            "{mode:?}: the client's key, byte for byte"
        );
    }
}

/// HTTP/2 lowercases every header name on the wire, so this is what a real
/// client sends. Covered at the carrier already; repeated here because the
/// wiring is what would drop it.
#[tokio::test]
async fn the_header_is_found_in_the_case_a_real_client_sends() {
    let mut request = Request::builder().uri("/register");
    request = request.header("idempotency-key", "op-lowercased");
    let (mut parts, _) = request.body(()).expect("a valid request").into_parts();
    let state = state_under(IdempotencyEnforcementMode::MandatoryKey);

    let OperationKeyExtractor(key) = OperationKeyExtractor::from_request_parts(&mut parts, &state)
        .await
        .expect("a lowercased header name is the same header");

    assert_eq!(key.expect("resolves").as_str(), "op-lowercased");
}

// ---------------------------------------------------------------------------
// What each mode does about a key that never arrived
// ---------------------------------------------------------------------------

/// The point of the whole boundary. Under the enforcing mode a request with no
/// key is refused **here** — the handler is never reached, so the operation
/// cannot run unreserved.
#[tokio::test]
async fn a_missing_key_is_refused_before_the_operation_under_mandatory_key() {
    let result = extract(IdempotencyEnforcementMode::MandatoryKey, None).await;

    assert!(
        matches!(result, Err(TransportError::BadRequest)),
        "a runtime that requires a key must not dispatch a request without one"
    );
}

/// And the mirror: a deployment that declared itself still in transition
/// dispatches normally. `None` is the answer, not an error — the request simply
/// carries no operation identity.
#[tokio::test]
async fn a_missing_key_is_admitted_under_compatibility() {
    let OperationKeyExtractor(key) = extract(IdempotencyEnforcementMode::Compatibility, None)
        .await
        .expect("compatibility exists precisely to admit this");

    assert!(key.is_none());
}

// ---------------------------------------------------------------------------
// Compatibility tolerates absence — never malformed input
// ---------------------------------------------------------------------------

/// The distinction the whole policy rests on. `Compatibility` admits a request
/// that carried *no* key; it does not admit one that carried an unusable one.
/// Collapsing the two would let malformed input silently disable the guarantee
/// for exactly the requests most likely to come from a broken client.
#[tokio::test]
async fn an_invalid_key_is_refused_under_both_modes() {
    for mode in [
        IdempotencyEnforcementMode::Compatibility,
        IdempotencyEnforcementMode::MandatoryKey,
    ] {
        let result = extract(mode, Some("   ")).await;

        assert!(
            matches!(result, Err(TransportError::BadRequest)),
            "{mode:?}: a whitespace-only key is supplied-but-unusable, which no \
             mode admits — compatibility tolerates absence, not garbage"
        );
    }
}
