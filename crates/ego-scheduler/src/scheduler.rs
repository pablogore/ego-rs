//! Main scheduler implementation.

use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::state::SchedulerState;
use crate::policy::SchedulingPolicy;
use crate::event_bus::SchedulerEventReceiver;
use crate::types::EntityTriple;
use crate::metric::SchedulerMetrics;

/// The main scheduler that consumes events and produces activation suggestions.
pub struct Scheduler {
    /// The current state of the scheduler.
    state: Arc<Mutex<SchedulerState>>,

    /// The receiver for events.
    receiver: SchedulerEventReceiver,

    /// The scheduling policy to use.
    policy: Box<dyn SchedulingPolicy>,

    /// Metrics collected by the scheduler.
    metrics: Arc<Mutex<SchedulerMetrics>>,
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
            metrics: Arc::new(Mutex::new(SchedulerMetrics::default())),
        }
    }

    /// Drains and applies all available events from the event bus.
    pub async fn drain_and_apply(&mut self) {
        let mut state = self.state.lock().await;
        let mut metrics = self.metrics.lock().await;
        while let Some(envelope) = self.receiver.recv().await {
            state.apply_event(&envelope);
            metrics.increment_events_consumed();
            
            // Check for gaps and update metrics
            if let Some(last_seq) = state.last_sequence_id {
                if let Some(actor_seq) = state.actor_sequences.get(&envelope.source_actor) {
                    if *actor_seq != last_seq {
                        // Gap detected - update metrics
                        metrics.increment_gaps_detected();
                    }
                }
            }
        }
    }

    /// Suggests an entity to activate based on the current state and pending entities.
    pub async fn suggest_activation(
        &self,
        pending_entities: &HashSet<EntityTriple>,
    ) -> Option<EntityTriple> {
        let state = self.state.lock().await;
        let mut metrics = self.metrics.lock().await;
        let suggestion = self.policy.suggest_activation(&state, pending_entities);
        metrics.increment_suggestions_made();
        suggestion
    }
    
    /// Gets the current metrics.
    pub async fn get_metrics(&self) -> SchedulerMetrics {
        self.metrics.lock().await.clone()
    }
}