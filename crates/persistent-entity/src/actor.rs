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

/// Core actor that owns entity state, mailbox, and lifecycle.
///
/// Runs recovery on start, processes commands from its mailbox,
/// persists events, publishes them, and passivates after inactivity.
///
/// Generic over `Sig: PassivationSignal` so the passivation timing strategy
/// can be swapped without Tokio coupling — production code uses
/// [`TokioPassivationSignal`](crate::passivation_signal::TokioPassivationSignal),
/// tests use [`ManualSignal`](crate::passivation_signal::ManualSignal).
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
    /// Runs the actor lifecycle: recover → process commands → passivate.
    ///
    /// If recovery fails, the actor drains any pending mailbox items with an
    /// error reply (so callers are not left hanging on their oneshot receivers),
    /// then exits.
    pub async fn run(&mut self) {
        self.recover_state().await;
        if self.lifecycle.current_state == EntityState::Failed {
            self.registry
                .remove_active(&self.entity_id.aggregate_id())
                .await;
            // Drain pending commands so callers get an error rather than hanging.
            self.mailbox.close();
            while let Ok(envelope) = self.mailbox.recv().await {
                let _ = envelope
                    .reply
                    .send(Err(crate::error::EntityError::EntityNotActive));
            }
            return;
        }
        self.process_commands().await;
        self.passivate().await;
    }

    /// Rebuilds entity state from persistence.
    ///
    /// Loads the latest snapshot (if any), then replays events on top of it.
    /// Returns the reconstituted state and the current version number, or an
    /// error string if the load failed.
    ///
    /// This is the single canonical implementation of the load-snapshot →
    /// replay-events pattern; both `recover_state` and `execute_command`
    /// delegate to it.
    async fn rebuild_state_from_persistence(&self) -> Result<(S, u64), String> {
        let (snap_data, stored_events) = self
            .persistence
            .load_for_recovery(
                &self.entity_id.aggregate_id(),
                Some(&self.entity_id.tenant_id),
            )
            .await?;

        let (mut state, snap_version): (S, u64) = match snap_data {
            Some(ref snap) => {
                let s = serde_json::from_slice(&snap.data).unwrap_or_else(|e| {
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

    /// Processes commands from the mailbox until the passivation signal fires or the
    /// mailbox is closed.
    ///
    /// `tokio::select!` composes two trait-provided futures without directly
    /// importing `tokio::time` or `tokio::sync` — per AD-6 this is acceptable
    /// because the Tokio runtime coupling now lives in the `PassivationSignal`
    /// implementation, not in the actor loop itself.
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

    /// Executes a single command: handle → persist → publish.
    ///
    /// Sends the result back to the caller via `actor_envelope.reply` on
    /// **every** exit path. Failing to do so would cause callers to block
    /// forever on the oneshot receiver.
    ///
    /// On success, persists events, reloads state, emits scheduler events,
    /// optionally snapshots, and publishes events to downstream consumers.
    /// On persistence failure, transitions the actor to Failed.
    /// On domain-level handler errors, returns the error to the caller without
    /// failing the actor (the entity stays Active and processes subsequent commands).
    async fn execute_command(&mut self, actor_envelope: ActorEnvelope<C>) {
        let ActorEnvelope { envelope, reply } = actor_envelope;
        let CommandEnvelope { command, context } = envelope;

        let current_state = match &self.state {
            Some(s) => s.clone(),
            None => {
                // Actor has no recovered state — reply with an error and return.
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
                // No events — return the unchanged state to the caller.
                let result: CommandResult<E, S> = CommandResult::NoEvents {
                    state: current_state,
                };
                let boxed: CommandErasedResult = Box::new(result);
                let _ = reply.send(Ok(boxed));
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
                        // Rebuild state from persistence (single canonical path).
                        let (state, _) =
                            self.rebuild_state_from_persistence().await.unwrap_or_else(|e| {
                                warn!(
                                    error = %e,
                                    entity_id = %self.entity_id.aggregate_id(),
                                    "post-persist reload failed, using empty state"
                                );
                                (self.entity_handler.initial_state(), new_version)
                            });

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

                        // Check if we should take a snapshot.
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

                        // Reply to caller with success result.
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
                        // Reply with error so caller is not left hanging.
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

    /// Passivates the actor: drains remaining mailbox, stores snapshot,
    /// marks as passivated in registry, and transitions to Passivated state.
    async fn passivate(&mut self) {
        let _ = self.lifecycle.transition_to(EntityState::Passivating);

        // Close the mailbox first so recv() returns MailboxClosed once empty,
        // rather than blocking forever waiting for the next command.
        self.mailbox.close();

        while let Ok(actor_envelope) = self.mailbox.recv().await {
            self.execute_command(actor_envelope).await;
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
