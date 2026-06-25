//! Production [`EntityRef`] backed by a real [`EntityActor`] spawned via `tokio::spawn`.

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
use crate::lifecycle::LifecycleStateMachine;
use crate::mailbox::BoundedMailbox;
use crate::passivation_signal::TokioPassivationSignal;
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::PersistentEntity;
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::scheduler::EntityTriple;
use crate::scheduler_event::SchedulerEventSender;
use crate::snapshot::SnapshotStrategy;
use ego_domain::event::DomainEvent;

/// Calls `remove_active` on drop — guards against a leaked active entry when the spawned future
/// is dropped before it ever polls (e.g. runtime teardown before task starts). `remove_active`
/// is idempotent: calling it after normal passivation (which already removed the entry) is safe.
struct SpawnGuard {
    aggregate_id: String,
    registry: Arc<EntityRegistry>,
}

impl Drop for SpawnGuard {
    fn drop(&mut self) {
        self.registry.remove_active(&self.aggregate_id);
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
    /// Marks the entity active, spawns the actor, and returns the mailbox write-side.
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
    ) -> Self {
        let mailbox: BoundedMailbox<ActorEnvelope<C>> = BoundedMailbox::new(mailbox_capacity);
        let mailbox_for_actor = mailbox.clone();

        let entity_id = triple.clone();
        let registry_for_actor = registry.clone();

        let aggregate_id = triple.aggregate_id();

        let mut actor = EntityActor {
            entity_id: triple,
            mailbox: mailbox_for_actor,
            state: None,
            version: 0,
            lifecycle: LifecycleStateMachine::new(),
            registry: registry_for_actor,
            persistence,
            publisher,
            snapshot_strategy,
            entity_handler,
            event_sender,
            signal: TokioPassivationSignal::new(passivation_timeout),
            _phantom: PhantomData,
        };

        // Mark the entity as active before spawning so that callers who inspect
        // registry.active_count() immediately after entity_ref() observe the correct count.
        // Registry ops use std::sync::Mutex (no await), so this is safe from a sync fn.
        //
        // Known window: active_count() is inflated by one between this line and the actor's
        // first poll. Moving mark_active inside run() would hide the entity until the first
        // Tokio context switch, which is worse. SpawnGuard handles the case where the spawned
        // future is dropped before it ever polls (runtime teardown).
        registry.mark_active(&aggregate_id);

        // Guard calls remove_active on drop. If the runtime drops the spawned future before it
        // ever polls (e.g. shutdown), the active entry is cleaned up. In the normal path,
        // actor.run() already removes the entry; Drop is a safe no-op because remove_active
        // is idempotent.
        let _guard = SpawnGuard {
            aggregate_id: aggregate_id.clone(),
            registry: registry.clone(),
        };

        tokio::spawn(async move {
            actor.run().await;
            drop(_guard);
        });

        TokioEntityRef {
            entity_id,
            mailbox,
            _phantom: PhantomData,
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
