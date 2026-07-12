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
use kitlogger::KITLogger;
use kitlogger_log_domain::Severity;
use tokio::sync::watch;
use tokio::task::JoinHandle;

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
/// keys those per `(projection_id, tag, tenant)`, but this projection fans
/// every tenant's events into one tag stream and separates tenants inside
/// the handler instead (via `EventStreamElement::tenant_id`) — a single
/// constant scope is correct for offset/dedup bookkeeping here.
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
    /// `TagSchedulerImpl::run_until_stopped` poll-loop wrapper (finding 8
    /// fix — this module no longer hand-rolls the loop itself). Must be
    /// called from inside a running Tokio runtime.
    pub fn spawn(self) -> ReadSideRuntime {
        let (stop_tx, stop_rx) = watch::channel(false);
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());
        let store = self.store;
        let handler = self.handler;
        let offset_store = self.offset_store;
        let dedup_store = self.dedup_store;
        let logger = self.logger;

        // Tags are discovered dynamically (one per tenant, see `tenant_tag`)
        // on every poll via `run_until_stopped`'s `tag_provider` closure,
        // instead of a fixed list decided once — a tenant's tag only exists
        // once its first event has been written.
        let store_for_provider = store.clone();
        let task = scheduler.run_until_stopped(
            move || store_for_provider.known_tags(),
            POLL_INTERVAL,
            stop_rx,
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

        ReadSideRuntime { stop_tx, task }
    }
}

/// Handle to the spawned polling loop.
///
/// `stop()` is the read-side half of Task 3's graceful shutdown: it signals
/// the loop to stop scheduling new polls, then awaits the task join so a
/// poll already in flight finishes draining before this returns — no
/// read-side work is dropped mid-batch.
pub struct ReadSideRuntime {
    stop_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl ReadSideRuntime {
    pub async fn stop(self) {
        let _ = self.stop_tx.send(true);
        let _ = self.task.await;
    }
}
