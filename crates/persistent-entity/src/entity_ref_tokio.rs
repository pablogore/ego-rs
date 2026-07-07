//! Production [`EntityRef`] backed by a real [`EntityActor`] spawned via `tokio::spawn`.

use std::any::Any;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::oneshot;

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

/// Calls `deactivate_if_mine` on drop — guards against a leaked routing entry when the spawned
/// future is dropped before it ever polls (e.g. runtime teardown before task starts).
/// `deactivate_if_mine` is idempotent and epoch-scoped: calling it after normal passivation
/// (which already removed the entry) or against a superseded epoch is a safe no-op.
///
/// Phase 3's `TeardownGuard` (design.md ADR-005) supersedes this with the full
/// close-mailbox+drain+remove contract; this guard only covers the pre-poll-drop case for now.
struct SpawnGuard {
    aggregate_id: String,
    registry: Arc<EntityRegistry>,
    epoch: u64,
}

impl Drop for SpawnGuard {
    fn drop(&mut self) {
        self.registry.deactivate_if_mine(&self.aggregate_id, self.epoch);
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

                // Phase 2 parity shim: publish Active synchronously here, mirroring
                // today's eager mark_active-before-spawn timing bug-for-bug, so
                // active_count() keeps its current observable semantics. Phase 3
                // (design.md ADR-003) moves this into the actor's own lifecycle
                // transitions, publishing through `tx` only once recovery completes.
                let _ = tx.send(EntityState::Active);

                let entity_id = triple.clone();
                let registry_for_actor = registry.clone();

                let mut actor = EntityActor {
                    entity_id: triple,
                    mailbox: mailbox_for_actor,
                    state: None,
                    version: 0,
                    lifecycle: LifecycleStateMachine::new(),
                    registry: registry_for_actor,
                    epoch,
                    persistence,
                    publisher,
                    snapshot_strategy,
                    entity_handler,
                    event_sender,
                    signal: TokioPassivationSignal::new(passivation_timeout),
                    _phantom: PhantomData,
                };

                // Guard calls deactivate_if_mine on drop. If the runtime drops the
                // spawned future before it ever polls (e.g. shutdown), the routing
                // entry is cleaned up. In the normal path, actor.run() already
                // removes the entry; Drop is then a safe, epoch-scoped no-op.
                let _guard = SpawnGuard {
                    aggregate_id: aggregate_id.clone(),
                    registry: registry.clone(),
                    epoch,
                };

                tokio::spawn(async move {
                    actor.run().await;
                    drop(_guard);
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
