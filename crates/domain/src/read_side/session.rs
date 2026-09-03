//! Read side session — batch execution with metadata-atomic commit.

use std::marker::PhantomData;
use std::sync::Arc;

use super::claim::{ClaimError, ClaimFence, ClaimId, ReadSideClaimStore};
use super::config::ReadSideConfig;
use super::dedup::DedupStore;
use super::error::ProjectionError;
use super::event_tag::EventTag;
use super::handler::Handler;
use super::offset::Offset;
use super::offset::OffsetStore;
use super::progress::ProgressReporter;
use super::store::ReadSideStore;
use crate::operation::OwnerId;
use crate::time::clock::Clock;

/// Everything a session needs to claim its stream before fetching or
/// invoking its handler (PROD-014C AD-6).
///
/// An optional knob: attaching one via [`ReadSideSession::with_claiming`] is
/// what turns per-batch claiming on. Every existing [`ReadSideSession::new`]
/// call site compiles and runs unchanged, unclaimed, exactly as before this
/// knob existed — `Profile::Production` is what makes claiming non-optional
/// in a real composition, not this type.
#[derive(Clone)]
pub struct ReadSideClaiming {
    /// The claim store backing this session's stream.
    pub store: Arc<dyn ReadSideClaimStore + Send + Sync>,
    /// This session's claimant identity.
    ///
    /// MUST be unique per **process instance**, not merely per replica or
    /// per deployment. Two live processes sharing one [`OwnerId`] can each
    /// satisfy the other's fence check, silently degrading execution
    /// exclusion down to lease-expiry alone — the very property claiming
    /// exists to provide. [`ReadSideClaimStore`] has no way to verify this
    /// from inside the port; violating it is a host misconfiguration this
    /// session cannot detect or refuse (documented Open Question, AD-6).
    pub owner: OwnerId,
    /// The clock lease expiry is computed against — never the store's own
    /// notion of "now", so every claim decision in a fleet is judged
    /// against one consistent time source.
    pub clock: Arc<dyn Clock>,
    /// How long a granted or renewed claim remains valid before it becomes
    /// takeover-eligible.
    pub lease: chrono::Duration,
}

/// A session manages the execution of a single batch of events.
///
/// Phase 1: Fetch events from ReadSideStore
/// Phase 2: Filter duplicates via DedupStore
/// Phase 3: Execute handler
/// Phase 4: Commit offsets and dedup markers atomically
pub struct ReadSideSession<E, H, RS, DS, OS, PR>
where
    E: Clone,
    H: Handler<E>,
    RS: ReadSideStore<E>,
    DS: DedupStore,
    OS: OffsetStore,
    PR: ProgressReporter,
{
    _phantom: PhantomData<E>,
    projection_id: String,
    tag: EventTag,
    tenant: String,
    config: ReadSideConfig,
    handler: H,
    read_store: RS,
    dedup_store: DS,
    offset_store: OS,
    reporter: PR,
    claiming: Option<ReadSideClaiming>,
}

impl<E, H, RS, DS, OS, PR> ReadSideSession<E, H, RS, DS, OS, PR>
where
    E: Clone,
    H: Handler<E>,
    RS: ReadSideStore<E>,
    DS: DedupStore,
    OS: OffsetStore,
    PR: ProgressReporter,
{
    /// Creates a new session.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        projection_id: String,
        tag: EventTag,
        tenant: String,
        config: ReadSideConfig,
        handler: H,
        read_store: RS,
        dedup_store: DS,
        offset_store: OS,
        reporter: PR,
    ) -> Self {
        Self {
            _phantom: std::marker::PhantomData,
            projection_id,
            tag,
            tenant,
            config,
            handler,
            read_store,
            dedup_store,
            offset_store,
            reporter,
            claiming: None,
        }
    }

    /// Attaches claiming to this session (PROD-014C AD-6). Optional — a
    /// session with no claiming attached runs exactly as it did before this
    /// knob existed.
    pub fn with_claiming(mut self, claiming: ReadSideClaiming) -> Self {
        self.claiming = Some(claiming);
        self
    }

    fn claim_id(&self) -> ClaimId {
        ClaimId {
            projection_id: self.projection_id.clone(),
            tag: self.tag.clone(),
            tenant: self.tenant.clone(),
        }
    }

    /// Executes one batch: claim (if configured), read the persisted offset,
    /// fetch, dedup, handle, commit, release.
    ///
    /// When claiming is attached, a refused claim (`Ok(None)`) is not an
    /// error: another worker holds an unexpired lease on this stream. The
    /// caller MUST NOT fetch or invoke the handler in that case, and
    /// `execute()` returns `Ok(None)` — the same "nothing advanced this
    /// tick" result an empty fetch already returns (PROD-014C IS-4, AD-4).
    ///
    /// Returns the new offset after the batch, or an error.
    /// Returns `Ok(None)` if no events were available or the claim was
    /// refused.
    pub async fn execute(&self) -> Result<Option<Offset>, ProjectionError> {
        let Some(claiming) = &self.claiming else {
            return self.run_batch(None).await;
        };

        let claim_id = self.claim_id();
        let fence = claiming
            .store
            .try_claim(
                &claim_id,
                &claiming.owner,
                claiming.clock.now() + claiming.lease,
            )
            .await
            .map_err(|e| ProjectionError::transient(format!("claim acquisition failed: {e}")))?;

        let Some(fence) = fence else {
            // Refused: another worker holds an unexpired lease. No fetch, no
            // handler — this is the normal steady state on every non-owning
            // replica's tick, not an error (AD-4).
            return Ok(None);
        };

        let result = self.run_batch(Some(&fence)).await;

        // Best-effort and deliberately swallowed: the work this batch did
        // (if any) already committed, and the lease expires on its own. A
        // failed `release` is not a batch failure (AD-6).
        let _ = claiming.store.release(&fence).await;

        result
    }

    /// The persisted offset is read internally on every call — the caller
    /// never passes one in — so a batch always resumes from where the last
    /// successful commit left off instead of always re-fetching from the
    /// start of the tag stream. Without this, once a tag stream accumulates
    /// more than `batch_size` events, every poll would keep re-fetching the
    /// same first batch (all already deduped) and the projection would
    /// stall forever short of the full stream.
    async fn run_batch(
        &self,
        fence: Option<&ClaimFence>,
    ) -> Result<Option<Offset>, ProjectionError> {
        let last_offset = self
            .offset_store
            .read_offset(&self.projection_id, &self.tag, &self.tenant)
            .await
            .map_err(|e| ProjectionError::transient(format!("read offset failed: {}", e)))?;

        // Phase 1: Fetch events. Tenant isolation is now passed explicitly to
        // the store (type-enforced) instead of relying solely on the tenant
        // being folded into `self.tag`.
        let events = self
            .read_store
            .fetch(
                &self.tenant,
                &self.tag,
                last_offset.as_ref(),
                self.config.batch_size,
            )
            .await
            .map_err(|e| ProjectionError::transient(format!("fetch failed: {}", e)))?;

        if events.is_empty() {
            return Ok(None);
        }

        // Phase 2: Filter duplicates
        let mut unique_events = Vec::new();
        for event in &events {
            let is_duplicate = self
                .dedup_store
                .seen(&self.projection_id, &self.tag, event.event_id())
                .await
                .map_err(|e| ProjectionError::transient(format!("dedup check failed: {}", e)))?;

            if !is_duplicate {
                unique_events.push((*event).clone());
            }
        }

        if unique_events.is_empty() {
            return Ok(None);
        }

        // Phase 3: Execute handler
        let result = self.handler.handle(&unique_events).await;

        // Phase 4: Commit
        match result {
            Ok(()) => {
                // Re-verify ownership before writing anything (PROD-014C
                // AD-6): the fence gate sits as late as possible, between
                // the handler and the commit loop. `StaleOwner` aborts here
                // — no mark_seen, no write_offset — so a replaced owner can
                // never rewind the current owner's offset.
                if let (Some(claiming), Some(fence)) = (&self.claiming, fence) {
                    claiming
                        .store
                        .renew(fence, claiming.clock.now() + claiming.lease)
                        .await
                        .map_err(|e| match e {
                            ClaimError::StaleOwner => ProjectionError::transient(
                                "claim lost before commit; this batch's offset and dedup \
                                 writes were withheld so a replaced owner cannot rewind the \
                                 current owner's offset",
                            ),
                            other => {
                                ProjectionError::transient(format!("claim renew failed: {other}"))
                            }
                        })?;
                }

                let new_offset = Offset::sequence(unique_events.last().unwrap().event_version());

                // Mark all events as seen
                for event in &unique_events {
                    self.dedup_store
                        .mark_seen(&self.projection_id, &self.tag, event.event_id())
                        .await
                        .map_err(|e| {
                            ProjectionError::transient(format!("mark dedup failed: {}", e))
                        })?;
                }

                // Write offset
                self.offset_store
                    .write_offset(&self.projection_id, &self.tag, &self.tenant, &new_offset)
                    .await
                    .map_err(|e| {
                        ProjectionError::transient(format!("write offset failed: {}", e))
                    })?;

                // Report progress
                self.reporter.on_batch_completed(
                    &self.projection_id,
                    &self.tag,
                    unique_events.len(),
                    &new_offset,
                );

                Ok(Some(new_offset))
            }
            Err(err) => {
                self.reporter
                    .on_error(&self.projection_id, &format!("{}", err));
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod claiming_tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::{DateTime, Utc};

    use super::*;
    use crate::operation::FencingToken;
    use crate::read_side::dedup::DedupStoreError;
    use crate::read_side::event_stream::EventStreamElement;
    use crate::read_side::offset::OffsetStoreError;
    use crate::read_side::progress::NoopProgressReporter;
    use crate::read_side::store::ReadSideStoreError;

    fn tag() -> EventTag {
        EventTag::new("users-by-tenant")
    }

    fn event(version: i64) -> EventStreamElement<serde_json::Value> {
        EventStreamElement::new(
            format!("evt-{version}"),
            "agg-1",
            "tenant-a",
            "Something",
            serde_json::json!({}),
            version,
            Utc::now(),
            vec![tag()],
        )
    }

    fn test_fence() -> ClaimFence {
        ClaimFence {
            claim_id: ClaimId {
                projection_id: "proj".to_string(),
                tag: tag(),
                tenant: "tenant-a".to_string(),
            },
            owner_id: OwnerId::new("owner-1"),
            fencing_token: FencingToken::initial(),
        }
    }

    fn claiming(store: Arc<ScriptedClaimStore>) -> ReadSideClaiming {
        ReadSideClaiming {
            store,
            owner: OwnerId::new("owner-1"),
            clock: Arc::new(FixedClock(Utc::now())),
            lease: chrono::Duration::seconds(30),
        }
    }

    // ---- scripted doubles, no pool anywhere ----

    struct FixedClock(DateTime<Utc>);

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    #[derive(Clone, Default)]
    struct CountingReadStore {
        fetch_calls: Arc<AtomicUsize>,
        events: Vec<EventStreamElement<serde_json::Value>>,
    }

    #[async_trait::async_trait]
    impl ReadSideStore<serde_json::Value> for CountingReadStore {
        async fn fetch(
            &self,
            _tenant: &str,
            _tag: &EventTag,
            _offset: Option<&Offset>,
            _batch_size: usize,
        ) -> Result<Vec<EventStreamElement<serde_json::Value>>, ReadSideStoreError> {
            self.fetch_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.events.clone())
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum HandlerOutcome {
        Success,
        Failure,
    }

    #[derive(Clone)]
    struct CountingHandler {
        calls: Arc<AtomicUsize>,
        outcome: HandlerOutcome,
    }

    #[async_trait::async_trait]
    impl Handler<serde_json::Value> for CountingHandler {
        async fn handle(
            &self,
            _events: &[EventStreamElement<serde_json::Value>],
        ) -> Result<(), ProjectionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.outcome {
                HandlerOutcome::Success => Ok(()),
                HandlerOutcome::Failure => Err(ProjectionError::transient("handler failed")),
            }
        }
    }

    #[derive(Clone, Default)]
    struct ConfigurableDedupStore {
        already_seen: HashSet<String>,
        mark_seen_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl DedupStore for ConfigurableDedupStore {
        async fn seen(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            event_id: &str,
        ) -> Result<bool, DedupStoreError> {
            Ok(self.already_seen.contains(event_id))
        }

        async fn mark_seen(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _event_id: &str,
        ) -> Result<(), DedupStoreError> {
            self.mark_seen_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct CountingOffsetStore {
        write_offset_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl OffsetStore for CountingOffsetStore {
        async fn read_offset(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _tenant: &str,
        ) -> Result<Option<Offset>, OffsetStoreError> {
            Ok(None)
        }

        async fn write_offset(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _tenant: &str,
            _offset: &Offset,
        ) -> Result<(), OffsetStoreError> {
            self.write_offset_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A `ReadSideClaimStore` whose `try_claim`/`renew` outcomes are fixed
    /// up front by the test, and whose call counts are observable
    /// afterwards. No pool, no real store anywhere (PROD-014C AD-10).
    struct ScriptedClaimStore {
        try_claim_result: Result<Option<ClaimFence>, ClaimError>,
        renew_result: Result<(), ClaimError>,
        try_claim_calls: AtomicUsize,
        renew_calls: AtomicUsize,
        release_calls: AtomicUsize,
    }

    impl ScriptedClaimStore {
        fn refusing() -> Self {
            Self {
                try_claim_result: Ok(None),
                renew_result: Ok(()),
                try_claim_calls: AtomicUsize::new(0),
                renew_calls: AtomicUsize::new(0),
                release_calls: AtomicUsize::new(0),
            }
        }

        fn granting(fence: ClaimFence) -> Self {
            Self {
                try_claim_result: Ok(Some(fence)),
                renew_result: Ok(()),
                try_claim_calls: AtomicUsize::new(0),
                renew_calls: AtomicUsize::new(0),
                release_calls: AtomicUsize::new(0),
            }
        }

        fn granting_but_stale_on_renew(fence: ClaimFence) -> Self {
            Self {
                try_claim_result: Ok(Some(fence)),
                renew_result: Err(ClaimError::StaleOwner),
                try_claim_calls: AtomicUsize::new(0),
                renew_calls: AtomicUsize::new(0),
                release_calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl ReadSideClaimStore for ScriptedClaimStore {
        async fn try_claim(
            &self,
            _claim_id: &ClaimId,
            _owner_id: &OwnerId,
            _lease_until: DateTime<Utc>,
        ) -> Result<Option<ClaimFence>, ClaimError> {
            self.try_claim_calls.fetch_add(1, Ordering::SeqCst);
            self.try_claim_result.clone()
        }

        async fn renew(
            &self,
            _fence: &ClaimFence,
            _lease_until: DateTime<Utc>,
        ) -> Result<(), ClaimError> {
            self.renew_calls.fetch_add(1, Ordering::SeqCst);
            self.renew_result.clone()
        }

        async fn release(&self, _fence: &ClaimFence) -> Result<(), ClaimError> {
            self.release_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // ---- 5.1: a refused try_claim skips fetch and the handler entirely ----

    #[tokio::test]
    async fn refused_claim_skips_fetch_and_handler_and_returns_ok_none() {
        let fetch_calls = Arc::new(AtomicUsize::new(0));
        let handle_calls = Arc::new(AtomicUsize::new(0));
        let claim_store = Arc::new(ScriptedClaimStore::refusing());

        let session = ReadSideSession::new(
            "proj".to_string(),
            tag(),
            "tenant-a".to_string(),
            ReadSideConfig::default(),
            CountingHandler {
                calls: handle_calls.clone(),
                outcome: HandlerOutcome::Success,
            },
            CountingReadStore {
                fetch_calls: fetch_calls.clone(),
                events: vec![event(1)],
            },
            ConfigurableDedupStore::default(),
            CountingOffsetStore::default(),
            NoopProgressReporter,
        )
        .with_claiming(claiming(claim_store.clone()));

        let result = session.execute().await;

        assert_eq!(result, Ok(None));
        assert_eq!(
            fetch_calls.load(Ordering::SeqCst),
            0,
            "a refused claim must never call fetch"
        );
        assert_eq!(
            handle_calls.load(Ordering::SeqCst),
            0,
            "a refused claim must never invoke the handler"
        );
        assert_eq!(claim_store.try_claim_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            claim_store.release_calls.load(Ordering::SeqCst),
            0,
            "a refusal never acquired anything, so there is nothing to release"
        );
    }

    // ---- 5.2: a stale-owner renew withholds mark_seen and write_offset ----

    #[tokio::test]
    async fn stale_owner_on_renew_withholds_mark_seen_and_write_offset() {
        let mark_seen_calls = Arc::new(AtomicUsize::new(0));
        let write_offset_calls = Arc::new(AtomicUsize::new(0));
        let claim_store = Arc::new(ScriptedClaimStore::granting_but_stale_on_renew(test_fence()));

        let session = ReadSideSession::new(
            "proj".to_string(),
            tag(),
            "tenant-a".to_string(),
            ReadSideConfig::default(),
            CountingHandler {
                calls: Arc::new(AtomicUsize::new(0)),
                outcome: HandlerOutcome::Success,
            },
            CountingReadStore {
                fetch_calls: Arc::new(AtomicUsize::new(0)),
                events: vec![event(1)],
            },
            ConfigurableDedupStore {
                mark_seen_calls: mark_seen_calls.clone(),
                ..Default::default()
            },
            CountingOffsetStore {
                write_offset_calls: write_offset_calls.clone(),
            },
            NoopProgressReporter,
        )
        .with_claiming(claiming(claim_store.clone()));

        let result = session.execute().await;

        let err = result.expect_err("a stale-owner renew must fail the batch");
        assert!(
            err.is_transient(),
            "a stale-owner renew must propagate as transient, got {err:?}"
        );
        let message = err.to_string();
        assert!(
            message.contains("offset"),
            "the error must name the withheld offset write: {message}"
        );
        assert!(
            message.contains("dedup"),
            "the error must name the withheld dedup write: {message}"
        );
        assert_eq!(
            mark_seen_calls.load(Ordering::SeqCst),
            0,
            "no dedup marker may be written once renew reports StaleOwner"
        );
        assert_eq!(
            write_offset_calls.load(Ordering::SeqCst),
            0,
            "no offset write may happen once renew reports StaleOwner"
        );
        assert_eq!(
            claim_store.release_calls.load(Ordering::SeqCst),
            1,
            "release is unconditional even on a renew failure (AD-6)"
        );
    }

    // ---- 5.3: release runs on every exit path ----

    #[tokio::test]
    async fn release_is_called_on_the_success_path() {
        let claim_store = Arc::new(ScriptedClaimStore::granting(test_fence()));

        let session = ReadSideSession::new(
            "proj".to_string(),
            tag(),
            "tenant-a".to_string(),
            ReadSideConfig::default(),
            CountingHandler {
                calls: Arc::new(AtomicUsize::new(0)),
                outcome: HandlerOutcome::Success,
            },
            CountingReadStore {
                fetch_calls: Arc::new(AtomicUsize::new(0)),
                events: vec![event(1)],
            },
            ConfigurableDedupStore::default(),
            CountingOffsetStore::default(),
            NoopProgressReporter,
        )
        .with_claiming(claiming(claim_store.clone()));

        let result = session.execute().await;

        assert_eq!(result, Ok(Some(Offset::sequence(1))));
        assert_eq!(claim_store.release_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn release_is_called_when_fetch_returns_no_events() {
        let claim_store = Arc::new(ScriptedClaimStore::granting(test_fence()));

        let session = ReadSideSession::new(
            "proj".to_string(),
            tag(),
            "tenant-a".to_string(),
            ReadSideConfig::default(),
            CountingHandler {
                calls: Arc::new(AtomicUsize::new(0)),
                outcome: HandlerOutcome::Success,
            },
            CountingReadStore {
                fetch_calls: Arc::new(AtomicUsize::new(0)),
                events: vec![],
            },
            ConfigurableDedupStore::default(),
            CountingOffsetStore::default(),
            NoopProgressReporter,
        )
        .with_claiming(claiming(claim_store.clone()));

        let result = session.execute().await;

        assert_eq!(result, Ok(None));
        assert_eq!(claim_store.release_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn release_is_called_when_every_event_is_a_duplicate() {
        let claim_store = Arc::new(ScriptedClaimStore::granting(test_fence()));
        let mut already_seen = HashSet::new();
        already_seen.insert("evt-1".to_string());

        let session = ReadSideSession::new(
            "proj".to_string(),
            tag(),
            "tenant-a".to_string(),
            ReadSideConfig::default(),
            CountingHandler {
                calls: Arc::new(AtomicUsize::new(0)),
                outcome: HandlerOutcome::Success,
            },
            CountingReadStore {
                fetch_calls: Arc::new(AtomicUsize::new(0)),
                events: vec![event(1)],
            },
            ConfigurableDedupStore {
                already_seen,
                ..Default::default()
            },
            CountingOffsetStore::default(),
            NoopProgressReporter,
        )
        .with_claiming(claiming(claim_store.clone()));

        let result = session.execute().await;

        assert_eq!(result, Ok(None));
        assert_eq!(claim_store.release_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn release_is_called_on_the_handler_error_path() {
        let claim_store = Arc::new(ScriptedClaimStore::granting(test_fence()));

        let session = ReadSideSession::new(
            "proj".to_string(),
            tag(),
            "tenant-a".to_string(),
            ReadSideConfig::default(),
            CountingHandler {
                calls: Arc::new(AtomicUsize::new(0)),
                outcome: HandlerOutcome::Failure,
            },
            CountingReadStore {
                fetch_calls: Arc::new(AtomicUsize::new(0)),
                events: vec![event(1)],
            },
            ConfigurableDedupStore::default(),
            CountingOffsetStore::default(),
            NoopProgressReporter,
        )
        .with_claiming(claiming(claim_store.clone()));

        let result = session.execute().await;

        assert!(result.is_err(), "the handler error must propagate");
        assert_eq!(claim_store.release_calls.load(Ordering::SeqCst), 1);
    }

    // ---- regression: no claiming attached runs exactly as before ----

    #[tokio::test]
    async fn no_claiming_configured_runs_the_batch_directly() {
        let session = ReadSideSession::new(
            "proj".to_string(),
            tag(),
            "tenant-a".to_string(),
            ReadSideConfig::default(),
            CountingHandler {
                calls: Arc::new(AtomicUsize::new(0)),
                outcome: HandlerOutcome::Success,
            },
            CountingReadStore {
                fetch_calls: Arc::new(AtomicUsize::new(0)),
                events: vec![event(1)],
            },
            ConfigurableDedupStore::default(),
            CountingOffsetStore::default(),
            NoopProgressReporter,
        );

        let result = session.execute().await;

        assert_eq!(result, Ok(Some(Offset::sequence(1))));
    }
}
