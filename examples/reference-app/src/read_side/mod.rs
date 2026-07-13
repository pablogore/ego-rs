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
use ego_domain::read_side::event_tag::EventTag;
use ego_domain::read_side::progress::NoopProgressReporter;
use ego_runtime::read_side::scheduler::TagSchedulerImpl;
use ego_runtime::read_side::ReadSideProjectionHandle;
use kitlogger::KITLogger;
use kitlogger_log_domain::Severity;

pub use projection::{TenantUsersView, UserSummary, UsersByTenantHandler, UsersByTenantStore};
pub use store::{InMemoryDedupStore, InMemoryOffsetStore, ReadSideSink, SharedReadSideStore};

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

/// Bookkeeping-only scope passed to `OffsetStore`/`DedupStore`. CORE-005
/// keys those per `(projection_id, tag, tenant)`, and tenants are already
/// isolated at the store level via `tenant_tag` (one tag stream per
/// tenant, not a single shared stream) — so this constant is just the
/// third component of that composite key, not a claim that tenants share
/// bookkeeping state. A fixed value is correct here specifically because
/// `tag` already varies per tenant; this string never needs to.
const BOOKKEEPING_SCOPE: &str = "all-tenants";

/// Poll interval for the background scheduler loop. `TagSchedulerImpl`
/// itself is not a persistent poller — `start_projection` processes one
/// batch per call and returns (see `crates/runtime/src/read_side/scheduler.rs`)
/// — so this module supplies the "actively polls" half of CORE-005's own
/// pull-based assumption (see the archived read-side-projections spec's
/// Assumptions section).
const POLL_INTERVAL: Duration = Duration::from_millis(50);

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
    offset_store: InMemoryOffsetStore,
    dedup_store: InMemoryDedupStore,
    /// Logs poll-loop failures instead of letting them vanish silently.
    /// `None` by default (e.g. in tests that don't care); `build_runtime`
    /// wires the real `Runtime`'s logger, mirroring
    /// `RegisterUserImpl`'s `Option<Arc<dyn Observability>>`.
    logger: Option<Arc<KITLogger>>,
}

impl ReadSideHandles {
    /// `store` must be the same `SharedReadSideStore` given to the
    /// `ReadSideSink` wired into `RegisterUserImpl` — otherwise the
    /// scheduler polls an empty store.
    pub fn new(store: SharedReadSideStore) -> Self {
        let query = UsersByTenantStore::default();
        Self {
            handler: UsersByTenantHandler::new(query.clone()),
            query,
            store,
            offset_store: InMemoryOffsetStore::default(),
            dedup_store: InMemoryDedupStore::default(),
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
    /// `TagSchedulerImpl` via the framework's own
    /// `TagSchedulerImpl::spawn_projection` spawn/stop lifecycle wrapper
    /// (CORE-026 Phase 3 — this module no longer wires the stop channel or
    /// the poll-loop wrapper itself). Must be called from inside a running
    /// Tokio runtime.
    pub fn spawn(self) -> ReadSideRuntime {
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());
        let store = self.store;
        let handler = self.handler;
        let offset_store = self.offset_store;
        let dedup_store = self.dedup_store;
        let logger = self.logger;

        // Tags are discovered dynamically (one per tenant, see `tenant_tag`)
        // on every poll via the `tag_provider` closure `spawn_projection`
        // calls fresh each iteration, instead of a fixed list decided once —
        // a tenant's tag only exists once its first event has been written.
        let store_for_provider = store.clone();
        let handle = scheduler.spawn_projection(
            move || store_for_provider.known_tags(),
            POLL_INTERVAL,
            PROJECTION_ID.to_string(),
            BOOKKEEPING_SCOPE.to_string(),
            handler,
            store,
            dedup_store,
            offset_store,
            NoopProgressReporter,
            move |e| {
                if let Some(logger) = &logger {
                    let _ = logger.log(Severity::Error, &format!("read_side poll failed: {e}"));
                }
            },
        );

        ReadSideRuntime { handle }
    }
}

/// Handle to the spawned polling loop — a thin adapter over the framework's
/// `ReadSideProjectionHandle`, mapping its raw `JoinError` to this app's own
/// `RuntimeInfraError::Teardown` so `Runtime::shutdown_async` (which awaits
/// this as a registered async teardown hook) can report the failure instead
/// of printing "shutdown complete" regardless (CORE-018 Finding F-02).
pub struct ReadSideRuntime {
    handle: ReadSideProjectionHandle,
}

impl ReadSideRuntime {
    /// Signals the loop to stop and awaits the in-flight batch's drain —
    /// see `ReadSideProjectionHandle::stop`.
    pub async fn stop(self) -> Result<(), ego_service_sdk::RuntimeInfraError> {
        self.handle.stop().await.map_err(|join_err| ego_service_sdk::RuntimeInfraError::Teardown {
            reason: format!("read-side scheduler task failed to drain: {join_err}"),
        })
    }
}
