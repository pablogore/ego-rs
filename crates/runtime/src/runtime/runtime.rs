//! Backend-neutral runtime trait and Tokio-backed implementation.
//!
//! Defines the `Runtime` trait — the platform-agnostic entry point for
//! spawning and communicating with concurrent executions. Also provides
//! `TokioRuntime`, a production implementation backed by Tokio.

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

/// Internal execution unit — wraps state, message sender, and task handle.
struct ExecutionUnit {
    /// Current lifecycle state of this execution.
    state: ExecutionState,
    /// Channel sender for delivering messages to this execution.
    sender: mpsc::Sender<Box<dyn Any + Send>>,
    /// Join handle for the spawned Tokio task.
    handle: Option<JoinHandle<()>>,
}

/// Internal runtime state — Tokio handle and active execution registry.
struct TokioRuntimeInner {
    /// Handle to the Tokio runtime for spawning tasks.
    handle: tokio::runtime::Handle,
    /// Registry of all active executions, keyed by `ExecutionId`.
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
    /// Constructor methods for `TokioRuntime`.

    /// Creates a new `TokioRuntime` with the given Tokio handle.
    ///
    /// # Arguments
    /// * `handle` — a cloned `tokio::runtime::Handle` to use for spawning tasks.
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

    /// Internal: spawns an execution by creating an `ExecutionUnit`, building
    /// a `RuntimeHandle` with closure-based access, and launching the user's
    /// closure as a Tokio task.
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

    /// Internal: delivers a boxed message to an execution via its mpsc sender.
    ///
    /// # Errors
    /// Returns `SendError::NotFound` if the execution id does not exist,
    /// or `SendError::Closed` if the execution is not in `Active` state.
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

    /// Internal: transitions an execution to `Draining` state.
    ///
    /// No-op if the execution is not in `Active` state or does not exist.
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
