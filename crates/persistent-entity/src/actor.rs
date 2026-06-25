//! Entity actor — the core execution unit for persistent entities.
//!
//! The [`EntityActor`] owns the lifecycle of a single entity instance:
//! recovery, command processing, passivation, and failure handling.

use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::event::DomainEvent;

use crate::command_envelope::{ActorEnvelope, CommandEnvelope};
use crate::lifecycle::{EntityState, LifecycleStateMachine};
use crate::mailbox::{BoundedMailbox, CommandErasedResult};
use crate::passivation_signal::PassivationSignal;
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::{CommandResult, PersistentEntity};
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::scheduler::EntityTriple;
use crate::scheduler_event::{SchedulerEvent, SchedulerEventSender};
use crate::snapshot::SnapshotStrategy;
use tracing::{error, warn};

/// Owns the lifecycle of a single entity: recovery → command loop → passivation.
pub struct EntityActor<C, E: DomainEvent, S, Sig: PassivationSignal> {
    /// The entity's identity (tenant/type/id).
    pub(crate) entity_id: EntityTriple,
    /// Bounded mailbox for incoming actor envelopes (command + reply channel).
    pub(crate) mailbox: BoundedMailbox<ActorEnvelope<C>>,
    /// Current entity state, if recovered.
    pub(crate) state: Option<S>,
    /// Current version number (number of committed events).
    pub(crate) version: u64,
    /// Lifecycle state machine.
    pub(crate) lifecycle: LifecycleStateMachine,
    /// Shared entity registry.
    pub(crate) registry: Arc<EntityRegistry>,
    /// Persistence facade for loading and storing events/snapshots.
    pub(crate) persistence: Arc<PersistenceFacade<E>>,
    /// Event publisher for notifying downstream consumers.
    pub(crate) publisher: Arc<dyn EventPublisher<E>>,
    /// Snapshot strategy for periodic state snapshots.
    pub(crate) snapshot_strategy: Arc<dyn SnapshotStrategy>,
    /// Domain handler that implements command → events and event application.
    pub(crate) entity_handler: Arc<dyn PersistentEntity<Command = C, Event = E, State = S>>,
    /// Sender for emitting scheduler lifecycle events.
    pub(crate) event_sender: SchedulerEventSender,
    /// Passivation signal that fires when the actor should stop.
    pub(crate) signal: Sig,
    pub(crate) _phantom: PhantomData<(C, S)>,
}

impl<C, E, S, Sig> EntityActor<C, E, S, Sig>
where
    C: Send + Sync + serde::Serialize + 'static,
    E: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + 'static,
    S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + Sync + 'static,
    Sig: PassivationSignal,
{
    /// Runs recovery, then the command loop, then passivation; drains mailbox on recovery failure.
    pub async fn run(&mut self) {
        self.recover_state().await;
        if self.lifecycle.current_state == EntityState::Failed {
            self.drain_mailbox_with_error(crate::error::EntityError::EntityNotActive)
                .await;
            return;
        }
        self.process_commands().await;
        if self.lifecycle.current_state == EntityState::Failed {
            self.drain_mailbox_with_error(crate::error::EntityError::PersistenceError(
                "actor entered failed state during command processing".to_string(),
            ))
            .await;
            return;
        }
        self.passivate().await;
    }

    /// Loads the latest snapshot (if any) then replays events; returns `(state, version)` or an error.
    async fn rebuild_state_from_persistence(&self) -> Result<(S, u64), String> {
        let (snap_data, stored_events) = self
            .persistence
            .load_for_recovery(
                &self.entity_id.aggregate_id(),
                Some(&self.entity_id.tenant_id),
            )
            .await?;

        let (mut state, snap_version): (S, u64) = match snap_data {
            Some(snap) => {
                let s = serde_json::from_value::<S>(snap.data).unwrap_or_else(|e| {
                    warn!(
                        error = %e,
                        "snapshot deserialization failed, falling back to initial state"
                    );
                    self.entity_handler.initial_state()
                });
                (s, snap.version)
            }
            None => (self.entity_handler.initial_state(), 0),
        };
        let mut version = snap_version;

        if !stored_events.is_empty() {
            let new_state = self
                .entity_handler
                .apply_events(
                    &state,
                    &stored_events
                        .iter()
                        .map(|e| e.event.clone())
                        .collect::<Vec<_>>(),
                )
                .await
                .unwrap_or_else(|e| {
                    warn!(error = %e, "event replay failed, falling back to initial state");
                    self.entity_handler.initial_state()
                });
            state = new_state;
            version += stored_events.len() as u64;
        }

        Ok((state, version))
    }

    /// Loads persisted state and replays events to recover the entity.
    ///
    /// On success, transitions to Active and emits a RecoveryCompleted event.
    /// On failure, transitions to Failed.
    async fn recover_state(&mut self) {
        match self.rebuild_state_from_persistence().await {
            Ok((state, version)) => {
                self.state = Some(state);
                self.version = version;
                let _ = self.lifecycle.transition_to(EntityState::Active);

                if !self.event_sender.emit(SchedulerEvent::RecoveryCompleted {
                    entity: self.entity_id.clone(),
                    state_version: version,
                }) {
                    warn!(
                        entity_id = %self.entity_id.aggregate_id(),
                        "scheduler bus full, event dropped"
                    );
                }
            }
            Err(e) => {
                let _ = self.lifecycle.transition_to(EntityState::Failed);
                error!(
                    error = %e,
                    entity_id = %self.entity_id.aggregate_id(),
                    "entity recovery failed — entity transitioned to Failed state"
                );
            }
        }
    }

    /// Drives the mailbox loop until the passivation signal fires or the mailbox closes.
    async fn process_commands(&mut self) {
        loop {
            tokio::select! {
                result = self.mailbox.recv() => {
                    match result {
                        Ok(actor_envelope) => {
                            self.execute_command(actor_envelope).await;
                            if !self.lifecycle.is_active() {
                                break;
                            }
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
                _ = self.signal.passivated() => {
                    break;
                }
            }
        }
    }

    /// Handles one command: validate state → persist events → apply in-memory → publish; replies on every path.
    async fn execute_command(&mut self, actor_envelope: ActorEnvelope<C>) {
        let ActorEnvelope { envelope, reply } = actor_envelope;
        let CommandEnvelope { command, context } = envelope;

        let current_state = match &self.state {
            Some(s) => s.clone(),
            None => {
                let _ = reply.send(Err(crate::error::EntityError::EntityNotActive));
                return;
            }
        };

        let handler_result = self
            .entity_handler
            .handle_command(&command, &current_state, &context)
            .await;

        match handler_result {
            Ok(events) if events.is_empty() => {
                let result: CommandResult<E, S> = CommandResult::NoEvents {
                    state: current_state,
                };
                let boxed: CommandErasedResult = Box::new(result);
                let _ = reply.send(Ok(boxed));
            }
            Ok(events) => {
                let persist_result = self
                    .persistence
                    .persist_events(
                        &self.entity_id.aggregate_id(),
                        Some(&self.entity_id.tenant_id),
                        self.version,
                        &events,
                    )
                    .await;

                match persist_result {
                    Ok(new_version) => {
                        let state = match self
                            .entity_handler
                            .apply_events(&current_state, &events)
                            .await
                        {
                            Ok(s) => s,
                            Err(e) => {
                                let _ = self.lifecycle.transition_to(EntityState::Failed);
                                error!(
                                    error = %e,
                                    entity_id = %self.entity_id.aggregate_id(),
                                    "post-persist apply_events failed — actor transitioned to Failed"
                                );
                                let _ = reply.send(Err(crate::error::EntityError::PersistenceError(
                                    e.to_string(),
                                )));
                                return;
                            }
                        };

                        self.state = Some(state.clone());
                        self.version = new_version;

                        if !self.event_sender.emit(SchedulerEvent::ExecutionCompleted {
                            entity: self.entity_id.clone(),
                            state_version: new_version,
                        }) {
                            warn!(
                                entity_id = %self.entity_id.aggregate_id(),
                                "scheduler bus full, event dropped"
                            );
                        }

                        if !self.event_sender.emit(SchedulerEvent::EntityStateUpdated {
                            entity: self.entity_id.clone(),
                            state_version: new_version,
                        }) {
                            warn!(
                                entity_id = %self.entity_id.aggregate_id(),
                                "scheduler bus full, event dropped"
                            );
                        }

                        let should_snapshot = self
                            .snapshot_strategy
                            .should_take_snapshot(new_version, events.len() as u64)
                            .await
                            .unwrap_or(false);
                        if should_snapshot {
                            let _ = self
                                .persistence
                                .store_snapshot(
                                    &self.entity_id.aggregate_id(),
                                    Some(&self.entity_id.tenant_id),
                                    new_version,
                                    &serde_json::to_value(&state)
                                        .unwrap_or(serde_json::Value::Null),
                                )
                                .await;
                        }

                        let _ = self.publisher.publish(&events).await;

                        let result: CommandResult<E, S> = CommandResult::Events {
                            new_state: state,
                            events,
                        };
                        let boxed: CommandErasedResult = Box::new(result);
                        let _ = reply.send(Ok(boxed));
                    }
                    Err(e) => {
                        let _ = self.lifecycle.transition_to(EntityState::Failed);
                        error!(
                            error = %e,
                            entity_id = %self.entity_id.aggregate_id(),
                            "event persistence failed — entity transitioned to Failed state"
                        );
                        let _ = reply.send(Err(crate::error::EntityError::PersistenceError(
                            e.to_string(),
                        )));
                    }
                }
            }
            Err(err_string) => {
                // Domain-level handler errors are returned to the caller but
                // do NOT fail the actor.  The entity remains Active and ready
                // for the next command.  Only system-level failures (e.g.
                // persistence errors) should transition the actor to Failed.
                let _ = reply.send(Err(crate::error::EntityError::Internal(
                    err_string.to_string(),
                )));
            }
        };
    }

    /// Removes the entity from the active registry, closes the mailbox, and
    /// drains all pending envelopes by sending `err` to each caller.
    async fn drain_mailbox_with_error(&mut self, err: crate::error::EntityError) {
        self.registry.remove_active(&self.entity_id.aggregate_id());
        self.mailbox.close();
        while let Ok(envelope) = self.mailbox.recv().await {
            let _ = envelope.reply.send(Err(err.clone()));
        }
    }

    /// Drains the mailbox, snapshots state, and marks the entity passivated in the registry.
    async fn passivate(&mut self) {
        // Close the mailbox first so recv() returns MailboxClosed once empty,
        // rather than blocking forever waiting for the next command.
        self.mailbox.close();

        while let Ok(actor_envelope) = self.mailbox.recv().await {
            self.execute_command(actor_envelope).await;
            if self.lifecycle.current_state == EntityState::Failed {
                while let Ok(envelope) = self.mailbox.recv().await {
                    let _ = envelope.reply.send(Err(
                        crate::error::EntityError::PersistenceError(
                            "actor failed during passivation drain".to_string(),
                        ),
                    ));
                }
                self.registry.remove_active(&self.entity_id.aggregate_id());
                return;
            }
        }

        let _ = self.lifecycle.transition_to(EntityState::Passivating);

        if let Some(state) = &self.state {
            let _ = self
                .persistence
                .store_snapshot(
                    &self.entity_id.aggregate_id(),
                    Some(&self.entity_id.tenant_id),
                    self.version,
                    &serde_json::to_value(&state).unwrap_or(serde_json::Value::Null),
                )
                .await;
        }

        self.registry
            .mark_passivated(self.entity_id.aggregate_id(), self.version);

        let _ = self.lifecycle.transition_to(EntityState::Passivated);
    }
}
