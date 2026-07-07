//! Entity actor — the core execution unit for persistent entities.
//!
//! The [`EntityActor`] owns the lifecycle of a single entity instance:
//! recovery, command processing, passivation, and failure handling.

use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::event::DomainEvent;
use tokio::sync::watch;

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
    /// Write side of this entry's published-state cell (ADR-003). The actor
    /// is the sole writer during normal operation; publishes on every
    /// `transition_to(_)`. [`crate::entity_ref_tokio::TeardownGuard`] holds a
    /// clone as a Drop-time backstop for the panic/cancellation case where
    /// the actor never gets to publish anything itself.
    pub(crate) tx: watch::Sender<EntityState>,
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
    /// Transitions the lifecycle state machine and publishes the new state
    /// through `tx` (ADR-003) in one step, so every call site that mutates
    /// lifecycle also keeps the registry's read view current.
    fn transition(&mut self, state: EntityState) {
        let _ = self.lifecycle.transition_to(state);
        let _ = self.tx.send(state);
    }

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
                .map_err(|e| format!("event replay failed: {e}"))?;
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
                self.transition(EntityState::Active);

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
                self.transition(EntityState::Failed);
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
                                self.transition(EntityState::Failed);
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
                        self.transition(EntityState::Failed);
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

    /// Closes the mailbox and drains all pending envelopes by sending `err`
    /// to each caller. This is a prompt, best-effort drain — the guaranteed
    /// path (ADR-005, FR-009) is `TeardownGuard::drop()`, which answers
    /// whatever this loop didn't reach (including if this loop itself
    /// panics). Registry-entry removal is no longer done here; it flows
    /// exclusively through the guard once `run()` returns.
    async fn drain_mailbox_with_error(&mut self, err: crate::error::EntityError) {
        self.mailbox.close();
        while let Ok(envelope) = self.mailbox.recv().await {
            let _ = envelope.reply.send(Err(err.clone()));
        }
    }

    /// Drains the mailbox, snapshots state, and marks the entity passivated in the registry.
    ///
    /// Registry-entry removal is no longer done here; it flows exclusively
    /// through `TeardownGuard::drop()` (ADR-005) once `run()` returns.
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
                self.registry
                    .mark_passivated(self.entity_id.aggregate_id(), self.version);
                return;
            }
        }

        self.transition(EntityState::Passivating);

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

        self.transition(EntityState::Passivated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_context::CommandContext;
    use crate::command_envelope::{ActorEnvelope, CommandEnvelope};
    use crate::entity_ref_tokio::TeardownGuard;
    use crate::passivation_signal::ManualSignal;
    use crate::persistence::PersistenceFacade;
    use crate::scheduler_event::event_bus_channel;
    use crate::snapshot::NoSnapshot;
    use crate::testing::{NoopPublisher, TestState};
    use async_trait::async_trait;
    use std::time::Duration;
    use tokio::sync::oneshot;
    use tokio::sync::watch;

    fn ctx() -> CommandContext {
        crate::testing::create_test_context()
    }

    /// TASK-008 probe command: `Boom` panics inside `handle_command` itself —
    /// a real panic raised by production code (`execute_command`'s call into
    /// the handler), not a simulated/injected one.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
    enum ProbeCommand {
        Noop,
        Boom,
    }

    #[derive(Debug)]
    struct PanicOnBoomHandler;

    #[async_trait]
    impl PersistentEntity for PanicOnBoomHandler {
        type Command = ProbeCommand;
        type Event = crate::testing::TestEvent;
        type State = TestState;

        fn initial_state(&self) -> TestState {
            TestState::new(0)
        }

        async fn handle_command(
            &self,
            command: &ProbeCommand,
            _state: &TestState,
            _context: &CommandContext,
        ) -> Result<Vec<crate::testing::TestEvent>, crate::error::EntityError> {
            match command {
                ProbeCommand::Noop => Ok(vec![]),
                ProbeCommand::Boom => panic!("TASK-008: intentional panic mid-processing"),
            }
        }

        async fn apply_event(
            &self,
            state: &TestState,
            _event: &crate::testing::TestEvent,
        ) -> Result<TestState, crate::error::EntityError> {
            Ok(state.clone())
        }

        async fn apply_events(
            &self,
            state: &TestState,
            _events: &[crate::testing::TestEvent],
        ) -> Result<TestState, crate::error::EntityError> {
            Ok(state.clone())
        }
    }

    /// TASK-008 / FR-009: an actor `Active` with N commands already queued
    /// behind the one currently being processed must terminally answer all N
    /// queued callers even when the in-processing command panics.
    ///
    /// The panic is real: `PanicOnBoomHandler::handle_command` calls
    /// `panic!()` directly, which `execute_command` invokes via
    /// `entity_handler.handle_command(...)` in the normal, unmodified
    /// production path — nothing here catches or redirects the panic before
    /// it unwinds through `process_commands` -> `run` -> the spawned task.
    #[tokio::test(flavor = "multi_thread")]
    async fn panic_mid_processing_answers_all_already_enqueued_callers() {
        const N: usize = 5;
        let mailbox: BoundedMailbox<ActorEnvelope<ProbeCommand>> = BoundedMailbox::new(N + 1);

        // The command currently being processed at panic time.
        let (panic_tx, panic_rx) = oneshot::channel();
        mailbox
            .send(ActorEnvelope {
                envelope: CommandEnvelope {
                    command: ProbeCommand::Boom,
                    context: ctx(),
                },
                reply: panic_tx,
            })
            .await
            .expect("queueing the panicking command must succeed");

        // N commands already enqueued behind it.
        let mut queued_rxs = Vec::with_capacity(N);
        for _ in 0..N {
            let (tx, rx) = oneshot::channel();
            mailbox
                .send(ActorEnvelope {
                    envelope: CommandEnvelope {
                        command: ProbeCommand::Noop,
                        context: ctx(),
                    },
                    reply: tx,
                })
                .await
                .expect("queueing a trailing command must succeed");
            queued_rxs.push(rx);
        }

        // Keep an independent handle to the queue alive, exactly like a real
        // caller's `TokioEntityRef` clone would — otherwise the actor's own
        // `mailbox` field is the *only* remaining reference and dropping it
        // during unwind would deallocate the queue itself (dropping the
        // queued oneshots as a side effect of Arc teardown, not because
        // anything actually answered them). Holding this clone reproduces
        // the real bug: the queue survives the panic, untouched, and every
        // queued oneshot hangs forever without a guard to drain it.
        let _caller_side_mailbox = mailbox.clone();

        let (event_sender, _rx) = event_bus_channel();
        let registry = Arc::new(EntityRegistry::new());
        let entity_id = EntityTriple::new("default".to_string(), "probe", "actor-panic-1");
        let (tx, _rx_watch) = watch::channel(EntityState::Recovering);

        // Wire the actor + TeardownGuard exactly like the production spawn
        // path in `entity_ref_tokio.rs::TokioEntityRef::new` — the guarantee
        // under test comes from the guard, not from anything inside
        // `EntityActor::run()` itself.
        let mut actor = EntityActor {
            entity_id: entity_id.clone(),
            mailbox: mailbox.clone(),
            state: None,
            version: 0,
            lifecycle: LifecycleStateMachine::new(),
            registry: registry.clone(),
            tx: tx.clone(),
            persistence: Arc::new(PersistenceFacade::new()),
            publisher: Arc::new(NoopPublisher::new()),
            snapshot_strategy: Arc::new(NoSnapshot),
            entity_handler: Arc::new(PanicOnBoomHandler),
            event_sender,
            signal: ManualSignal::new(),
            _phantom: PhantomData,
        };
        let guard = TeardownGuard {
            aggregate_id: entity_id.aggregate_id(),
            registry,
            epoch: 0,
            mailbox,
            tx,
        };

        let join_result = tokio::spawn(async move {
            actor.run().await;
            drop(guard);
        })
        .await;
        assert!(
            join_result.is_err(),
            "the actor task must actually have panicked"
        );

        // The in-flight command's own reply Sender lives on the unwinding
        // stack frame; dropping it on unwind closes the channel.
        assert!(
            panic_rx.await.is_err(),
            "the in-flight command's reply channel must close on panic-unwind"
        );

        for rx in queued_rxs {
            let resolved = tokio::time::timeout(Duration::from_secs(5), rx)
                .await
                .expect("FR-009: every queued caller must eventually resolve, not hang forever");
            let terminal = resolved.expect("oneshot sender must not be dropped without a value");
            assert!(
                terminal.is_err(),
                "a queued command whose actor died mid-processing must resolve to a terminal Err"
            );
        }
    }
}
