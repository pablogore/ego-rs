use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::event::DomainEvent;
use tokio::sync::oneshot;

use crate::actor::EntityActor;
use crate::command_context::CommandContext;
use crate::error::EntityError;
use crate::lifecycle::LifecycleStateMachine;
use crate::mailbox::{CommandEnvelope, CommandErasedResult};
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::{PersistentEntity, CommandResult};
use crate::publisher::EventPublisher;
use crate::registry::{ActorHandle, EntityRegistry};
use crate::scheduler::EntityTriple;
use crate::snapshot::SnapshotStrategy;

pub struct EntityRef<C, E: DomainEvent, S> {
    entity: EntityTriple,
    registry: Arc<EntityRegistry>,
    persistence: Arc<PersistenceFacade<E>>,
    publisher: Arc<dyn EventPublisher<E>>,
    mailbox_capacity: usize,
    snapshot_strategy: Arc<dyn SnapshotStrategy>,
    entity_handler: Arc<dyn PersistentEntity<C, E, S>>,
    _phantom: PhantomData<(C, S)>,
}

impl<C, E, S> EntityRef<C, E, S>
where
    C: Send + 'static,
    E: DomainEvent + Clone + serde::de::DeserializeOwned + 'static,
    S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + 'static,
{
    pub fn new(
        entity: EntityTriple,
        registry: Arc<EntityRegistry>,
        persistence: Arc<PersistenceFacade<E>>,
        publisher: Arc<dyn EventPublisher<E>>,
        mailbox_capacity: usize,
        snapshot_strategy: Arc<dyn SnapshotStrategy>,
        entity_handler: Arc<dyn PersistentEntity<C, E, S>>,
    ) -> Self {
        EntityRef {
            entity,
            registry,
            persistence,
            publisher,
            mailbox_capacity,
            snapshot_strategy,
            entity_handler,
            _phantom: PhantomData,
        }
    }

    pub async fn send(
        &self,
        command: C,
        ctx: CommandContext,
        expected_version: Option<u64>,
    ) -> Result<CommandResult<E, S>, EntityError> {
        let (response_tx, response_rx) = oneshot::channel();

        let envelope = CommandEnvelope {
            command,
            ctx,
            response_tx,
            expected_version,
        };

        let sender = self.registry.get_active_sender::<C>(&self.entity).await;

        match sender {
            Some(tx) => {
                tx.send(envelope).await.map_err(|_| {
                    EntityError::Runtime("actor mailbox closed".into())
                })?;
                let erased = response_rx.await.map_err(|_| {
                    EntityError::Runtime("response channel closed".into())
                })?;
                downcast_result::<E, S>(erased)
            }
            None => {
                self.activate_and_send(envelope, response_rx).await
            }
        }
    }

    async fn activate_and_send(
        &self,
        envelope: CommandEnvelope<C>,
        response_rx: oneshot::Receiver<Result<CommandErasedResult, EntityError>>,
    ) -> Result<CommandResult<E, S>, EntityError> {
        let activation = self.registry.get_or_create_activation(self.entity.clone()).await;
        let guard = activation.lock.lock().await;

        if let Some(sender) = self.registry.get_active_sender::<C>(&self.entity).await {
            drop(guard);
            sender.send(envelope).await.map_err(|_| {
                EntityError::Runtime("actor mailbox closed".into())
            })?;
            let erased = response_rx.await.map_err(|_| {
                EntityError::Runtime("response channel closed".into())
            })?;
            return downcast_result::<E, S>(erased);
        }

        let (mailbox_tx, mailbox_rx) = tokio::sync::mpsc::channel(self.mailbox_capacity);

        let entity_id = self.entity.clone();
        let registry = self.registry.clone();
        let persistence = self.persistence.clone();
        let publisher = self.publisher.clone();
        let snapshot_strategy = self.snapshot_strategy.clone();
        let entity_handler = self.entity_handler.clone();

        let join_handle = tokio::spawn(async move {
            let mut actor = EntityActor::<C, E, S> {
                entity_id: entity_id.clone(),
                mailbox: mailbox_rx,
                state: None,
                version: 0,
                lifecycle: LifecycleStateMachine::new(),
                registry: registry.clone(),
                persistence: persistence.clone(),
                publisher: publisher.clone(),
                snapshot_strategy: (*snapshot_strategy).clone_boxed(),
                entity_handler,
                _phantom: PhantomData,
            };
            actor.run().await;
        });

        self.registry.insert_active(
            self.entity.clone(),
            ActorHandle::new(mailbox_tx.clone(), join_handle),
        ).await;

        self.registry.remove_passivated(&self.entity).await;
        drop(guard);
        self.registry.remove_activation(&self.entity).await;

        mailbox_tx.send(envelope).await.map_err(|_| {
            EntityError::Runtime("mailbox closed after activation".into())
        })?;

        let erased = response_rx.await.map_err(|_| {
            EntityError::Runtime("response channel closed".into())
        })?;
        downcast_result::<E, S>(erased)
    }

    pub fn entity_id(&self) -> &EntityTriple {
        &self.entity
    }
}

fn downcast_result<E: 'static, S: 'static>(
    result: Result<CommandErasedResult, EntityError>,
) -> Result<CommandResult<E, S>, EntityError> {
    match result {
        Ok(erased) => {
            let concrete: Box<CommandResult<E, S>> = erased.downcast::<CommandResult<E, S>>().map_err(|_| {
                EntityError::Runtime("type mismatch in command result".into())
            })?;
            Ok(*concrete)
        }
        Err(e) => Err(e),
    }
}
