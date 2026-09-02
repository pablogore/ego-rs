//! Read-side wiring for the `UsersByTenant` projection — genuinely new
//! capability (not in CORE-018's original tasks.md), built on CORE-005's
//! real read-side/CQRS engine (`ego-domain::read_side::*` +
//! `ego-runtime::read_side::*`).
//!
//! **Not** `ego-service-sdk`'s `resolve_projection::<T>()` — that is an
//! unrelated DI dependency-kind. This module wires the actual tag-based,
//! batch, idempotent, resumable projection engine: `RegisterUser` writes
//! feed `ReadSideSink`, which inserts real `EventStreamElement`s into a
//! shared `InMemoryReadSideStore`; `ego-runtime`'s `TagSchedulerImpl` polls
//! that store, dedups, and delivers batches to `UsersByTenantHandler`,
//! which populates the queryable `UsersByTenantStore`.
//!
//! Construction (`ReadSideHandles::new`) is a plain sync call — safe from
//! `tests/pipeline.rs`'s non-`#[tokio::test]` tests. Only `spawn` requires
//! a running Tokio runtime.

pub mod projection;
pub mod store;

use std::sync::Arc;
use std::time::Duration;

use ego_domain::read_side::config::ReadSideConfig;
use ego_domain::read_side::dedup::DedupStore;
use ego_domain::read_side::event_tag::EventTag;
use ego_domain::read_side::offset::OffsetStore;
use ego_runtime::read_side::scheduler::{ProjectionSpec, ReadSideStopOutcome, TagSchedulerImpl};
use ego_runtime::read_side::ReadSideProjectionHandle;
use kitlogger::KITLogger;
use kitlogger_log_domain::Severity;

pub use projection::{TenantUsersView, UserSummary, UsersByTenantHandler, UsersByTenantStore};
pub use store::{
    FakeDurableDedupStore, FakeDurableOffsetStore, InMemoryDedupStore, InMemoryOffsetStore,
    ReadSideSink, SharedReadSideStore,
};

/// CORE-005 projection ID. Every `UsersByTenant`-relevant event is filed
/// under a tenant-scoped tag (`tenant_tag`) — one tag stream per tenant,
/// structurally isolating tenants at the store level instead of relying
/// solely on the handler filtering by `EventStreamElement::tenant_id` after
/// the fact (see `projection::UsersByTenantHandler`).
pub const PROJECTION_ID: &str = "users-by-tenant";
pub(crate) const PROJECTION_TAG: &str = "users-by-tenant";

/// The tag a given tenant's events are filed under. `store::ReadSideSink`
/// writes under it; the poll loop below discovers the full set of
/// currently-known tags each tick via `SharedReadSideStore::known_tags`
/// instead of assuming a single fixed tag.
pub(crate) fn tenant_tag(tenant_id: &str) -> EventTag {
    EventTag::new(format!("{PROJECTION_TAG}:{tenant_id}"))
}

/// The tenant encoded in a `tenant_tag`-shaped tag, i.e. the `{tenant_id}`
/// part of `"{PROJECTION_TAG}:{tenant_id}"`. Returns `None` for any tag not
/// produced by [`tenant_tag`].
///
/// This is a *secondary* cross-check, not the authoritative tenant: per the
/// `ReadSideStore::fetch` contract, the explicit `tenant` argument is always
/// the real tenant and the tag may only ever narrow to that same tenant (see
/// [`store::SharedReadSideStore::fetch`]). The scheduler pairs each tag with
/// this decoded tenant when starting the projection, so `fetch` receives the
/// authoritative tenant directly — `tenant_from_tag` merely re-derives it as
/// a defense-in-depth mismatch check.
pub(crate) fn tenant_from_tag(tag: &EventTag) -> Option<&str> {
    tag.value()
        .strip_prefix(&format!("{PROJECTION_TAG}:"))
        .filter(|t| !t.is_empty())
}

/// Poll interval for the background scheduler loop. `TagSchedulerImpl`
/// itself is not a persistent poller — `start_projection` processes one
/// batch per call and returns (see `crates/runtime/src/read_side/scheduler.rs`)
/// — so this module supplies the "actively polls" half of CORE-005's own
/// pull-based assumption (see the archived read-side-projections spec's
/// Assumptions section).
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Bound on how long `ReadSideRuntime::stop` waits for the poll loop to
/// drain before aborting it — see `ReadSideProjectionHandle::stop`.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);

/// The durable progress pair a projection resumes from — offset and dedup
/// together, because neither alone is resume state (PROD-014A IS-2/AD-8).
///
/// This type exists so the choice of progress storage is **stated** at the
/// composition root, never defaulted inside the thing that uses it. Before
/// it, `ReadSideHandles::new` constructed `InMemoryOffsetStore`/
/// `InMemoryDedupStore` with no parameter and no composition-visible
/// decision at all.
#[derive(Clone)]
pub struct ReadSideProgressStores {
    pub offset: Arc<dyn OffsetStore + Send + Sync>,
    pub dedup: Arc<dyn DedupStore + Send + Sync>,
}

impl ReadSideProgressStores {
    /// Volatile. First-class and unchanged for Dev and tests (PROD-014A
    /// IS-6/OOS-9); refused by `Profile::Production` once registered, which
    /// is the point.
    pub fn in_memory() -> Self {
        Self {
            offset: Arc::new(InMemoryOffsetStore::default()),
            dedup: Arc::new(InMemoryDedupStore::default()),
        }
    }

    /// See [`store::FakeDurableOffsetStore`] (PROD-014A AD-9). Never wire
    /// this into a deployment — it loses every offset on restart.
    pub fn fake_durable() -> Self {
        Self {
            offset: Arc::new(FakeDurableOffsetStore::default()),
            dedup: Arc::new(FakeDurableDedupStore::default()),
        }
    }
}

/// Not-yet-spawned read-side wiring, returned by `build_runtime`.
///
/// Splitting construction (`new`, sync) from `spawn` (requires a running
/// Tokio runtime) is what lets `build_runtime` stay a plain sync function
/// — `tests/pipeline.rs` calls it from ordinary `#[test]` fns with no
/// Tokio runtime available.
pub struct ReadSideHandles {
    /// Queryable handle to the read model (`GET /tenants/{tenant_id}/users`).
    pub query: UsersByTenantStore,
    store: SharedReadSideStore,
    handler: UsersByTenantHandler,
    progress: ReadSideProgressStores,
    /// Logs poll-loop failures instead of letting them vanish silently.
    /// `None` by default (e.g. in tests that don't care); `build_runtime`
    /// wires the real `Runtime`'s logger, mirroring
    /// `RegisterUserImpl`'s `Option<Arc<dyn Observability>>`.
    logger: Option<Arc<KITLogger>>,
}

impl ReadSideHandles {
    /// `store` must be the same `SharedReadSideStore` given to the
    /// `ReadSideSink` wired into `RegisterUserImpl` — otherwise the
    /// scheduler polls an empty store. `progress` is the durable progress
    /// pair this composition states at the composition root (PROD-014A
    /// IS-7/AD-8) — `ReadSideHandles` never constructs one itself.
    pub fn new(store: SharedReadSideStore, progress: ReadSideProgressStores) -> Self {
        let query = UsersByTenantStore::default();
        Self {
            handler: UsersByTenantHandler::new(query.clone()),
            query,
            store,
            progress,
            logger: None,
        }
    }

    /// Wires a logger so poll-loop failures are logged instead of silently
    /// swallowed (mirrors `RegisterUserImpl::with_read_side_sink`).
    pub fn with_logger(mut self, logger: Option<Arc<KITLogger>>) -> Self {
        self.logger = logger;
        self
    }

    /// Spawns the background polling loop that drives CORE-005's real
    /// `TagSchedulerImpl` via the framework's own `TagSchedulerImpl::spawn`
    /// spawn/stop lifecycle wrapper, configured through a `ProjectionSpec`
    /// (CORE-026 Phase 3 — this module no longer wires the stop channel or
    /// the poll-loop wrapper itself). Must be called from inside a running
    /// Tokio runtime.
    pub fn spawn(self) -> ReadSideRuntime {
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());
        let store = self.store;
        let handler = self.handler;
        let offset_store = self.progress.offset;
        let dedup_store = self.progress.dedup;
        let logger = self.logger;

        // Tags are discovered dynamically (one per tenant, see `tenant_tag`)
        // on every poll via the `tag_provider` closure the scheduler calls
        // fresh each iteration, instead of a fixed list decided once — a
        // tenant's tag only exists once its first event has been written.
        //
        // Each tag is paired with its own real tenant (decoded via
        // `tenant_from_tag`) so the scheduler threads the authoritative tenant
        // into `ReadSideStore::fetch` — the offset/dedup composite key
        // `(projection_id, tag, tenant)` stays unique per tag because the tag
        // itself already varies per tenant. Any tag that is not tenant-scoped
        // (no decodable tenant) is skipped: it is not part of this projection.
        //
        // `ProjectionSpec` groups the wiring and defaults the boilerplate: the
        // progress reporter defaults to `NoopProgressReporter`, so it no longer
        // appears here; only the interval and error logger are overridden.
        let store_for_provider = store.clone();
        let spec = ProjectionSpec::new(
            PROJECTION_ID,
            move || {
                store_for_provider
                    .known_tags()
                    .into_iter()
                    .filter_map(|tag| {
                        tenant_from_tag(&tag).map(|tenant| (tag.clone(), tenant.to_string()))
                    })
                    .collect()
            },
            handler,
            store,
            dedup_store,
            offset_store,
        )
        .interval(POLL_INTERVAL)
        .on_error(move |e| {
            if let Some(logger) = &logger {
                let _ = logger.log(Severity::Error, &format!("read_side poll failed: {e}"));
            }
        });

        let handle = scheduler.spawn(spec);

        ReadSideRuntime { handle }
    }
}

/// Handle to the spawned polling loop — a thin adapter over the framework's
/// `ReadSideProjectionHandle`, mapping a non-`Stopped` outcome to this app's
/// own `RuntimeInfraError::Teardown` so `Runtime::shutdown_async` (which
/// awaits this as a registered async teardown hook) can report the failure
/// instead of printing "shutdown complete" regardless (CORE-018 Finding
/// F-02, extended to also report a stop that timed out).
pub struct ReadSideRuntime {
    handle: ReadSideProjectionHandle,
}

impl ReadSideRuntime {
    /// Signals the loop to stop and awaits the in-flight batch's drain,
    /// bounded by [`SHUTDOWN_DEADLINE`] — see `ReadSideProjectionHandle::stop`.
    pub async fn stop(self) -> Result<(), ego_service_sdk::RuntimeInfraError> {
        match self.handle.stop(SHUTDOWN_DEADLINE).await {
            ReadSideStopOutcome::Stopped => Ok(()),
            ReadSideStopOutcome::Panicked(join_err) => {
                Err(ego_service_sdk::RuntimeInfraError::Teardown {
                    reason: format!("read-side scheduler task panicked: {join_err}"),
                })
            }
            ReadSideStopOutcome::TimedOut => Err(ego_service_sdk::RuntimeInfraError::Teardown {
                reason: format!(
                    "read-side scheduler task did not stop within {SHUTDOWN_DEADLINE:?} and was aborted"
                ),
            }),
        }
    }
}
