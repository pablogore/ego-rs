//! Main scheduler implementation.

use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::state::SchedulerState;
use crate::policy::SchedulingPolicy;
use crate::event_bus::SchedulerEventReceiver;
use crate::types::EntityTriple;

/// The main scheduler that consumes events and produces activation suggestions.
pub struct Scheduler {
    /// The current state of the scheduler.
    state: Arc<Mutex<SchedulerState>>,

    /// The receiver for events.
    receiver: SchedulerEventReceiver,

    /// The scheduling policy to use.
    policy: Box<dyn SchedulingPolicy>,
}

impl Scheduler {
    /// Creates a new scheduler with the given initial state, event receiver, and policy.
    pub fn new(
        initial_state: SchedulerState,
        receiver: SchedulerEventReceiver,
        policy: Box<dyn SchedulingPolicy>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(initial_state)),
            receiver,
            policy,
        }
    }

    /// Drains and applies all available events from the event bus.
    pub async fn drain_and_apply(&mut self) {
        let mut state = self.state.lock().await;
        while let Some(envelope) = self.receiver.recv().await {
            state.apply_event(&envelope);
        }
    }

    /// Suggests an entity to activate based on the current state and pending entities.
    pub async fn suggest_activation(
        &self,
        pending_entities: &HashSet<EntityTriple>,
    ) -> Option<EntityTriple> {
        let state = self.state.lock().await;
        self.policy.suggest_activation(&state, pending_entities)
    }
}