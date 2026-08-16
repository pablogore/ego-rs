//! **Guarantee:** one adapter, wired the way a host wires it, carries both the
//! SDK's metrics and an entity runtime's to the same backend.
//!
//! **Layers traversed:** `RuntimeBuilder::with_observability` and
//! `EntityRuntimeBuilder::with_observability` → `OtlpMetrics` →
//! `SdkMeterProvider` → an in-memory reader.
//!
//! # The failure this exists to catch
//!
//! There is **no retroactive wiring**. A host registers an entity runtime as
//! `Arc<EntityRuntime<_>>`, `EntityRuntime::with_observability` consumes `self`,
//! and `Runtime::observability()` only exists once the runtime is already built.
//! So an adapter can only reach entity actors if the host hands it to *both*
//! builders before either is finished.
//!
//! Wire only the `RuntimeBuilder` and the result looks healthy: reservation and
//! purge metrics flow, dashboards populate, nothing errors — and
//! `idempotency.receipt.outcome` is silently absent, because the actors that
//! produce it were never given a sink. That is the shape this file refuses to
//! let pass, and it is why the assertion is about **both** signals arriving
//! through **one** provider rather than about either alone.
//!
//! # What is asserted, and what is not
//!
//! Translation and topology. The eight AD-10 signals prove their own contracts
//! where they are emitted — names, kinds, closed value sets, redaction — and
//! repeating that here would restate those tests against a slower fixture. What
//! only this file can show is that the two independently-wired halves of the
//! system reach one backend with their kinds and dimensions intact.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::Request;
use ego_domain::operation::{OperationFingerprint, OperationIdentity, OperationKey};
use ego_domain::Observability;
use ego_infrastructure::metrics_otlp::OtlpMetrics;
use ego_transport::state::AppState;
use ego_transport::OperationKeyExtractor;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use reference_app::domain::user::{UserCommand, UserEntity, UserRegistered, UserState};
use reference_app::AppConfig;

const ENTITY_TYPE: &str = "User";
const ENTITY_ID: &str = "e1";

/// One exported series: name, the arm it landed in, and its dimensions.
///
/// The arm is the evidence for the kind. A counter arrives as a `Sum`, a gauge
/// as a `Gauge`, a histogram as a `Histogram` — so reading which one the SDK put
/// it in is what proves the adapter translated rather than guessed.
type Series = (String, &'static str, Vec<(String, String)>);

fn exported(exporter: &InMemoryMetricExporter) -> Vec<Series> {
    let mut out = Vec::new();
    for resource_metric in exporter
        .get_finished_metrics()
        .expect("the in-memory exporter is readable")
    {
        for scope in resource_metric.scope_metrics() {
            for metric in scope.metrics() {
                let name = metric.name().to_string();
                if let AggregatedMetrics::F64(data) = metric.data() {
                    match data {
                        MetricData::Sum(sum) => {
                            for point in sum.data_points() {
                                out.push((name.clone(), "sum", attrs(point.attributes())));
                            }
                        }
                        MetricData::Gauge(gauge) => {
                            for point in gauge.data_points() {
                                out.push((name.clone(), "gauge", attrs(point.attributes())));
                            }
                        }
                        MetricData::Histogram(hist) => {
                            for point in hist.data_points() {
                                out.push((name.clone(), "histogram", attrs(point.attributes())));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    out
}

/// The dimensions of one exported point, in a canonical order.
///
/// Sorted, because an OpenTelemetry attribute set is identified by its members
/// and not their sequence — the SDK returns them in its own order, which is not
/// the emitter's. Comparing positionally would make this test assert an ordering
/// nobody promises, and it would fail for the right reasons at the wrong time.
fn attrs<'a>(kvs: impl Iterator<Item = &'a opentelemetry::KeyValue>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = kvs
        .map(|kv| (kv.key.to_string(), kv.value.to_string()))
        .collect();
    out.sort();
    out
}

fn series_named<'a>(exported: &'a [Series], name: &str) -> Vec<&'a Series> {
    exported.iter().filter(|(n, ..)| n == name).collect()
}

/// Both halves of the system report to the one adapter the host built.
#[tokio::test]
async fn the_sdk_and_an_entity_runtime_reach_the_same_backend() {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let metrics = Arc::new(OtlpMetrics::from_provider(provider));

    // ONE sink. Cloned into both builders, before either finishes — the only
    // order that works, since neither can be given one afterwards.
    let observability: Arc<dyn Observability> = metrics.clone();

    // --- BOTH halves, from the one production entry point ------------------
    // `build_runtime_observed` is what a host calls. It hands this exact `Arc`
    // to `App::builder().observability(...)` for the SDK's own signals and,
    // through `compose_entity_runtimes`, to every entity runtime. Building
    // either half here instead would prove only that a fixture can be written
    // correctly.
    let built = reference_app::build_runtime_observed(&AppConfig::default(), Some(observability))
        .expect("the reference app builds");

    // --- provoke the SDK signal -------------------------------------------
    // Through the real HTTP extractor, over the state the built app resolves.
    // An *invalid* key rather than a missing one: the reference app declares
    // `Compatibility`, which admits a missing key and would count nothing.
    provoke_a_key_rejection(&built).await;

    // --- provoke the entity signal ----------------------------------------
    send_a_registration(&built).await;

    metrics.shutdown().expect("the provider shuts down");
    let exported = exported(&exporter);

    // --- both arrived, through one provider --------------------------------
    let rejected = series_named(&exported, "idempotency.key.rejected");
    assert_eq!(
        rejected
            .iter()
            .map(|(_, arm, a)| (*arm, a.clone()))
            .collect::<Vec<_>>(),
        vec![(
            "sum",
            vec![
                ("carrier".to_string(), "http:Idempotency-Key".to_string()),
                ("reason".to_string(), "invalid".to_string()),
            ]
        )],
        "the SDK-side signal must reach the backend as a counter carrying both \
         dimensions: {exported:?}"
    );

    let receipt = series_named(&exported, "idempotency.receipt.outcome");
    assert_eq!(
        receipt
            .iter()
            .map(|(_, arm, a)| (*arm, a.clone()))
            .collect::<Vec<_>>(),
        vec![(
            "sum",
            vec![
                ("aggregate_type".to_string(), ENTITY_TYPE.to_string()),
                ("outcome".to_string(), "confirmed".to_string()),
            ]
        )],
        "the entity runtime's signal must reach the SAME backend, with its \
         dimensions intact — its absence here is exactly what wiring only the \
         RuntimeBuilder produces: {exported:?}"
    );
}

/// Wiring only the SDK leaves the entity half dark, and nothing reports it.
///
/// The negative control, and the reason the test above is not vacuous. Without
/// it, an implementation that wired both halves to *separate* providers — or
/// that reached entity actors by some accident of construction — would pass the
/// positive assertion and prove nothing about the topology.
///
/// It also documents the failure precisely: the SDK's signal is present and
/// healthy, so nothing anywhere says the other half is missing.
#[tokio::test]
async fn wiring_only_the_sdk_silently_loses_the_entity_signal() {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let metrics = Arc::new(OtlpMetrics::from_provider(provider));
    let observability: Arc<dyn Observability> = metrics.clone();

    // The mistake: the host wires the SDK and composes the entity half without
    // the sink. Expressed by calling the production functions separately —
    // which is exactly what a host does when it forgets `build_runtime_observed`
    // and assembles the two halves by hand.
    let built = reference_app::build_runtime_observed(&AppConfig::default(), Some(observability))
        .expect("the reference app builds");
    let unobserved = reference_app::compose_entity_runtimes(None);

    provoke_a_key_rejection(&built).await;
    send_a_registration_to(&unobserved).await;

    metrics.shutdown().expect("the provider shuts down");
    let exported = exported(&exporter);

    assert!(
        !series_named(&exported, "idempotency.key.rejected").is_empty(),
        "the SDK half is wired, so it reports normally — which is what makes the \
         gap invisible in production: {exported:?}"
    );
    assert!(
        series_named(&exported, "idempotency.receipt.outcome").is_empty(),
        "an entity runtime with no sink emits nothing, and nothing anywhere says \
         so: {exported:?}"
    );
}

/// Drives the real HTTP extractor against the built app's own state.
///
/// A whitespace-only `Idempotency-Key` is refused under **either** mode, which
/// matters because the reference app declares `Compatibility` — a *missing* key
/// is admitted there and would count nothing.
async fn provoke_a_key_rejection(built: &reference_app::BuiltRuntime) {
    let state = AppState::new(built.app.resolver(), built.authn.clone());
    let (mut parts, _) = Request::builder()
        .uri("/probe")
        .header("Idempotency-Key", "   ")
        .body(())
        .expect("a valid request")
        .into_parts();
    let refused = OperationKeyExtractor::from_request_parts(&mut parts, &state).await;
    assert!(
        refused.is_err(),
        "a malformed key is refused under every mode, which is what counts"
    );
}

/// Registers a user through the entity runtime the built app resolved.
///
/// Resolved rather than constructed: `App::resolve_entity` returns the runtime
/// `build_runtime_observed` registered, so the actor this reaches is the one a
/// deployment would have.
async fn send_a_registration(built: &reference_app::BuiltRuntime) {
    let runtime = built
        .app
        .resolve_entity::<UserEntity>()
        .expect("the reference app registers the User aggregate");
    let entity_ref = runtime
        .entity_ref::<UserCommand, UserState>(ENTITY_TYPE, ENTITY_ID, Arc::new(UserEntity))
        .expect("an entity ref is obtainable");
    drive(entity_ref).await;
}

/// The same registration, against runtimes composed without a sink.
async fn send_a_registration_to(runtimes: &reference_app::ObservedEntityRuntimes) {
    let entity_ref = runtimes
        .user
        .entity_ref::<UserCommand, UserState>(ENTITY_TYPE, ENTITY_ID, Arc::new(UserEntity))
        .expect("an entity ref is obtainable");
    drive(entity_ref).await;
}

/// Sends one `Register` carrying an operation identity — the only shape that
/// makes the actor write a receipt, and therefore report an outcome.
async fn drive(entity_ref: impl EntityRef<Command = UserCommand>) {
    let sent: Result<
        persistent_entity::persistent_entity::CommandResult<UserRegistered, UserState>,
        EntityError,
    > = entity_ref
        .send_command(
            UserCommand::Register {
                user_id: ENTITY_ID.to_string(),
                email: "probe@example.test".to_string(),
                tenant_id: "tenant-a".to_string(),
            },
            CommandContext::new(ENTITY_TYPE.to_string()).carrying(Some(OperationIdentity::new(
                OperationKey::parse("op-acceptance").expect("a valid key"),
                OperationFingerprint::new("fp"),
            ))),
        )
        .await;
    sent.expect("the registration succeeds");
}
