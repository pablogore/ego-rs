//! Reactive Scheduler & Deterministic Projection Engine
//!
//! This crate provides a reactive scheduling layer that observes CORE-006 execution events,
//! maintains a deterministic SchedulerState, and produces advisory activation suggestions
//! via a pure SchedulingPolicy function.
//!
//! # Core Components
//!
//! - [`scheduler::Scheduler`]: Main scheduler that consumes events and produces suggestions
//! - [`state::SchedulerState`]: Deterministic projection state
//! - [`policy::SchedulingPolicy`]: Trait for activation suggestion algorithms
//! - [`policy::RoundRobin`]: Built-in round-robin scheduling policy
//! - [`event_bus`]: Bounded event bus for observing events
//!
//! # Architecture Invariants
//!
//! 1. **Observed stream determinism**: SchedulerState is a pure function of the observed event stream
//! 2. **Per-actor ordering only**: No global ordering enforced
//! 3. **Non-self-healing**: Recovery is external to the scheduler
//!
//! # Usage
//!
//! ```rust,ignore
//! use ego_scheduler::scheduler::Scheduler;
//! use ego_scheduler::state::SchedulerState;
//! use ego_scheduler::policy::RoundRobin;
//! use ego_scheduler::event_bus::{event_bus_channel_with_config, EventBusConfig, DropPolicy};
//! use ego_scheduler::types::EntityTriple;
//! use ego_scheduler::event::SchedulerEvent;
//! use ego_scheduler::event::SchedulerEventEnvelope;
//! use std::collections::HashSet;
//!
//! // Create event bus
//! let config = EventBusConfig { capacity: 4096, drop_policy: DropPolicy::Block };
//! let (sender, receiver) = event_bus_channel_with_config(config);
//!
//! // Create scheduler with round-robin policy
//! let policy = RoundRobin;
//! let mut scheduler = Scheduler::new(SchedulerState::new(), receiver, Box::new(policy));
//!
//! // Create an event and send it
//! let entity = EntityTriple {
//!     tenant: "tenant1".to_string(),
//!     entity_type: "actor".to_string(),
//!     entity_id: "actor1".to_string(),
//! };
//! let event = SchedulerEvent::ExecutionCompleted {
//!     entity: entity.clone(),
//!     state_version: 1,
//! };
//! let envelope = SchedulerEventEnvelope::new(event, entity, 1);
//! sender.send(envelope).await.unwrap();
//!
//! // Process events and get suggestions
//! // Note: drain_and_apply() and suggest_activation() are async functions
//! // In practice, you would await these in an async context
//! ```

pub mod error;
pub mod types;
pub mod event;
pub mod event_bus;
pub mod state;
pub mod policy;
pub mod scheduler;
pub mod suggestion;
pub mod gap;
pub mod metric;

#[cfg(test)]
mod tests {

    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}