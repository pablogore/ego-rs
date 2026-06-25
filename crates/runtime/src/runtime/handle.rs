//! Scoped execution handle.
//!
//! Provides `RuntimeHandle`, a closure-based handle that gives an execution
//! self-access to send, shutdown, and state operations without requiring
//! a `dyn Runtime` reference.

use std::any::Any;
use std::sync::Arc;

use crate::runtime::execution::ExecutionId;
use crate::runtime::failure::SendError;
use crate::runtime::lifecycle::ExecutionState;

type SendFn =
    Arc<dyn Fn(&ExecutionId, Box<dyn Any + Send + 'static>) -> Result<(), SendError> + Send + Sync>;
type ShutdownFn = Arc<dyn Fn(&ExecutionId) + Send + Sync>;
type StateFn = Arc<dyn Fn(&ExecutionId) -> Option<ExecutionState> + Send + Sync>;

/// A handle to a running execution, providing closure-based scoped access.
///
/// `RuntimeHandle` does not store a `dyn Runtime` reference. Instead, it holds
/// boxed closures that capture the runtime's send, shutdown, and state
/// operations. This avoids the need for object-safety on the `Runtime` trait.
#[derive(Clone)]
pub struct RuntimeHandle {
    id: ExecutionId,
    send: SendFn,
    shutdown: ShutdownFn,
    state: StateFn,
}

impl RuntimeHandle {
    /// Constructor and accessor methods for `RuntimeHandle`.
    /// Create a new `RuntimeHandle` with the given execution id and closure
    /// callbacks.
    ///
    /// # Arguments
    /// * `id` — the execution id this handle wraps.
    /// * `send` — closure that dispatches a message to this execution.
    /// * `shutdown` — closure that requests graceful shutdown.
    /// * `state` — closure that queries the execution's current state.
    pub fn new<F, S, St>(id: ExecutionId, send: F, shutdown: S, state: St) -> Self
    where
        F: Fn(&ExecutionId, Box<dyn Any + Send + 'static>) -> Result<(), SendError>
            + Send
            + Sync
            + 'static,
        S: Fn(&ExecutionId) + Send + Sync + 'static,
        St: Fn(&ExecutionId) -> Option<ExecutionState> + Send + Sync + 'static,
    {
        Self {
            id,
            send: Arc::new(send),
            shutdown: Arc::new(shutdown),
            state: Arc::new(state),
        }
    }

    /// Return the execution id associated with this handle.
    pub fn id(&self) -> ExecutionId {
        self.id
    }

    /// Send a message to this execution.
    ///
    /// The message is boxed internally and dispatched via the closure stored
    /// in this handle. No `dyn Runtime` is involved.
    ///
    /// # Errors
    /// Returns `SendError::NotFound` if the execution no longer exists,
    /// or `SendError::Closed` if it is not accepting messages.
    pub fn send_self<M>(&self, msg: M) -> Result<(), SendError>
    where
        M: Send + 'static,
    {
        (self.send)(&self.id, Box::new(msg))
    }

    /// Request graceful shutdown of this execution.
    ///
    /// Transitions the execution to `Draining` state. In-flight messages
    /// are completed before final termination.
    pub fn shutdown(&self) {
        (self.shutdown)(&self.id)
    }

    /// Return the current state of this execution, if it exists.
    ///
    /// Returns `None` if the execution has not been spawned or has been
    /// fully removed.
    pub fn state(&self) -> Option<ExecutionState> {
        (self.state)(&self.id)
    }
}

