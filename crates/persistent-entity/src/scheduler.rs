use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::registry::EntityRegistry;
use crate::scheduler_event::{SchedulerEventReceiver, SchedulerState};
use crate::scheduler_policy::SchedulingPolicy;

/// A simple entity triple identifier.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct EntityTriple {
    /// The tenant identifier.
    pub tenant_id: String,
    /// The entity type.
    pub entity_type: String,
    /// The entity identifier.
    pub entity_id: String,
}

impl EntityTriple {
    /// Create a new entity triple.
    pub fn new(tenant_id: String, entity_type: &str, entity_id: impl Into<String>) -> Self {
        Self {
            tenant_id,
            entity_type: entity_type.to_string(),
            entity_id: entity_id.into(),
        }
    }

    /// Get the aggregate identifier.
    pub fn aggregate_id(&self) -> String {
        format!("{}-{}", self.entity_type, self.entity_id)
    }
}

// ---------------------------------------------------------------------------
// SchedulerInner — single Mutex guards both receiver and state
// ---------------------------------------------------------------------------

/// All mutable scheduler state behind a single Mutex.
///
/// This eliminates lock ordering hazards and ensures that the receiver
/// drain and state update are atomic from the scheduler's perspective.
struct SchedulerInner {
    receiver: SchedulerEventReceiver,
    state: SchedulerState,
}

impl SchedulerInner {
    fn new(receiver: SchedulerEventReceiver) -> Self {
        Self {
            receiver,
            state: SchedulerState::new(),
        }
    }

    /// Drain all pending events and apply them to internal state.
    /// Returns the number of events consumed.
    fn drain_and_update(&mut self) -> usize {
        let events = self.receiver.drain_all();
        let count = events.len();
        self.state.apply_drained_events(events);
        count
    }
}

// ---------------------------------------------------------------------------
// Scheduler — reactive policy engine
// ---------------------------------------------------------------------------

/// Reactive policy engine for entity activation suggestions.
///
/// The Scheduler is NOT an execution authority. It evaluates policy
/// and emits recommendations. The Actor is the sole execution authority.
///
/// ## Concurrency model
///
/// All mutable state (event receiver + scheduler state) is protected by a
/// single `Mutex` to guarantee:
///   - Draining and state update are atomic
///   - No lock ordering hazards
///   - Deterministic single-threaded access to policy inputs
///
/// The Mutex is held only during synchronous operations and is NEVER
/// held across await points.
pub struct Scheduler {
    /// The entity registry.
    pub registry: Arc<EntityRegistry>,
    /// The scheduling policy for activation suggestions.
    pub policy: Arc<dyn SchedulingPolicy>,
    /// Inner state: receiver + SchedulerState behind a single lock.
    inner: Mutex<SchedulerInner>,
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("registry", &self.registry)
            .field("policy", &self.policy)
            .finish()
    }
}

impl Scheduler {
    /// Create a new scheduler.
    pub fn new(
        registry: Arc<EntityRegistry>,
        policy: Arc<dyn SchedulingPolicy>,
        event_receiver: SchedulerEventReceiver,
    ) -> Self {
        Self {
            registry,
            policy,
            inner: Mutex::new(SchedulerInner::new(event_receiver)),
        }
    }

    /// Drain all pending feedback events from the Actor.
    ///
    /// Updates internal state counters and replay buffer atomically.
    /// Returns the number of events consumed.
    pub fn drain_pending_events(&self) -> usize {
        let mut inner = self.inner.lock().unwrap();
        inner.drain_and_update()
    }

    /// Access the current scheduler state snapshot.
    ///
    /// Drains pending events first to ensure the state is current.
    pub fn scheduler_state(&self) -> SchedulerState {
        let mut inner = self.inner.lock().unwrap();
        inner.drain_and_update();
        inner.state.clone()
    }

    /// Reset the replay buffer in scheduler state.
    pub fn clear_replay_buffer(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.state.clear_replay_buffer();
    }

    /// Suggest which entity to activate next based on policy.
    ///
    /// All pending feedback events are drained before computing the
    /// recommendation, ensuring the policy has the most current view.
    ///
    /// ## Determinism
    ///
    /// Same input sequence + same drained events = same recommendation.
    /// The drain is unconditional — every call consumes all available
    /// events before computing the suggestion.
    ///
    /// This is a pure advisory recommendation. The caller (Actor or Runtime)
    /// is the sole execution authority and may accept or ignore it.
    pub fn suggest_activation(&self, pending: &HashSet<EntityTriple>) -> Option<EntityTriple> {
        let mut inner = self.inner.lock().unwrap();
        inner.drain_and_update();
        let suggestion = self.policy.suggest_activation(pending);
        inner.state.last_suggestion = suggestion.clone();
        suggestion
    }
}
