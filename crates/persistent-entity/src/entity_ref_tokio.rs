//! Tokio-backed entity reference.
//!
//! [`TokioEntityRef`] is the production implementation of [`EntityRef`].  It
//! spawns an [`EntityActor`] via `tokio::spawn` and forwards commands via a
//! [`BoundedMailbox`].  Each [`send_command`](TokioEntityRef::send_command)
//! call creates a per-command `oneshot` channel so the actor can deliver the
//! result back to the caller without shared mutable state.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
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

/// A production [`EntityRef`] backed by a real [`EntityActor`] task.
///
/// Created by [`EntityRuntime::entity_ref`].  Spawns the actor once on
/// construction and holds the write-side of the mailbox so that multiple
/// callers can send commands concurrently.
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

/// Manual `Clone` impl so that the `C: Clone` bound is not propagated.
///
/// `TokioEntityRef` only holds an `Arc`-backed [`BoundedMailbox`] and an
/// [`EntityTriple`] (value type).  Cloning produces a second ref that shares
/// the same actor mailbox — both can dispatch commands to the same actor.
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
    /// Constructs a [`TokioEntityRef`] and spawns the backing [`EntityActor`].
    ///
    /// The actor task is launched via [`tokio::spawn`] and runs until the
    /// passivation timeout fires or the mailbox is closed.  The returned ref
    /// holds the write-side of the mailbox so commands can be dispatched at
    /// any time after construction.
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

        // Mark the entity as active in the registry before spawning so that
        // callers who inspect registry.active_count() immediately after
        // entity_ref() observe the correct count.
        let registry_for_mark = registry.clone();
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

        tokio::spawn(async move {
            // Mark entity active inside the spawned task so it runs within the
            // Tokio runtime context.
            registry_for_mark.mark_active(&aggregate_id).await;
            actor.run().await;
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
    /// Sends a command to the actor and awaits its result.
    ///
    /// Creates a per-command [`oneshot`] channel, wraps the command in an
    /// [`ActorEnvelope`], enqueues it in the mailbox, and awaits the reply.
    /// The reply is a type-erased [`CommandErasedResult`]; this method
    /// downcasts it to `T`.
    ///
    /// # Errors
    ///
    /// - [`EntityError::MailboxClosed`] if the actor has already passivated.
    /// - [`EntityError::Internal`] if the oneshot channel is dropped before
    ///   a reply is sent (actor panicked or was cancelled).
    /// - Any [`EntityError`] the actor itself sends back (handler failure,
    ///   persistence failure, etc.).
    async fn send_command<T, Cmd>(
        &self,
        command: Cmd,
        context: CommandContext,
    ) -> Result<T, EntityError>
    where
        T: Send + 'static,
        Cmd: Serialize + Send + 'static,
    {
        // `EntityRef::send_command` is generic over `Cmd`, but this concrete
        // `TokioEntityRef<C, E, S>` only speaks `C`.  The `Any` downcast is a
        // runtime type-check: it succeeds when the caller passes `Cmd = C`
        // (which is the intended use) and fails with a clear error otherwise.
        // This mirrors the pattern used by `TestEntityRef`.
        let cmd_any: Box<dyn std::any::Any + Send> = Box::new(command);
        let cmd_c: C = *cmd_any.downcast::<C>().map_err(|_| {
            EntityError::Internal(
                "TokioEntityRef::send_command: command type mismatch".to_string(),
            )
        })?;

        let (tx, rx) = oneshot::channel();
        let actor_envelope = ActorEnvelope {
            envelope: CommandEnvelope {
                command: cmd_c,
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

        // Downcast the erased result to T.
        let boxed_t: Box<T> = erased.downcast::<T>().map_err(|_| {
            EntityError::Internal(
                "TokioEntityRef::send_command: result type mismatch".to_string(),
            )
        })?;
        Ok(*boxed_t)
    }
}
