use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use ego_domain::persistence::{EventStore, PersistenceError, StoredEvent};
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::runtime::EntityRuntime;
use persistent_entity::testing::{InMemoryEventStore, TestCommand, TestEvent, TestState};

/// Send `count` concurrent Increment commands to an entity.
/// Spawns one tokio task per command, each creating its own EntityRef.
///
/// Retries on `EntityError::MailboxClosed` with a fresh `entity_ref()` —
/// per `openspec/changes/archive/2026-07-07-activation-authority/design.md`'s
/// documented contract, `MailboxClosed` is "a distinct, caller-retryable
/// terminal error; caller may re-`entity_ref()`", not a hard failure. A
/// concurrent caller can legitimately be routed to a just-closed mailbox
/// during the (real, intentional) close→remove teardown window before its
/// registry entry is removed; without retrying, a burst racing that window
/// is flaky under CPU contention even though nothing is actually broken —
/// see `entity_ref_tokio.rs`'s `mailbox_closed_in_teardown_window_is_retried_to_a_fresh_actor`.
pub async fn spawn_concurrent_commands(
    count: usize,
    runtime: Arc<EntityRuntime<TestEvent>>,
    entity_type: &'static str,
    entity_id: &'static str,
    handler: Arc<dyn PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>>,
) -> Vec<Result<CommandResult<TestEvent, TestState>, EntityError>> {
    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        let command = TestCommand::Increment((i + 1) as u64);
        let h = handler.clone();
        let rt = runtime.clone();

        handles.push(tokio::spawn(async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let entity_ref = rt
                    .entity_ref::<TestCommand, TestState>(entity_type, entity_id, h.clone())
                    .unwrap();
                let ctx = CommandContext::new(entity_type.to_string());
                let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
                    entity_ref.send_command(command.clone(), ctx).await;
                match result {
                    Err(EntityError::MailboxClosed) if std::time::Instant::now() < deadline => {
                        continue
                    }
                    other => break other,
                }
            }
        }));
    }

    let mut results = Vec::with_capacity(count);
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    results
}

/// `EventStore` wrapping a real in-memory backing store, instrumented with an
/// `Arc<AtomicUsize>` load-call counter — actor-level activation-attempt
/// instrumentation (NFR-002) for tests that must prove "no duplicate actor"
/// by counting genuine recovery attempts, not by inspecting
/// `active_count()`/ID-set cardinality. Behaves identically to a plain
/// `InMemoryEventStore` otherwise.
pub struct CountingEventStore {
    inner: InMemoryEventStore<TestEvent>,
    pub load_calls: Arc<AtomicUsize>,
}

impl CountingEventStore {
    pub fn new(load_calls: Arc<AtomicUsize>) -> Self {
        Self {
            inner: InMemoryEventStore::new(),
            load_calls,
        }
    }
}

impl EventStore<TestEvent> for CountingEventStore {
    fn append(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<TestEvent>>,
    ) -> Result<i64, PersistenceError> {
        self.inner
            .append(aggregate_id, tenant_id, expected_version, events)
    }

    fn load(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<TestEvent>>, PersistenceError> {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.load(aggregate_id, tenant_id)
    }

    fn list_aggregate_ids(&self, tenant_id: Option<&str>) -> Result<Vec<String>, PersistenceError> {
        self.inner.list_aggregate_ids(tenant_id)
    }
}
