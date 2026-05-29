use ego_domain::{ActorId, ActorLifecycleState, SupervisionStrategy};
use std::fmt::Debug;

/// The runtime abstraction contract.
///
/// This trait is the platform entry point for executing actors. It is
/// backend-agnostic and does not reference actor types directly.
pub trait Runtime: Debug {
    /// The unique identifier for an execution.
    type ExecutionId: Debug + Clone + PartialEq + Eq + Send + Sync;

    /// The handle for managing an execution.
    type ExecutionHandle: Debug + Clone + Send + Sync;

    /// The error type for runtime operations.
    type SendError: Debug + Send + Sync;

    /// The state of an execution.
    type ExecutionState: Debug + Clone + Send + Sync;

    /// The isolation strategy for an actor.
    type Isolation: Debug + Clone + Send + Sync;

    /// The scheduling policy for actor execution.
    type SchedulingPolicy: Debug + Clone + Send + Sync;

    /// The error type for runtime operations.
    type RuntimeError: Debug + Send + Sync;

    /// Spawn a new actor with the given ID and lifecycle state.
    fn spawn(
        &self,
        actor_id: ActorId,
        lifecycle_state: ActorLifecycleState,
        isolation: Self::Isolation,
        scheduling: Self::SchedulingPolicy,
    ) -> Result<Self::ExecutionHandle, Self::RuntimeError>;

    /// Send a message to an actor.
    fn send(
        &self,
        handle: Self::ExecutionHandle,
        message: Box<dyn std::any::Any + Send + Sync>,
    ) -> Result<(), Self::SendError>;

    /// Get the current state of an execution.
    fn get_state(
        &self,
        handle: Self::ExecutionHandle,
    ) -> Result<Self::ExecutionState, Self::RuntimeError>;

    /// Stop an actor gracefully.
    fn stop(&self, handle: Self::ExecutionHandle) -> Result<(), Self::RuntimeError>;

    /// Restart an actor.
    fn restart(&self, handle: Self::ExecutionHandle) -> Result<(), Self::RuntimeError>;

    /// Escalate a failure to the parent supervisor.
    fn escalate(&self, handle: Self::ExecutionHandle) -> Result<(), Self::RuntimeError>;
}
