//! Entity actor — the core execution unit for persistent entities.
//!
//! The [`EntityActor`] owns the lifecycle of a single entity instance:
//! recovery, command processing, passivation, and failure handling.

use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::event::DomainEvent;
use tokio::sync::watch;

use crate::command_envelope::{ActorEnvelope, CommandEnvelope};
use crate::effect_acceptor::EffectAcceptor;
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
    /// Post-commit external-effect acceptance port (AD-1, AD-3). `None` when
    /// no effect delivery subsystem is configured — keeps the cost of this
    /// capability at zero for entities/deployments that never describe
    /// effects (AD-2). Set by whoever wires the runtime lifecycle (Phase 9).
    pub(crate) effect_acceptor: Option<Arc<dyn EffectAcceptor>>,
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
                self.entity_id.aggregate_type(),
                &self.entity_id.entity_id,
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

        // Idempotency gate, before dispatch and before anything is persisted.
        //
        // Both halves of the identity must be present. `operation_key` says
        // which operation this is; `fingerprint` says which *request* it came
        // from, and without it a retry cannot be told apart from a different
        // command reusing the key. A command carrying neither takes the
        // pre-existing path untouched and pays for no lookup.
        if let (Some(key), Some(fingerprint)) = (&context.operation_key, &context.fingerprint) {
            let found = self
                .persistence
                .find_receipt(
                    self.entity_id.aggregate_type(),
                    &self.entity_id.entity_id,
                    Some(&self.entity_id.tenant_id),
                    key.as_str(),
                )
                .await;

            match found {
                // A lookup that failed is not a miss. A miss means "run the
                // command", so falling through to the handler here would
                // re-execute an operation that may already have completed —
                // the exact duplicate the receipt exists to prevent.
                Err(e) => {
                    let _ = reply.send(Err(crate::error::EntityError::PersistenceError(format!(
                        "could not read the operation receipt: {e}"
                    ))));
                    return;
                }
                Ok(Some(receipt)) if receipt.fingerprint() == fingerprint => {
                    // The same request, arriving again. Nothing runs, nothing
                    // is written, and no effect is accepted a second time. The
                    // outcome travels as durable evidence, never as a rebuilt
                    // result: see `CommandResult::Replayed`.
                    let result: CommandResult<E, S> = CommandResult::Replayed {
                        outcome: receipt.outcome().clone(),
                    };
                    let boxed: CommandErasedResult = Box::new(result);
                    let _ = reply.send(Ok(boxed));
                    return;
                }
                Ok(Some(_)) => {
                    // A different request reusing an operation key. Refused
                    // permanently, and `handle_command` is never reached:
                    // executing it would let one caller's key drive another
                    // caller's command.
                    let _ = reply.send(Err(crate::error::EntityError::OperationConflict {
                        operation_key: key.as_str().to_string(),
                    }));
                    return;
                }
                Ok(None) => {}
            }
        }

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
                        self.entity_id.aggregate_type(),
                        &self.entity_id.entity_id,
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
                                let _ = reply.send(Err(
                                    crate::error::EntityError::PersistenceError(e.to_string()),
                                ));
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

                        // AD-1: the acceptance seam is the actor's own
                        // post-persist sequence, right beside the
                        // fire-and-forget publish above — never the
                        // dormant EffectInterpreter arm. `external_effects`
                        // receives the just-committed `state`/`events`, not
                        // the pre-command state (AD-2).
                        let described_effects = self
                            .entity_handler
                            .external_effects(&command, &state, &events, &context)
                            .await;

                        if !described_effects.is_empty() {
                            // Backpressure/acceptance-failure-retry delays
                            // the reply below, never the commit above, which
                            // has already completed (AD-9, spec: "Acceptance
                            // backpressure delays the reply, not the
                            // commit"). F-03 (PR3 review): a handler that
                            // opts into describing effects on an actor with
                            // no `effect_acceptor` configured must fail
                            // closed, not silently discard the effects and
                            // reply as if nothing was described.
                            let first_idempotency_key =
                                described_effects[0].idempotency_key.clone();
                            let acceptance = match &self.effect_acceptor {
                                Some(acceptor) => match ego_domain::TenantId::new(
                                    self.entity_id.tenant_id.clone(),
                                ) {
                                    Ok(tenant) => acceptor.accept(&tenant, described_effects).await,
                                    Err(_) => {
                                        Err(crate::effect_acceptor::EffectAcceptanceError::Permanent {
                                            message: format!(
                                                "invalid tenant identity '{}' for effect acceptance",
                                                self.entity_id.tenant_id
                                            ),
                                            failed_at_index: 0,
                                            failed_idempotency_key: first_idempotency_key,
                                        })
                                    }
                                },
                                None => Err(crate::effect_acceptor::EffectAcceptanceError::Permanent {
                                    message: "external effects were described but no EffectAcceptor \
                                              is configured"
                                        .to_string(),
                                    failed_at_index: 0,
                                    failed_idempotency_key: first_idempotency_key,
                                }),
                            };

                            if let Err(error) = acceptance {
                                // AD-9 REQUIRED constraint: this is NOT a
                                // command failure — the commit above is final
                                // and was never rolled back.
                                // `CommandResult::EffectsAcceptanceFailed`
                                // keeps the reply `Ok(..)` while carrying a
                                // distinguishable outcome, so a caller can
                                // never mistake this for "not committed, safe
                                // to retry" and cause a duplicate command
                                // execution.
                                let result: CommandResult<E, S> =
                                    CommandResult::EffectsAcceptanceFailed {
                                        new_state: state,
                                        events,
                                        error,
                                    };
                                let boxed: CommandErasedResult = Box::new(result);
                                let _ = reply.send(Ok(boxed));
                                return;
                            }
                        }

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
                    let _ = envelope
                        .reply
                        .send(Err(crate::error::EntityError::PersistenceError(
                            "actor failed during passivation drain".to_string(),
                        )));
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
            effect_acceptor: None,
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

    // --- CORE-019 Phase 8: handler API + actor post-persist wiring ---

    use crate::effect_acceptor::EffectAcceptanceError;
    use crate::testing::{TestCommand, TestEvent};
    use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    fn sample_effect() -> ExternalEffectDescription {
        ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
            effect_type: "invoice.created".to_string(),
            payload: vec![],
            destination: "https://example.com".to_string(),
        }
    }

    /// Describes one external effect for `Increment`, none for `Decrement` —
    /// lets tests exercise both "acceptor called" and "acceptor skipped"
    /// paths through the same handler.
    #[derive(Debug)]
    struct EffectEmittingHandler;

    #[async_trait]
    impl PersistentEntity for EffectEmittingHandler {
        type Command = TestCommand;
        type Event = TestEvent;
        type State = TestState;

        fn initial_state(&self) -> TestState {
            TestState::new(0)
        }

        async fn handle_command(
            &self,
            command: &TestCommand,
            _state: &TestState,
            _context: &CommandContext,
        ) -> Result<Vec<TestEvent>, crate::error::EntityError> {
            match command {
                TestCommand::Increment(v) => Ok(vec![TestEvent::Incremented(*v)]),
                TestCommand::Decrement(v) => Ok(vec![TestEvent::Decremented(*v)]),
                TestCommand::GetState => Ok(vec![]),
            }
        }

        async fn apply_event(
            &self,
            state: &TestState,
            event: &TestEvent,
        ) -> Result<TestState, crate::error::EntityError> {
            match event {
                TestEvent::Incremented(v) => Ok(TestState {
                    value: state.value + v,
                    version: state.version + 1,
                }),
                TestEvent::Decremented(v) => Ok(TestState {
                    value: state.value.saturating_sub(*v),
                    version: state.version + 1,
                }),
            }
        }

        async fn apply_events(
            &self,
            state: &TestState,
            events: &[TestEvent],
        ) -> Result<TestState, crate::error::EntityError> {
            let mut s = state.clone();
            for event in events {
                s = self.apply_event(&s, event).await?;
            }
            Ok(s)
        }

        async fn external_effects(
            &self,
            command: &TestCommand,
            _new_state: &TestState,
            events: &[TestEvent],
            _context: &CommandContext,
        ) -> Vec<ExternalEffectDescription> {
            if events.is_empty() {
                return Vec::new();
            }
            match command {
                TestCommand::Increment(_) => vec![sample_effect()],
                _ => Vec::new(),
            }
        }
    }

    struct RecordingAcceptor {
        calls: Arc<AtomicUsize>,
        result: Result<(), EffectAcceptanceError>,
    }

    #[async_trait]
    impl EffectAcceptor for RecordingAcceptor {
        async fn accept(
            &self,
            _tenant: &TenantId,
            effects: Vec<ExternalEffectDescription>,
        ) -> Result<(), EffectAcceptanceError> {
            self.calls.fetch_add(effects.len(), Ordering::SeqCst);
            self.result.clone()
        }
    }

    struct GatedAcceptor {
        gate: Arc<Notify>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl EffectAcceptor for GatedAcceptor {
        async fn accept(
            &self,
            _tenant: &TenantId,
            effects: Vec<ExternalEffectDescription>,
        ) -> Result<(), EffectAcceptanceError> {
            self.calls.fetch_add(effects.len(), Ordering::SeqCst);
            self.gate.notified().await;
            Ok(())
        }
    }

    /// Builds a standalone actor (state pre-seeded, no recovery run) wired
    /// with `EffectEmittingHandler` and the given optional acceptor — enough
    /// to call `execute_command` directly without a full `run()` loop.
    fn build_effect_actor(
        effect_acceptor: Option<Arc<dyn EffectAcceptor>>,
    ) -> EntityActor<TestCommand, TestEvent, TestState, ManualSignal> {
        let (event_sender, _rx) = event_bus_channel();
        let registry = Arc::new(EntityRegistry::new());
        let entity_id = EntityTriple::new("tenant-x".to_string(), "probe", "actor-effects-1");
        let mailbox: BoundedMailbox<ActorEnvelope<TestCommand>> = BoundedMailbox::new(8);
        let (tx, _rx_watch) = watch::channel(EntityState::Recovering);

        EntityActor {
            entity_id,
            mailbox,
            state: Some(TestState::new(0)),
            version: 0,
            lifecycle: LifecycleStateMachine::new(),
            registry,
            tx,
            persistence: Arc::new(PersistenceFacade::new()),
            publisher: Arc::new(NoopPublisher::new()),
            effect_acceptor,
            snapshot_strategy: Arc::new(NoSnapshot),
            entity_handler: Arc::new(EffectEmittingHandler),
            event_sender,
            signal: ManualSignal::new(),
            _phantom: PhantomData,
        }
    }

    #[tokio::test]
    async fn handler_describing_effects_calls_acceptor_after_commit_before_reply() {
        let calls = Arc::new(AtomicUsize::new(0));
        let acceptor = Arc::new(RecordingAcceptor {
            calls: calls.clone(),
            result: Ok(()),
        });
        let mut actor = build_effect_actor(Some(acceptor));

        let (tx, rx) = oneshot::channel();
        let envelope = ActorEnvelope {
            envelope: CommandEnvelope {
                command: TestCommand::Increment(1),
                context: ctx(),
            },
            reply: tx,
        };
        actor.execute_command(envelope).await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the acceptor must be called exactly once for the one described effect"
        );
        assert!(
            rx.await.unwrap().is_ok(),
            "successful acceptance yields an Ok reply"
        );
    }

    #[tokio::test]
    async fn handler_returning_no_effects_never_calls_the_acceptor() {
        let calls = Arc::new(AtomicUsize::new(0));
        let acceptor = Arc::new(RecordingAcceptor {
            calls: calls.clone(),
            result: Ok(()),
        });
        let mut actor = build_effect_actor(Some(acceptor));

        let (tx, rx) = oneshot::channel();
        let envelope = ActorEnvelope {
            envelope: CommandEnvelope {
                command: TestCommand::Decrement(1),
                context: ctx(),
            },
            reply: tx,
        };
        actor.execute_command(envelope).await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "Decrement describes zero effects — the acceptor must never be touched"
        );
        assert!(rx.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn acceptance_failure_yields_committed_but_unaccepted_reply_not_a_command_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let acceptor = Arc::new(RecordingAcceptor {
            calls: calls.clone(),
            result: Err(EffectAcceptanceError::Permanent {
                message: "backend corrupt".to_string(),
                failed_at_index: 0,
                failed_idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
            }),
        });
        let mut actor = build_effect_actor(Some(acceptor));

        let (tx, rx) = oneshot::channel();
        let envelope = ActorEnvelope {
            envelope: CommandEnvelope {
                command: TestCommand::Increment(5),
                context: ctx(),
            },
            reply: tx,
        };
        actor.execute_command(envelope).await;

        // Commit is final regardless of acceptance outcome (AD-9) — the
        // actor's own in-memory state already reflects the committed event.
        assert_eq!(
            actor.state.as_ref().unwrap().value,
            5,
            "commit must never be rolled back on acceptance failure"
        );

        let reply = rx.await.unwrap();
        let boxed = reply.expect(
            "acceptance failure must still be a successful Ok reply, never Err(EntityError) — \
             collapsing it into Err would make it indistinguishable from a real command failure \
             and cause a caller to retry an already-committed command",
        );
        let result: Box<CommandResult<TestEvent, TestState>> = boxed
            .downcast()
            .expect("reply carries the expected CommandResult<TestEvent, TestState>");
        match *result {
            CommandResult::EffectsAcceptanceFailed {
                new_state,
                events,
                error,
            } => {
                assert_eq!(new_state.value, 5);
                assert_eq!(events.len(), 1);
                assert!(matches!(error, EffectAcceptanceError::Permanent { .. }));
            }
            other => panic!(
                "expected CommandResult::EffectsAcceptanceFailed, got a different variant: {other:?}"
            ),
        }
    }

    /// F-03 (PR3 review, BLOCKER): a handler that opts into describing
    /// effects, running on an actor with `effect_acceptor: None`, must fail
    /// closed (an honest `EffectsAcceptanceFailed` reply) rather than
    /// silently discarding the described effects and replying as if nothing
    /// had been described. The commit itself must remain intact regardless.
    #[tokio::test]
    async fn missing_acceptor_with_described_effects_fails_closed_not_silently_discarded() {
        let mut actor = build_effect_actor(None);

        let (tx, rx) = oneshot::channel();
        let envelope = ActorEnvelope {
            envelope: CommandEnvelope {
                command: TestCommand::Increment(3),
                context: ctx(),
            },
            reply: tx,
        };
        actor.execute_command(envelope).await;

        assert_eq!(
            actor.state.as_ref().unwrap().value,
            3,
            "commit must still happen even though no acceptor is configured"
        );

        let reply = rx.await.unwrap();
        let boxed = reply.expect(
            "must still be an Ok reply — the commit succeeded, only acceptance was impossible",
        );
        let result: Box<CommandResult<TestEvent, TestState>> = boxed
            .downcast()
            .expect("reply carries the expected CommandResult<TestEvent, TestState>");
        match *result {
            CommandResult::EffectsAcceptanceFailed {
                new_state,
                events,
                error,
            } => {
                assert_eq!(new_state.value, 3);
                assert_eq!(events.len(), 1);
                assert!(
                    matches!(error, EffectAcceptanceError::Permanent { .. }),
                    "a missing acceptor is a permanent acceptance failure, not retryable"
                );
            }
            other => panic!(
                "expected EffectsAcceptanceFailed when no acceptor is configured, \
                 got a different variant: {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn acceptance_backpressure_delays_reply_but_commit_already_happened() {
        let gate = Arc::new(Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let acceptor = Arc::new(GatedAcceptor {
            gate: gate.clone(),
            calls: calls.clone(),
        });
        let mut actor = build_effect_actor(Some(acceptor));

        let (tx, mut rx) = oneshot::channel();
        let envelope = ActorEnvelope {
            envelope: CommandEnvelope {
                command: TestCommand::Increment(7),
                context: ctx(),
            },
            reply: tx,
        };

        let handle = tokio::spawn(async move {
            actor.execute_command(envelope).await;
            actor
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            while calls.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the acceptor must be reached before the reply resolves");

        assert!(
            rx.try_recv().is_err(),
            "the reply must be delayed while acceptance is still pending"
        );

        gate.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(1), rx)
            .await
            .expect("reply eventually arrives once acceptance completes")
            .unwrap();
        assert!(result.is_ok());

        let actor = handle.await.unwrap();
        assert_eq!(
            actor.state.as_ref().unwrap().value,
            7,
            "the commit that happened before the gate must be reflected regardless of the delayed reply"
        );
    }

    // --- CommandContext carries the operation key through to the actor ---

    /// Records the exact `operation_key` `handle_command` was called with,
    /// so a test can compare it against the value set at the boundary rather
    /// than merely proving the field exists on `CommandContext`.
    #[derive(Debug)]
    struct RecordingContextHandler {
        seen_operation_key: Arc<std::sync::Mutex<Option<ego_domain::operation::OperationKey>>>,
    }

    #[async_trait]
    impl PersistentEntity for RecordingContextHandler {
        type Command = TestCommand;
        type Event = TestEvent;
        type State = TestState;

        fn initial_state(&self) -> TestState {
            TestState::new(0)
        }

        async fn handle_command(
            &self,
            _command: &TestCommand,
            _state: &TestState,
            context: &CommandContext,
        ) -> Result<Vec<TestEvent>, crate::error::EntityError> {
            *self.seen_operation_key.lock().unwrap() = context.operation_key.clone();
            Ok(vec![])
        }

        async fn apply_event(
            &self,
            state: &TestState,
            _event: &TestEvent,
        ) -> Result<TestState, crate::error::EntityError> {
            Ok(state.clone())
        }

        async fn apply_events(
            &self,
            state: &TestState,
            _events: &[TestEvent],
        ) -> Result<TestState, crate::error::EntityError> {
            Ok(state.clone())
        }
    }

    #[tokio::test]
    async fn command_context_operation_key_reaches_the_actor_unchanged() {
        use ego_domain::operation::OperationKey;

        let seen = Arc::new(std::sync::Mutex::new(None));
        let (event_sender, _rx) = event_bus_channel();
        let registry = Arc::new(EntityRegistry::new());
        let entity_id = EntityTriple::new("tenant-x".to_string(), "probe", "actor-opkey-1");
        let mailbox: BoundedMailbox<ActorEnvelope<TestCommand>> = BoundedMailbox::new(4);
        let (tx, _rx_watch) = watch::channel(EntityState::Recovering);

        let mut actor = EntityActor {
            entity_id,
            mailbox,
            state: Some(TestState::new(0)),
            version: 0,
            lifecycle: LifecycleStateMachine::new(),
            registry,
            tx,
            persistence: Arc::new(PersistenceFacade::new()),
            publisher: Arc::new(NoopPublisher::new()),
            effect_acceptor: None,
            snapshot_strategy: Arc::new(NoSnapshot),
            entity_handler: Arc::new(RecordingContextHandler {
                seen_operation_key: seen.clone(),
            }),
            event_sender,
            signal: ManualSignal::new(),
            _phantom: PhantomData,
        };

        let key = OperationKey::parse("op-carriage-1").unwrap();
        let mut context = ctx();
        context.operation_key = Some(key.clone());

        let (reply_tx, reply_rx) = oneshot::channel();
        let envelope = ActorEnvelope {
            envelope: CommandEnvelope {
                command: TestCommand::GetState,
                context,
            },
            reply: reply_tx,
        };

        actor.execute_command(envelope).await;
        reply_rx
            .await
            .expect("reply sender must not be dropped")
            .expect("a zero-event command must reply Ok");

        assert_eq!(
            seen.lock().unwrap().clone(),
            Some(key),
            "handle_command must observe the identical OperationKey set at the boundary, \
             not a regenerated, normalised, or reconstructed one"
        );
    }

    #[tokio::test]
    async fn command_context_with_no_operation_key_reaches_the_actor_as_none() {
        let seen = Arc::new(std::sync::Mutex::new(Some(
            ego_domain::operation::OperationKey::parse("sentinel").unwrap(),
        )));
        let (event_sender, _rx) = event_bus_channel();
        let registry = Arc::new(EntityRegistry::new());
        let entity_id = EntityTriple::new("tenant-x".to_string(), "probe", "actor-opkey-2");
        let mailbox: BoundedMailbox<ActorEnvelope<TestCommand>> = BoundedMailbox::new(4);
        let (tx, _rx_watch) = watch::channel(EntityState::Recovering);

        let mut actor = EntityActor {
            entity_id,
            mailbox,
            state: Some(TestState::new(0)),
            version: 0,
            lifecycle: LifecycleStateMachine::new(),
            registry,
            tx,
            persistence: Arc::new(PersistenceFacade::new()),
            publisher: Arc::new(NoopPublisher::new()),
            effect_acceptor: None,
            snapshot_strategy: Arc::new(NoSnapshot),
            entity_handler: Arc::new(RecordingContextHandler {
                seen_operation_key: seen.clone(),
            }),
            event_sender,
            signal: ManualSignal::new(),
            _phantom: PhantomData,
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        let envelope = ActorEnvelope {
            envelope: CommandEnvelope {
                command: TestCommand::GetState,
                context: ctx(),
            },
            reply: reply_tx,
        };

        actor.execute_command(envelope).await;
        reply_rx
            .await
            .expect("reply sender must not be dropped")
            .expect("a zero-event command must reply Ok");

        assert_eq!(
            seen.lock().unwrap().clone(),
            None,
            "an absent operation key must reach the actor as None, not a stale prior value"
        );
    }
}
