//! Entity actor — the core execution unit for persistent entities.
//!
//! The [`EntityActor`] owns the lifecycle of a single entity instance:
//! recovery, command processing, passivation, and failure handling.

use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::event::DomainEvent;

use crate::command_envelope::CommandEnvelope;
use crate::lifecycle::{EntityState, LifecycleStateMachine};
use crate::mailbox::BoundedMailbox;
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::PersistentEntity;
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::scheduler::EntityTriple;
use crate::scheduler_event::{SchedulerEvent, SchedulerEventSender};
use crate::snapshot::SnapshotStrategy;
use tracing::{error, warn};

/// Core actor that owns entity state, mailbox, and lifecycle.
///
/// Runs recovery on start, processes commands from its mailbox,
/// persists events, publishes them, and passivates after inactivity.
pub struct EntityActor<C, E: DomainEvent, S> {
    /// The entity's identity (tenant/type/id).
    pub entity_id: EntityTriple,
    /// Bounded mailbox for incoming command envelopes.
    pub mailbox: BoundedMailbox<CommandEnvelope<C>>,
    /// Current entity state, if recovered.
    pub state: Option<S>,
    /// Current version number (number of committed events).
    pub version: u64,
    /// Lifecycle state machine.
    pub lifecycle: LifecycleStateMachine,
    /// Shared entity registry.
    pub registry: Arc<EntityRegistry>,
    /// Persistence facade for loading and storing events/snapshots.
    pub persistence: Arc<PersistenceFacade<E>>,
    /// Event publisher for notifying downstream consumers.
    pub publisher: Arc<dyn EventPublisher<E>>,
    /// Snapshot strategy for periodic state snapshots.
    pub snapshot_strategy: Arc<dyn SnapshotStrategy>,
    /// Domain handler that implements command → events and event application.
    pub entity_handler: Arc<dyn PersistentEntity<Command = C, Event = E, State = S>>,
    /// Sender for emitting scheduler lifecycle events.
    pub event_sender: SchedulerEventSender,
    pub _phantom: PhantomData<(C, S)>,
}

impl<C, E, S> EntityActor<C, E, S>
where
    C: Send + Sync + serde::Serialize + 'static,
    E: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + 'static,
    S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + Sync + 'static,
{
    /// Runs the actor lifecycle: recover → process commands → passivate.
    ///
    /// If recovery fails, the actor transitions to Failed and exits immediately.
    pub async fn run(&mut self) {
        self.recover_state().await;
        if self.lifecycle.current_state == EntityState::Failed {
            self.registry
                .remove_active(&self.entity_id.aggregate_id())
                .await;
            return;
        }
        self.process_commands().await;
        self.passivate().await;
    }

    /// Loads persisted state and replays events to recover the entity.
    ///
    /// On success, transitions to Active and emits a RecoveryCompleted event.
    /// On failure, transitions to Failed.
    async fn recover_state(&mut self) {
        let load_result = self
            .persistence
            .load_for_recovery(
                &self.entity_id.aggregate_id(),
                Some(&self.entity_id.tenant_id),
            )
            .await;

        match load_result {
            Ok((snap_data, stored_events)) => {
                let (mut state, snap_version): (S, u64) = match snap_data {
                    Some(ref snap) => {
                        let s = serde_json::from_slice(&snap.data)
                            .unwrap_or_else(|e| {
                                warn!(error = %e, "snapshot deserialization failed, falling back to initial state");
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

                self.state = Some(state.clone());
                self.version = version;
                let _ = self.lifecycle.transition_to(EntityState::Active);

                if !self.event_sender.emit(SchedulerEvent::RecoveryCompleted {
                    entity: self.entity_id.clone(),
                    state_version: version,
                }) {
                    warn!(entity_id = %self.entity_id.aggregate_id(), "scheduler bus full, event dropped");
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

    /// Processes commands from the mailbox until passivation timeout or mailbox close.
    async fn process_commands(&mut self) {
        let timeout = std::time::Duration::from_secs(300);

        loop {
            tokio::select! {
                result = self.mailbox.recv() => {
                    match result {
                        Ok(envelope) => {
                            self.execute_command(envelope).await;
                            if !self.lifecycle.is_active() {
                                break;
                            }
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(timeout) => {
                    break;
                }
            }
        }
    }

    /// Executes a single command: handle → persist → publish.
    ///
    /// On success, persists events, reloads state, emits scheduler events,
    /// optionally snapshots, and publishes events to downstream consumers.
    /// On failure, transitions the actor to Failed.
    async fn execute_command(&mut self, envelope: CommandEnvelope<C>) {
        let current_state = match &self.state {
            Some(s) => s.clone(),
            None => return,
        };

        let handler_result = self
            .entity_handler
            .handle_command(&envelope.command, &current_state, &envelope.context)
            .await;

        // Process the command result directly without serialization concerns
        match handler_result {
            Ok(events) if events.is_empty() => {
                // No events case - just update state
                self.state = Some(current_state);
            }
            Ok(events) => {
                let events_clone = events.clone();
                let persist_result = self
                    .persistence
                    .persist_events(
                        &self.entity_id.aggregate_id(),
                        Some(&self.entity_id.tenant_id),
                        self.version,
                        events_clone,
                    )
                    .await;

                match persist_result {
                    Ok(new_version) => {
                        let reload = self
                            .persistence
                            .load_for_recovery(
                                &self.entity_id.aggregate_id(),
                                Some(&self.entity_id.tenant_id),
                            )
                            .await;
                        let (snap_data, stored_events) = reload.unwrap_or_else(|e| {
                            warn!(error = %e, entity_id = %self.entity_id.aggregate_id(), "post-persist reload failed, using empty state");
                            (None, Vec::new())
                        });
                        let mut state = match snap_data {
                            Some(snap) => serde_json::from_slice(&snap.data)
                                .unwrap_or_else(|e| {
                                    warn!(error = %e, "snapshot deserialization failed, falling back to initial state");
                                    self.entity_handler.initial_state()
                                }),
                            None => self.entity_handler.initial_state(),
                        };
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
                                    warn!(error = %e, entity_id = %self.entity_id.aggregate_id(), "post-persist event replay failed, using initial state");
                                    self.entity_handler.initial_state()
                                });
                            state = new_state;
                        }
                        self.state = Some(state.clone());
                        self.version = new_version;

                        if !self.event_sender.emit(SchedulerEvent::ExecutionCompleted {
                            entity: self.entity_id.clone(),
                            state_version: new_version,
                        }) {
                            warn!(entity_id = %self.entity_id.aggregate_id(), "scheduler bus full, event dropped");
                        }

                        if !self.event_sender.emit(SchedulerEvent::EntityStateUpdated {
                            entity: self.entity_id.clone(),
                            state_version: new_version,
                        }) {
                            warn!(entity_id = %self.entity_id.aggregate_id(), "scheduler bus full, event dropped");
                        }

                        // Check if we should take a snapshot
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

                        let published_events: Vec<E> =
                            stored_events.iter().map(|s| s.event.clone()).collect();
                        let _ = self.publisher.publish(&published_events).await;
                    }
                    Err(e) => {
                        let _ = self.lifecycle.transition_to(EntityState::Failed);
                        error!(
                            error = %e,
                            entity_id = %self.entity_id.aggregate_id(),
                            "event persistence failed — entity transitioned to Failed state"
                        );
                        return;
                    }
                }
            }
            Err(err_string) => {
                let _ = self.lifecycle.transition_to(EntityState::Failed);
                error!(
                    error = %err_string,
                    entity_id = %self.entity_id.aggregate_id(),
                    "command handler failed — entity transitioned to Failed state"
                );
                return;
            }
        };
    }

    /// Passivates the actor: drains remaining mailbox, stores snapshot,
    /// marks as passivated in registry, and transitions to Passivated state.
    async fn passivate(&mut self) {
        let _ = self.lifecycle.transition_to(EntityState::Passivating);

        // Close the mailbox first so recv() returns MailboxClosed once empty,
        // rather than blocking forever waiting for the next command.
        self.mailbox.close();

        while let Ok(envelope) = self.mailbox.recv().await {
            self.execute_command(envelope).await;
        }

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
            .mark_passivated(self.entity_id.aggregate_id(), self.version)
            .await;

        let _ = self.lifecycle.transition_to(EntityState::Passivated);
    }
}
