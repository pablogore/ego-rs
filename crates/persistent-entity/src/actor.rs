use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::event::DomainEvent;
use tokio::sync::mpsc;

use crate::error::EntityError;
use crate::lifecycle::{LifecycleStateMachine, LifecycleState};
use crate::mailbox::{CommandEnvelope, CommandErasedResult};
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::{PersistentEntity, CommandResult};
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::scheduler::EntityTriple;
use crate::snapshot::SnapshotStrategy;

pub struct EntityActor<C, E: DomainEvent, S> {
    pub entity_id: EntityTriple,
    pub mailbox: mpsc::Receiver<CommandEnvelope<C>>,
    pub state: Option<S>,
    pub version: u64,
    pub lifecycle: LifecycleStateMachine,
    pub registry: Arc<EntityRegistry>,
    pub persistence: Arc<PersistenceFacade<E>>,
    pub publisher: Arc<dyn EventPublisher<E>>,
    pub snapshot_strategy: Box<dyn SnapshotStrategy>,
    pub entity_handler: Arc<dyn PersistentEntity<C, E, S>>,
    pub _phantom: PhantomData<(C, S)>,
}

impl<C, E, S> EntityActor<C, E, S>
where
    C: Send + 'static,
    E: DomainEvent + Clone + serde::de::DeserializeOwned + 'static,
    S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + 'static,
{
    pub async fn run(&mut self) {
        self.recover_state().await;
        if self.lifecycle.state() == LifecycleState::Failed {
            self.registry.remove_active(&self.entity_id).await;
            return;
        }
        self.process_commands().await;
        self.passivate().await;
    }

    async fn recover_state(&mut self) {
        let load_result = self.persistence.load_for_recovery(
            &self.entity_id.aggregate_id(),
            Some(&self.entity_id.tenant_id),
        );

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

                for stored in &stored_events {
                    let new_state = self.entity_handler.apply_event(
                        &state,
                        stored.event.clone(),
                    ).await;
                    state = new_state;
                    version += 1;
                }

                self.state = Some(state);
                self.version = version;
                let _ = self.lifecycle.transition_to(LifecycleState::Active);
            }
            Err(e) => {
                let _ = self.lifecycle.transition_to(LifecycleState::Failed);
                log::error!("Recovery failed for {}: {}", self.entity_id.aggregate_id(), e);
            }
        }
    }

    async fn process_commands(&mut self) {
        let timeout = std::time::Duration::from_secs(300);

        loop {
            tokio::select! {
                Some(envelope) = self.mailbox.recv() => {
                    self.execute_command(envelope).await;
                    if self.lifecycle.should_passivate(timeout) {
                        break;
                    }
                }
                _ = tokio::time::sleep(timeout) => {
                    if self.lifecycle.should_passivate(timeout) {
                        break;
                    }
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
            &current_state,
            envelope.command,
            envelope.ctx.clone(),
        ).await;

        let response: Result<CommandErasedResult, EntityError> = match handler_result {
            Ok(events) if events.is_empty() => {
                Ok(Box::<CommandResult<E, S>>::new(CommandResult::NoEvents { state: current_state }) as CommandErasedResult)
            }
            Ok(events) => {
                let persist_result = self.persistence.persist_events(
                    &self.entity_id.aggregate_id(),
                    Some(&self.entity_id.tenant_id),
                    self.version,
                    events,
                );

                match persist_result {
                    Ok(new_version) => {
                        let reload = self.persistence.load_for_recovery(
                            &self.entity_id.aggregate_id(),
                            Some(&self.entity_id.tenant_id),
                        );
                        let (snap_data, stored_events) = reload.unwrap_or((None, Vec::new()));
                        let mut state = match snap_data {
                            Some(snap) => {
                                serde_json::from_slice(&snap.data).unwrap_or_else(|_| {
                                    self.entity_handler.initial_state()
                                })
                            }
                            None => self.entity_handler.initial_state(),
                        };
                        for stored in &stored_events {
                            let new_state = self.entity_handler.apply_event(
                                &state,
                                stored.event.clone(),
                            ).await;
                            state = new_state;
                        }
                        self.state = Some(state.clone());
                        self.version = new_version;

                        if self.snapshot_strategy.should_snapshot(new_version) {
                            let _ = self.persistence.store_snapshot(
                                &self.entity_id.aggregate_id(),
                                Some(&self.entity_id.tenant_id),
                                new_version,
                                &serde_json::to_value(&state).unwrap_or(serde_json::Value::Null),
                            );
                        }

                        let published_events: Vec<E> = stored_events.iter().map(|s| s.event.clone()).collect();
                        let _ = self.publisher.publish(&published_events).await;

                        Ok(Box::new(CommandResult::Events {
                            events: published_events,
                            new_state: state,
                            new_version,
                        }) as CommandErasedResult)
                    }
                    Err(e @ EntityError::VersionConflict { .. }) => {
                        Err(e)
                    }
                    Err(e) => {
                        let _ = self.lifecycle.transition_to(LifecycleState::Failed);
                        Err(EntityError::Runtime(e.to_string()))
                    }
                }
            }
            Err(err_string) => {
                Err(EntityError::Handler(err_string))
            }
        };

        let _ = envelope.response_tx.send(response);
    }

    async fn passivate(&mut self) {
        let _ = self.lifecycle.transition_to(LifecycleState::Passivating);

        while let Some(envelope) = self.mailbox.recv().await {
            self.execute_command(envelope).await;
        }

        if let Some(state) = &self.state {
            let _ = self.persistence.store_snapshot(
                &self.entity_id.aggregate_id(),
                Some(&self.entity_id.tenant_id),
                self.version,
                &serde_json::to_value(state).unwrap_or(serde_json::Value::Null),
            );
        }

        self.registry
            .mark_passivated(self.entity_id.clone(), self.version)
            .await;

        let _ = self.lifecycle.transition_to(LifecycleState::Passivated);
    }
}
