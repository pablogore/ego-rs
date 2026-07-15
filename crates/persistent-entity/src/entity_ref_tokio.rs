//! Production [`EntityRef`] backed by a real [`EntityActor`] spawned via `tokio::spawn`.

use std::any::Any;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{oneshot, watch};

use crate::actor::EntityActor;
use crate::command_context::CommandContext;
use crate::command_envelope::{ActorEnvelope, CommandEnvelope};
use crate::entity_ref::EntityRef;
use crate::error::EntityError;
use crate::lifecycle::{EntityState, LifecycleStateMachine};
use crate::mailbox::BoundedMailbox;
use crate::passivation_signal::TokioPassivationSignal;
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::PersistentEntity;
use crate::publisher::EventPublisher;
use crate::registry::{EntityRegistry, RouteOutcome};
use crate::scheduler::EntityTriple;
use crate::scheduler_event::SchedulerEventSender;
use crate::snapshot::SnapshotStrategy;
use ego_domain::event::DomainEvent;

/// The actor's sole teardown contract (ADR-005, FR-009): every step is
/// synchronous so it can run from `Drop`, the only code guaranteed to execute
/// on *every* exit path — normal return, panic during recovery/command
/// processing/passivation-drain, task cancellation, and runtime shutdown.
///
/// Moved into the spawned actor task (`entity_ref_tokio.rs`'s
/// `TokioEntityRef::new`, `Inserted` branch) alongside the actor itself, so a
/// panic anywhere in `EntityActor::run()` drops this guard during unwind.
pub(crate) struct TeardownGuard<C> {
    pub(crate) aggregate_id: String,
    pub(crate) registry: Arc<EntityRegistry>,
    pub(crate) epoch: u64,
    /// The same mailbox handle the actor holds — draining through this
    /// clone is independent of whether the actor's own in-body drain
    /// (`passivate`/`drain_mailbox_with_error`) ran, finished, or panicked.
    pub(crate) mailbox: BoundedMailbox<ActorEnvelope<C>>,
    /// Clone of the actor's `watch::Sender` — a Drop-time backstop publish,
    /// not a competing normal-path writer (ADR-003: the actor is still the
    /// only writer on every path that reaches its own `transition_to`).
    pub(crate) tx: watch::Sender<EntityState>,
}

impl<C> Drop for TeardownGuard<C> {
    fn drop(&mut self) {
        // Steps 1+2: close the mailbox and synchronously drain every
        // still-queued envelope, terminally answering each one. Sync,
        // parking_lot-backed, and safe to call during panic unwind.
        for envelope in self.mailbox.close_and_drain() {
            let _ = envelope.reply.send(Err(EntityError::EntityNotActive));
        }

        // Step 3: remove-if-mine (epoch-scoped, idempotent — a safe no-op if
        // the actor's own exit path already removed this entry).
        self.registry.deactivate_if_mine(&self.aggregate_id, self.epoch);

        // Step 4: publish a terminal state — but never stomp a terminal
        // state the actor already legitimately published. This only
        // backstops the case where the actor died before publishing
        // anything itself (e.g. a panic before its first `transition_to`).
        let current = *self.tx.borrow();
        if current != EntityState::Failed && current != EntityState::Passivated {
            let _ = self.tx.send(EntityState::Failed);
        }
    }
}

/// Production [`EntityRef`] — spawns one [`EntityActor`] and holds its mailbox write-side.
pub struct TokioEntityRef<C, E, S> {
    /// The entity's identity (tenant/type/id).
    entity_id: EntityTriple,
    /// Write-side of the bounded mailbox shared with the spawned actor.
    mailbox: BoundedMailbox<ActorEnvelope<C>>,
    _phantom: PhantomData<(E, S)>,
}

impl<C, E, S> fmt::Debug for TokioEntityRef<C, E, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokioEntityRef")
            .field("entity_id", &self.entity_id)
            .finish()
    }
}

/// Clones share the same actor mailbox — both refs dispatch to the same actor.
impl<C, E, S> Clone for TokioEntityRef<C, E, S> {
    fn clone(&self) -> Self {
        Self {
            entity_id: self.entity_id.clone(),
            mailbox: self.mailbox.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<C, E, S> TokioEntityRef<C, E, S>
where
    C: Send + Sync + serde::Serialize + 'static,
    E: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static,
    S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + Sync + 'static,
{
    /// Looks up an existing live actor for `triple`, or spawns a new one
    /// (ADR-001's lookup-or-spawn), and returns the mailbox write-side.
    ///
    /// Returns `Err(EntityError::Internal(..))` if a live entry exists but its
    /// erased mailbox does not downcast to `BoundedMailbox<ActorEnvelope<C>>`
    /// (ADR-002) — a programming error (mismatched `entity_type`/command
    /// type). This is never treated as "no live entry" and never falls
    /// through to a competing spawn.
    pub fn new(
        triple: EntityTriple,
        registry: Arc<EntityRegistry>,
        persistence: Arc<PersistenceFacade<E>>,
        publisher: Arc<dyn EventPublisher<E>>,
        snapshot_strategy: Arc<dyn SnapshotStrategy>,
        entity_handler: Arc<dyn PersistentEntity<Command = C, Event = E, State = S>>,
        event_sender: SchedulerEventSender,
        mailbox_capacity: usize,
        passivation_timeout: std::time::Duration,
    ) -> Result<Self, EntityError> {
        let aggregate_id = triple.aggregate_id();

        // Single-flight critical section (ADR-001): lazily builds the mailbox only
        // if no live entry exists yet, under one lock acquisition.
        let outcome = registry.lookup_or_insert(&aggregate_id, || {
            let mailbox: BoundedMailbox<ActorEnvelope<C>> = BoundedMailbox::new(mailbox_capacity);
            Arc::new(mailbox) as Arc<dyn Any + Send + Sync>
        });

        let downcast = |erased: Arc<dyn Any + Send + Sync>| {
            erased
                .downcast::<BoundedMailbox<ActorEnvelope<C>>>()
                .map_err(|_| {
                    debug_assert!(
                        false,
                        "routing type mismatch for triple {aggregate_id}: erased mailbox is not \
                         BoundedMailbox<ActorEnvelope<C>> for this entity_type"
                    );
                    EntityError::Internal(format!("routing type mismatch for triple {aggregate_id}"))
                })
        };

        match outcome {
            RouteOutcome::Existing { mailbox: erased } => {
                // Downcast happens after the lock is released (ADR-001/ADR-002).
                let mailbox = downcast(erased)?;
                Ok(TokioEntityRef {
                    entity_id: triple,
                    mailbox: (*mailbox).clone(),
                    _phantom: PhantomData,
                })
            }
            RouteOutcome::Inserted { mailbox: erased, epoch, tx } => {
                // Freshly inserted under this call's own type parameters, so the
                // downcast is infallible in practice — kept uniform with the
                // Existing branch rather than special-cased.
                let mailbox = downcast(erased)
                    .expect("freshly-inserted mailbox always matches its own type");
                let mailbox_for_actor = (*mailbox).clone();

                let entity_id = triple.clone();
                let registry_for_actor = registry.clone();

                // The actor is the sole writer of its published state during
                // normal operation (ADR-003); the guard holds a clone purely
                // as a Drop-time backstop (see TeardownGuard::drop).
                let tx_for_actor = tx.clone();

                let mut actor = EntityActor {
                    entity_id: triple,
                    mailbox: mailbox_for_actor.clone(),
                    state: None,
                    version: 0,
                    lifecycle: LifecycleStateMachine::new(),
                    registry: registry_for_actor,
                    tx: tx_for_actor,
                    persistence,
                    publisher,
                    // Phase 9/PR4 (builder lifecycle wiring, out of scope
                    // here) is expected to thread a configured
                    // `EffectAcceptor` through once the runtime builder
                    // registers ≥1 external-effect executor. Until then,
                    // `None` keeps this capability at zero cost (AD-2/spec:
                    // "Zero cost when the capability is unused").
                    effect_acceptor: None,
                    snapshot_strategy,
                    entity_handler,
                    event_sender,
                    signal: TokioPassivationSignal::new(passivation_timeout),
                    _phantom: PhantomData,
                };

                // Moved into the spawned future, strictly after the registry
                // lock (ADR-001's critical section) has already been
                // released by `lookup_or_insert`'s return above — never
                // constructed under the lock (Round 3 self-deadlock fix).
                let guard = TeardownGuard {
                    aggregate_id: aggregate_id.clone(),
                    registry: registry.clone(),
                    epoch,
                    mailbox: mailbox_for_actor,
                    tx,
                };

                tokio::spawn(async move {
                    actor.run().await;
                    drop(guard);
                });

                Ok(TokioEntityRef {
                    entity_id,
                    mailbox: (*mailbox).clone(),
                    _phantom: PhantomData,
                })
            }
        }
    }
}

#[async_trait]
impl<C, E, S> EntityRef for TokioEntityRef<C, E, S>
where
    C: Send + Sync + serde::Serialize + 'static,
    E: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static,
    S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + Sync + 'static,
{
    type Command = C;

    /// Enqueues a command on the actor mailbox and awaits the reply.
    async fn send_command<T>(
        &self,
        command: C,
        context: CommandContext,
    ) -> Result<T, EntityError>
    where
        T: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let actor_envelope = ActorEnvelope {
            envelope: CommandEnvelope {
                command,
                context,
            },
            reply: tx,
        };

        self.mailbox
            .send(actor_envelope)
            .await
            .map_err(|_| EntityError::MailboxClosed)?;

        let erased = rx.await.map_err(|_| {
            EntityError::Internal("actor dropped reply channel without responding".to_string())
        })??;

        let boxed_t: Box<T> = erased.downcast::<T>().map_err(|_| {
            EntityError::Internal(
                "TokioEntityRef::send_command: result type mismatch".to_string(),
            )
        })?;
        Ok(*boxed_t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistent_entity::CommandResult;
    use crate::registry::RouteOutcome;
    use crate::scheduler_event::event_bus_channel;
    use crate::snapshot::NoSnapshot;
    use crate::test_entity::TestEntity;
    use crate::testing::{create_test_context, NoopPublisher, TestCommand, TestEvent, TestState};

    /// TASK-009 / FR-010 (ADR-008): a caller who observes `MailboxClosed`
    /// during the close→remove teardown window (the old actor's mailbox is
    /// closed but its registry entry has not yet been removed) must be able
    /// to retry `entity_ref()`/`TokioEntityRef::new()` and reach a freshly
    /// spawned, healthy actor for the same triple — the window must never be
    /// a dead end.
    ///
    /// The window is built deterministically (matching `mailbox.rs`'s
    /// `close_and_drain_races_concurrent_sends_without_losing_envelopes`
    /// pattern) rather than raced against the scheduler: a live registry
    /// entry is inserted directly and its mailbox closed out-of-band, which
    /// is exactly the observable state a concurrent caller would see between
    /// `deactivate()`'s step 1 (close) and step 3 (remove).
    #[tokio::test]
    async fn mailbox_closed_in_teardown_window_is_retried_to_a_fresh_actor() {
        let registry = Arc::new(EntityRegistry::new());
        let triple = EntityTriple::new("default".to_string(), "counter", "reactivate-window-1");
        let aggregate_id = triple.aggregate_id();

        // Simulate an old actor mid-teardown: insert a live entry directly
        // and close its mailbox out-of-band, without removing the entry —
        // the FR-010 window.
        let stale_epoch = match registry.lookup_or_insert(&aggregate_id, || {
            let mailbox: BoundedMailbox<ActorEnvelope<TestCommand>> = BoundedMailbox::new(4);
            mailbox.close();
            Arc::new(mailbox) as Arc<dyn Any + Send + Sync>
        }) {
            RouteOutcome::Inserted { epoch, .. } => epoch,
            RouteOutcome::Existing { .. } => panic!("expected a fresh insert"),
        };

        // A caller in this window finds the stale (but still-present) entry
        // and must observe MailboxClosed rather than hang or see a bogus
        // spawn-a-second-actor fallback.
        let stale_ref = TokioEntityRef::new(
            triple.clone(),
            registry.clone(),
            Arc::new(PersistenceFacade::<TestEvent>::new()),
            Arc::new(NoopPublisher::new()),
            Arc::new(NoSnapshot),
            Arc::new(TestEntity::new()),
            event_bus_channel().0,
            4,
            std::time::Duration::from_secs(300),
        )
        .expect("existing entry must downcast cleanly");

        let stale_result: Result<CommandResult<TestEvent, TestState>, EntityError> = stale_ref
            .send_command(TestCommand::GetState, create_test_context())
            .await;
        assert!(
            matches!(stale_result, Err(EntityError::MailboxClosed)),
            "a caller in the teardown window must observe MailboxClosed, distinguishable from a \
             permanent failure: got {stale_result:?}"
        );

        // Teardown completes: the stale entry is removed (deactivate() step 3).
        registry.deactivate_if_mine(&aggregate_id, stale_epoch);

        // The caller retries entity_ref(): no live entry exists now, so a
        // fresh, healthy actor is spawned for the same triple.
        let fresh_ref = TokioEntityRef::new(
            triple,
            registry,
            Arc::new(PersistenceFacade::<TestEvent>::new()),
            Arc::new(NoopPublisher::new()),
            Arc::new(NoSnapshot),
            Arc::new(TestEntity::new()),
            event_bus_channel().0,
            4,
            std::time::Duration::from_secs(300),
        )
        .expect("no live entry remains, a fresh spawn must succeed");

        let fresh_result: CommandResult<TestEvent, TestState> = fresh_ref
            .send_command(TestCommand::Increment(1), create_test_context())
            .await
            .expect("the retry must reach a newly-activated, healthy actor");

        match fresh_result {
            CommandResult::Events { new_state, .. } => assert_eq!(new_state.value, 1),
            other => panic!("expected Events variant from the fresh actor, got {other:?}"),
        }
    }
}
