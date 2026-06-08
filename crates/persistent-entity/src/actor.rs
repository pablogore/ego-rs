use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::event::DomainEvent;

use crate::lifecycle::{LifecycleStateMachine, EntityState};
use crate::mailbox::BoundedMailbox;
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::PersistentEntity;
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::scheduler::EntityTriple;
use crate::scheduler_event::{SchedulerEvent, SchedulerEventSender};
use crate::snapshot::SnapshotStrategy;
use crate::command_envelope::CommandEnvelope;
use tracing::info;

pub struct EntityActor<C, E: DomainEvent, S> {
    pub entity_id: EntityTriple,
    pub mailbox: BoundedMailbox<CommandEnvelope<C>>,
    pub state: Option<S>,
    pub version: u64,
    pub lifecycle: LifecycleStateMachine,
    pub registry: Arc<EntityRegistry>,
    pub persistence: Arc<PersistenceFacade<E>>,
    pub publisher: Arc<dyn EventPublisher<E>>,
    pub snapshot_strategy: Arc<dyn SnapshotStrategy>,
    pub entity_handler: Arc<dyn PersistentEntity<Command = C, Event = E, State = S>>,
    pub event_sender: SchedulerEventSender,
    pub _phantom: PhantomData<(C, S)>,
}

impl<C, E, S> EntityActor<C, E, S>
where
    C: Send + Sync + serde::Serialize + 'static,
    E: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + 'static,
    S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + Sync + 'static,
{
    pub async fn run(&mut self) {
        self.recover_state().await;
        if self.lifecycle.current_state == EntityState::Failed {
            self.registry.remove_active(&self.entity_id.aggregate_id()).await;
            return;
        }
        self.process_commands().await;
        self.passivate().await;
    }

    async fn recover_state(&mut self) {
        let load_result = self.persistence.load_for_recovery(
            &self.entity_id.aggregate_id(),
            Some(&self.entity_id.tenant_id),
        ).await;

        match load_result {
            Ok((snap_data, stored_events)) => {
                let (mut state, snap_version): (S, u64) = match snap_data {
                    Some(ref snap) => {
                        let s = serde_json::from_slice(&snap.data).unwrap_or_else(|_| {
                            self.entity_handler.initial_state()
                        });
                        (s, snap.version)
                    }
                    None => (self.entity_handler.initial_state(), 0),
                };
                let mut version = snap_version;

                if !stored_events.is_empty() {
                    let new_state = self.entity_handler.apply_events(
                        &state,
                        &stored_events.iter().map(|e| e.event.clone()).collect::<Vec<_>>(),
                    ).await.unwrap_or_else(|_| {
                        self.entity_handler.initial_state()
                    });
                    state = new_state;
                    version += stored_events.len() as u64;
                }

                self.state = Some(state.clone());
                self.version = version;
                let _ = self.lifecycle.transition_to(EntityState::Active);

                self.event_sender.emit(SchedulerEvent::RecoveryCompleted {
                    entity: self.entity_id.clone(),
                    state_version: version,
                });
            }
            Err(e) => {
                let _ = self.lifecycle.transition_to(EntityState::Failed);
                info!("Recovery failed for {}: {}", self.entity_id.aggregate_id(), e);
            }
        }
    }

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

    async fn execute_command(&mut self, envelope: CommandEnvelope<C>) {
        let current_state = match &self.state {
            Some(s) => s.clone(),
            None => return,
        };

        let handler_result = self.entity_handler.handle_command(
            &envelope.command,
            &current_state,
            &envelope.context,
        ).await;

        // Process the command result directly without serialization concerns
        match handler_result {
            Ok(events) if events.is_empty() => {
                // No events case - just update state
                self.state = Some(current_state);
            }
            Ok(events) => {
                let events_clone = events.clone();
                let persist_result = self.persistence.persist_events(
                    &self.entity_id.aggregate_id(),
                    Some(&self.entity_id.tenant_id),
                    self.version,
                    events_clone,
                ).await;

                match persist_result {
                    Ok(new_version) => {
                        let reload = self.persistence.load_for_recovery(
                            &self.entity_id.aggregate_id(),
                            Some(&self.entity_id.tenant_id),
                        ).await;
                        let (snap_data, stored_events) = reload.unwrap_or((None, Vec::new()));
                        let mut state = match snap_data {
                            Some(snap) => {
                                serde_json::from_slice(&snap.data).unwrap_or_else(|_| {
                                    self.entity_handler.initial_state()
                                })
                            }
                            None => self.entity_handler.initial_state(),
                        };
                        if !stored_events.is_empty() {
                            let new_state = self.entity_handler.apply_events(
                                &state,
&stored_events.iter().map(|e| e.event.clone()).collect::<Vec<_>>(),
                            ).await.unwrap_or_else(|_| {
                                self.entity_handler.initial_state()
                            });
                            state = new_state;
                        }
                        self.state = Some(state.clone());
                        self.version = new_version;

                        self.event_sender.emit(SchedulerEvent::ExecutionCompleted {
                            entity: self.entity_id.clone(),
                            state_version: new_version,
                        });

                        self.event_sender.emit(SchedulerEvent::EntityStateUpdated {
                            entity: self.entity_id.clone(),
                            state_version: new_version,
                        });

                        // Check if we should take a snapshot
                        let should_snapshot = self.snapshot_strategy.should_take_snapshot(new_version, events.len() as u64).await.unwrap_or(false);
                        if should_snapshot {
                            let _ = self.persistence.store_snapshot(
                                &self.entity_id.aggregate_id(),
                                Some(&self.entity_id.tenant_id),
                                new_version,
                                &serde_json::to_value(&state).unwrap_or(serde_json::Value::Null),
                            ).await;
                        }

                        let published_events: Vec<E> = stored_events.iter().map(|s| s.event.clone()).collect();
                        let _ = self.publisher.publish(&published_events).await;
                    }
                    Err(_e) => {
                        // Handle persistence error
                        let _ = self.lifecycle.transition_to(EntityState::Failed);
                        // The error will be handled by the caller
                        return;
                    }
                }
            }
            Err(_err_string) => {
                let _ = self.lifecycle.transition_to(EntityState::Failed);
                // The error will be handled by the caller
                return;
            }
        };

        // Fix the envelope context field access - it's just 'context', not 'response_tx'
        // The context is just metadata, not a sender. We don't need to send anything back.
    }

    async fn passivate(&mut self) {
        let _ = self.lifecycle.transition_to(EntityState::Passivating);

        while let Ok(envelope) = self.mailbox.recv().await {
            self.execute_command(envelope).await;
        }

if let Some(state) = &self.state {
            let _ = self.persistence.store_snapshot(
                &self.entity_id.aggregate_id(),
                Some(&self.entity_id.tenant_id),
                self.version,
                &serde_json::to_value(&state).unwrap(),
            ).await;
        }

        self.registry
            .mark_passivated(self.entity_id.aggregate_id(), self.version)
            .await;

        let _ = self.lifecycle.transition_to(EntityState::Passivated);
    }
}
