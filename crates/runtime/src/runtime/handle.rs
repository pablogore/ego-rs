use std::any::Any;
use std::sync::Arc;

use crate::runtime::execution::ExecutionId;
use crate::runtime::failure::SendError;
use crate::runtime::lifecycle::ExecutionState;

type SendFn = Arc<dyn Fn(&ExecutionId, Box<dyn Any + Send + 'static>) -> Result<(), SendError> + Send + Sync>;
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
    /// Create a new `RuntimeHandle` with the given execution id and closure
    /// callbacks.
    pub fn new<F, S, St>(
        id: ExecutionId,
        send: F,
        shutdown: S,
        state: St,
    ) -> Self
    where
        F: Fn(&ExecutionId, Box<dyn Any + Send + 'static>) -> Result<(), SendError> + Send + Sync + 'static,
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
    pub fn send_self<M>(&self, msg: M) -> Result<(), SendError>
    where
        M: Send + 'static,
    {
        (self.send)(&self.id, Box::new(msg))
    }

    /// Request graceful shutdown of this execution.
    pub fn shutdown(&self) {
        (self.shutdown)(&self.id)
    }

    /// Return the current state of this execution, if it exists.
    pub fn state(&self) -> Option<ExecutionState> {
        (self.state)(&self.id)
    }
}

unsafe impl Send for RuntimeHandle {}
unsafe impl Sync for RuntimeHandle {}
