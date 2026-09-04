//! Single integration-test binary for `reference-app` (issue #305).
//!
//! Every file under `tests/reference_app/` was previously its own
//! `tests/*.rs` integration-test target — each one a separate binary that
//! `cargo test` had to link and execute independently. None of them mutate
//! process-wide state (no env vars, no fixed ports, no global statics, no
//! CWD changes, no global tracing/OTel providers — audited file by file
//! before this merge), so nothing here requires a process boundary from
//! anything else. They are folded into modules of one binary instead,
//! cutting linking, loading, and (on platforms that pay it) per-binary
//! startup validation down to a single instance instead of one per former
//! file.
//!
//! `#[path = ...]` is required on every `mod` below: this file is the crate
//! root of the `reference_app` test binary, so plain `mod foo;` would look
//! for `foo.rs` next to this file (i.e. directly under `tests/`) — which
//! would make Cargo's test-target autodiscovery pick it up as its own
//! separate binary again, defeating the point. Routing each module through
//! `tests/reference_app/` keeps them out of that autodiscovered directory.
//!
//! Content is unchanged from the original files — only the module boundary
//! moved. If a future test genuinely needs process isolation (e.g. it must
//! crash the whole process, or mutate something truly global), give it back
//! its own `tests/<name>.rs` file and say why in a comment there.

#[path = "reference_app/support.rs"]
mod support;

#[path = "reference_app/effects_e2e.rs"]
mod effects_e2e;
#[path = "reference_app/entity_event_stores_profile.rs"]
mod entity_event_stores_profile;
#[path = "reference_app/external_data_provider_lint.rs"]
mod external_data_provider_lint;
#[path = "reference_app/http_idempotency_span.rs"]
mod http_idempotency_span;
#[path = "reference_app/http_operation_key_carriage.rs"]
mod http_operation_key_carriage;
#[path = "reference_app/http_replay_and_conflict.rs"]
mod http_replay_and_conflict;
#[path = "reference_app/http_route.rs"]
mod http_route;
#[path = "reference_app/idempotency_wiring.rs"]
mod idempotency_wiring;
#[path = "reference_app/idempotent_marker_completeness.rs"]
mod idempotent_marker_completeness;
#[path = "reference_app/ingress_trace_context.rs"]
mod ingress_trace_context;
#[path = "reference_app/ingress_trace_wiring.rs"]
mod ingress_trace_wiring;
#[path = "reference_app/metrics_reach_one_backend.rs"]
mod metrics_reach_one_backend;
#[path = "reference_app/outbound_trace_propagation.rs"]
mod outbound_trace_propagation;
#[path = "reference_app/pipeline.rs"]
mod pipeline;
#[path = "reference_app/production_profile_guard.rs"]
mod production_profile_guard;
#[path = "reference_app/providers_e2e.rs"]
mod providers_e2e;
#[path = "reference_app/read_side_error_logging.rs"]
mod read_side_error_logging;
#[path = "reference_app/register_user_guard_chain.rs"]
mod register_user_guard_chain;
#[path = "reference_app/register_user_multi_aggregate_recovery.rs"]
mod register_user_multi_aggregate_recovery;
#[path = "reference_app/register_user_observability.rs"]
mod register_user_observability;
#[path = "reference_app/register_user_partial_failure.rs"]
mod register_user_partial_failure;
#[path = "reference_app/register_user_tenant_guard.rs"]
mod register_user_tenant_guard;
#[path = "reference_app/stoolap_restart_persistence.rs"]
mod stoolap_restart_persistence;
#[path = "reference_app/tenant_org_entity.rs"]
mod tenant_org_entity;
#[path = "reference_app/user_entity.rs"]
mod user_entity;
#[path = "reference_app/users_by_tenant_projection.rs"]
mod users_by_tenant_projection;
