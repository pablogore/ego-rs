//! Thin orchestration pipeline — composition only, no business logic.
//!
//! # Architecture
//! The Scheduler is a fixed orchestration shell that composes 6 pure pipeline stages.
//! It MUST NOT contain domain logic, branching, or decision-making.
//! Pipeline order: ingest → route → reduce → detect → evaluate → emit.
//!
//! # Invariants
//! - I1: Determinism — SchedulerState = f(observed_stream)
//! - I2: Per-entity ordering — entity switch detection in route stage
//! - I3: Advisory output — suggestion is never a command
//! - I6: Single-consumer bus — Scheduler owns receiver exclusively
//!
//! # Drift Detection
//! If ANY logic beyond function calls appears in Scheduler: STOP and refactor into a module.

use crate::event_bus::{EntityTriple, SchedulerEventReceiver};
use crate::metric;
use crate::policy::SchedulingPolicy;
use crate::state::SchedulerState;
use std::collections::BTreeSet;

mod detect;
mod emit;
mod evaluate;
mod ingest;
mod reduce;
mod route;

/// Thin orchestrator that composes 6 pure pipeline components.
/// No domain logic — composition only.
pub struct Scheduler {
    state: SchedulerState,
    receiver: SchedulerEventReceiver,
    policy: Box<dyn SchedulingPolicy>,
    current_entity: Option<EntityTriple>,
    pending: BTreeSet<EntityTriple>,
}

impl Scheduler {
    /// Creates a new Scheduler with a receiver and policy.
    pub fn new(receiver: SchedulerEventReceiver, policy: Box<dyn SchedulingPolicy>) -> Self {
        Self {
            state: SchedulerState::new(),
            receiver,
            policy,
            current_entity: None,
            pending: BTreeSet::new(),
        }
    }

    /// Runs one pipeline cycle: drain → detect → route → reduce → evaluate → emit.
    /// Returns an advisory suggestion if one was produced.
    pub fn run_cycle(&mut self) -> Option<EntityTriple> {
        let events = ingest::drain(&mut self.receiver);
        if events.is_empty() {
            return None;
        }

        for item in &events {
            self.pending.insert(item.event.source_actor.clone());
        }

        detect::detect(&events, &mut self.state);

        let routed = route::route(events, &mut self.state, &mut self.current_entity);

        reduce::apply(routed, &mut self.state);

        metric::log_event_consumed(self.state.total_events_consumed);

        let suggestion = evaluate::evaluate(self.policy.as_ref(), &self.state, &self.pending);
        emit::emit(&mut self.state, suggestion.clone());

        if let Some(ref entity) = suggestion {
            self.pending.remove(entity);
        }

        suggestion
    }

    /// Returns a reference to the current SchedulerState.
    pub fn state(&self) -> &SchedulerState {
        &self.state
    }

    /// Returns a reference to the pending entity set.
    pub fn pending(&self) -> &BTreeSet<EntityTriple> {
        &self.pending
    }
}
