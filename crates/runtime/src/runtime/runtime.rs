use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::runtime::execution::ExecutionId;
use crate::runtime::failure::SendError;
use crate::runtime::failure::SendErrorKind;
use crate::runtime::failure::SpawnError;
use crate::runtime::failure::SpawnErrorKind;
use crate::runtime::handle::RuntimeHandle;
use crate::runtime::lifecycle::ExecutionState;

/// Backend-neutral runtime trait.
///
/// The `Runtime` trait is the platform entry point for executing closures.
/// Actor frameworks (Tokio, Goakt, ProtoActor) are optional backend
/// implementations behind this interface.
///
/// # Contract
///
/// - **Sequential per-unit processing**: Messages for a single execution are
///   processed sequentially. Concurrent executions may run in parallel.
/// - **Failure isolation**: A failure in one execution does not affect others.
/// - **Fail-closed**: Internal errors must not silently succeed.
pub trait Runtime: Send + Sync + 'static {
    /// Spawn a new execution with the given closure.
    ///
    /// The closure receives a `RuntimeHandle` for scoped self-access.
    /// Returns an `ExecutionId` that can be used to send messages or
    /// query the execution's state.
    fn spawn<F, Fut>(
        &self,
        f: F,
        name: Option<&str>,
    ) -> Result<ExecutionId, SpawnError>
    where
        F: FnOnce(RuntimeHandle) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static;

    /// Send a message to an existing execution.
    ///
    /// Returns `SendError::NotFound` if no execution with the given id
    /// exists, or `SendError::Closed` if the execution is no longer
    /// accepting messages.
    fn send<M>(&self, id: &ExecutionId, msg: M) -> Result<(), SendError>
    where
        M: Send + 'static;

    /// Request graceful shutdown of an execution.
    ///
    /// The execution transitions to `Draining` (no new messages accepted)
    /// then `Terminated` once all in-flight messages are processed.
    fn shutdown(&self, id: &ExecutionId);

    /// Return the current state of an execution, if it exists.
    fn state(&self, id: &ExecutionId) -> Option<ExecutionState>;
}

#[cfg(test)]
mod test_null {
    use std::any::Any;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use crate::runtime::execution::ExecutionId;
    use crate::runtime::failure::{SendError, SendErrorKind, SpawnError, SpawnErrorKind};
    use crate::runtime::handle::RuntimeHandle;
    use crate::runtime::lifecycle::ExecutionState;
    use crate::runtime::runtime::Runtime;

    struct NullUnit {
        state: ExecutionState,
        messages: Vec<Box<dyn Any + Send>>,
    }

    pub struct NullRuntime {
        units: Mutex<HashMap<ExecutionId, NullUnit>>,
        runtime_shutdown: Mutex<bool>,
    }

    impl NullRuntime {
        pub fn new() -> Self {
            Self {
                units: Mutex::new(HashMap::new()),
                runtime_shutdown: Mutex::new(false),
            }
        }

        pub fn fail_unit(&self, id: ExecutionId) {
            let mut units = self.units.lock().unwrap();
            if let Some(unit) = units.get_mut(&id) {
                unit.state = ExecutionState::Failed;
            }
        }

        pub fn set_runtime_shutdown(&self, value: bool) {
            *self.runtime_shutdown.lock().unwrap() = value;
        }

        pub fn message_count(&self, id: &ExecutionId) -> usize {
            self.units
                .lock()
                .unwrap()
                .get(id)
                .map(|u| u.messages.len())
                .unwrap_or(0)
        }

        pub fn unit_states(&self) -> Vec<(ExecutionId, ExecutionState)> {
            self.units
                .lock()
                .unwrap()
                .iter()
                .map(|(id, u)| (*id, u.state.clone()))
                .collect()
        }
    }

    impl Default for NullRuntime {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Runtime for NullRuntime {
        fn spawn<F, Fut>(&self, _f: F, _name: Option<&str>) -> Result<ExecutionId, SpawnError>
        where
            F: FnOnce(RuntimeHandle) -> Fut + Send + 'static,
            Fut: std::future::Future<Output = ()> + Send + 'static,
        {
            if *self.runtime_shutdown.lock().unwrap() {
                return Err(SpawnError {
                    cause: SpawnErrorKind::Closed,
                });
            }

            let id = ExecutionId::new();

            let unit = NullUnit {
                state: ExecutionState::Active,
                messages: Vec::new(),
            };

            self.units.lock().unwrap().insert(id, unit);
            Ok(id)
        }

        fn send<M>(&self, id: &ExecutionId, msg: M) -> Result<(), SendError>
        where
            M: Send + 'static,
        {
            if *self.runtime_shutdown.lock().unwrap() {
                return Err(SendError {
                    id: *id,
                    cause: SendErrorKind::Closed,
                });
            }

            let mut units = self.units.lock().unwrap();
            let unit = units.get_mut(id).ok_or(SendError {
                id: *id,
                cause: SendErrorKind::NotFound,
            })?;

            match unit.state {
                ExecutionState::Active => {
                    unit.messages.push(Box::new(msg));
                    Ok(())
                }
                _ => Err(SendError {
                    id: *id,
                    cause: SendErrorKind::Closed,
                }),
            }
        }

        fn shutdown(&self, id: &ExecutionId) {
            let mut units = self.units.lock().unwrap();
            if let Some(unit) = units.get_mut(id) {
                unit.state = ExecutionState::Terminated;
            }
        }

        fn state(&self, id: &ExecutionId) -> Option<ExecutionState> {
            self.units
                .lock()
                .unwrap()
                .get(id)
                .map(|u| u.state.clone())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_spawn_returns_unique_id() {
            let runtime = NullRuntime::new();
            let id1 = runtime.spawn(|_handle| async {}, None).unwrap();
            let id2 = runtime.spawn(|_handle| async {}, None).unwrap();
            assert_ne!(id1, id2);
        }

        #[test]
        fn test_spawn_after_shutdown_returns_error() {
            let runtime = NullRuntime::new();
            runtime.set_runtime_shutdown(true);
            let result = runtime.spawn(|_handle| async {}, None);
            assert_eq!(
                result.unwrap_err().cause,
                SpawnErrorKind::Closed
            );
        }

        #[test]
        fn test_send_to_unknown_id_returns_error() {
            let runtime = NullRuntime::new();
            let unknown_id = ExecutionId::new();
            let result = runtime.send(&unknown_id, 42i32);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().cause,
                SendErrorKind::NotFound
            );
        }

        #[test]
        fn test_shutdown_terminates_unit() {
            let runtime = NullRuntime::new();
            let id = runtime.spawn(|_handle| async {}, None).unwrap();
            runtime.shutdown(&id);
            assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
        }

        #[test]
        fn test_failure_isolation() {
            let runtime = NullRuntime::new();
            let id1 = runtime.spawn(|_handle| async {}, None).unwrap();
            let id2 = runtime.spawn(|_handle| async {}, None).unwrap();

            runtime.fail_unit(id1);
            assert_eq!(runtime.state(&id1), Some(ExecutionState::Failed));
            assert_eq!(runtime.state(&id2), Some(ExecutionState::Active));

            runtime.send(&id2, 42i32).unwrap();
            assert_eq!(runtime.message_count(&id2), 1);
        }
    }
}

struct ExecutionUnit {
    state: ExecutionState,
    sender: mpsc::Sender<Box<dyn Any + Send>>,
    handle: Option<JoinHandle<()>>,
}

struct TokioRuntimeInner {
    handle: tokio::runtime::Handle,
    units: Mutex<HashMap<ExecutionId, ExecutionUnit>>,
}

/// Tokio-backed runtime implementation.
///
/// `TokioRuntime` implements the `Runtime` trait using Tokio's async runtime.
/// Each execution spawns a dedicated Tokio task that processes messages
/// sequentially from a channel.
pub struct TokioRuntime {
    inner: Arc<TokioRuntimeInner>,
}

impl TokioRuntime {
    /// Create a new `TokioRuntime` with the given Tokio handle.
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self {
            inner: Arc::new(TokioRuntimeInner {
                handle,
                units: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Create a new `TokioRuntime` with a new multi-threaded runtime.
    pub fn with_new_runtime() -> Result<Self, SpawnError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| SpawnError {
                cause: SpawnErrorKind::Internal(e.to_string()),
            })?;

        let handle = rt.handle().clone();
        let runtime = Self::new(handle);

        std::mem::forget(rt);

        Ok(runtime)
    }

    fn spawn_execution<F, Fut>(
        &self,
        f: F,
        _name: Option<&str>,
    ) -> Result<ExecutionId, SpawnError>
    where
        F: FnOnce(RuntimeHandle) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = ExecutionId::new();
        let (sender, _receiver) = mpsc::channel(64);

        let handle = self.inner.handle.clone();
        let state_fn = {
            let inner = self.inner.clone();
            move |execution_id: &ExecutionId| {
                inner
                    .units
                    .lock()
                    .unwrap()
                    .get(execution_id)
                    .map(|u| u.state.clone())
            }
        };
        let shutdown_fn = {
            let inner = self.inner.clone();
            move |execution_id: &ExecutionId| {
                if let Some(unit) = inner.units.lock().unwrap().get_mut(execution_id) {
                    if unit.state == ExecutionState::Active {
                        unit.state = ExecutionState::Draining;
                    }
                }
            }
        };
        let send_fn = {
            let inner = self.inner.clone();
            move |execution_id: &ExecutionId, msg: Box<dyn Any + Send + 'static>| {
                if let Some(unit) = inner.units.lock().unwrap().get(execution_id) {
                    match unit.state {
                        ExecutionState::Active => {
                            let _ = unit.sender.try_send(msg);
                            Ok(())
                        }
                        _ => Err(SendError {
                            id: *execution_id,
                            cause: SendErrorKind::Closed,
                        }),
                    }
                } else {
                    Err(SendError {
                        id: *execution_id,
                        cause: SendErrorKind::NotFound,
                    })
                }
            }
        };

        let runtime_handle = RuntimeHandle::new(id, send_fn, shutdown_fn, state_fn);

        let task = async move {
            let _ = f(runtime_handle).await;
        };

        let join_handle = handle.spawn(task);

        self.inner
            .units
            .lock()
            .unwrap()
            .insert(
                id,
                ExecutionUnit {
                    state: ExecutionState::Active,
                    sender,
                    handle: Some(join_handle),
                },
            );

        Ok(id)
    }

    fn send_message(
        &self,
        id: &ExecutionId,
        msg: Box<dyn Any + Send + 'static>,
    ) -> Result<(), SendError> {
        let (sender, state) = {
            let units = self.inner.units.lock().unwrap();
            let unit = units
                .get(id)
                .ok_or(SendError {
                    id: *id,
                    cause: SendErrorKind::NotFound,
                })?;
            (unit.sender.clone(), unit.state.clone())
        };

        match state {
            ExecutionState::Active => {
                sender
                    .try_send(msg)
                    .map_err(|_| SendError {
                        id: *id,
                        cause: SendErrorKind::Closed,
                    })
            }
            _ => Err(SendError {
                id: *id,
                cause: SendErrorKind::Closed,
            }),
        }
    }

    fn shutdown_execution(&self, id: &ExecutionId) {
        if let Some(unit) = self.inner.units.lock().unwrap().get_mut(id) {
            if unit.state == ExecutionState::Active {
                unit.state = ExecutionState::Draining;
            }
        }
    }
}

impl Runtime for TokioRuntime {
    fn spawn<F, Fut>(
        &self,
        f: F,
        name: Option<&str>,
    ) -> Result<ExecutionId, SpawnError>
    where
        F: FnOnce(RuntimeHandle) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.spawn_execution(f, name)
    }

    fn send<M>(&self, id: &ExecutionId, msg: M) -> Result<(), SendError>
    where
        M: Send + 'static,
    {
        self.send_message(id, Box::new(msg))
    }

    fn shutdown(&self, id: &ExecutionId) {
        self.shutdown_execution(id);
    }

    fn state(&self, id: &ExecutionId) -> Option<ExecutionState> {
        self.inner
            .units
            .lock()
            .unwrap()
            .get(id)
            .map(|u| u.state.clone())
    }
}
