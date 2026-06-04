//! # ego-runtime-tokio
//!
//! Tokio-backed implementation of the `Runtime` trait.
//!
//! Provides `TokioRuntime` — a production runtime adapter that spawns
//! executions as Tokio tasks with per-execution mpsc channels for message
//! delivery. Also provides `TokioRuntimeBuilder` for configuring thread
//! count and runtime mode (single/multi-threaded).

use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use ego_runtime::runtime::execution::ExecutionId;
use ego_runtime::runtime::failure::{SendError, SendErrorKind, SpawnError, SpawnErrorKind};
use ego_runtime::runtime::handle::RuntimeHandle;
use ego_runtime::runtime::lifecycle::ExecutionState;
use ego_runtime::runtime::runtime::Runtime;

/// Internal state of a single execution unit.
///
/// Tracks lifecycle state and the mpsc sender for message delivery.
#[derive(Clone, Debug)]
enum UnitState {
    /// Execution is active with an open mpsc sender.
    Active(mpsc::Sender<Box<dyn Any + Send + 'static>>),
    /// Execution is draining — no new messages, in-flight may complete.
    Draining,
    /// Execution has terminated cleanly.
    Terminated,
    /// Execution has panicked or failed.
    Failed,
}

/// Internal context for a single execution within the Tokio runtime.
///
/// Bundles the unit's state, join handle, and optional message consumer task.
#[derive(Debug)]
struct UnitContext {
    /// Lifecycle state of this execution.
    state: UnitState,
    /// Join handle for the spawned Tokio task that runs the user closure.
    #[allow(dead_code)]
    handle: Option<JoinHandle<()>>,
    /// Join handle for the message consumer that drains the mpsc channel.
    #[allow(dead_code)]
    message_consumer: Option<JoinHandle<()>>,
}

/// Internal runtime state shared across all `TokioRuntime` clones.
#[derive(Debug)]
struct TokioRuntimeInner {
    /// Registry of all active executions keyed by `ExecutionId`.
    units: std::sync::Mutex<HashMap<ExecutionId, UnitContext>>,
    /// When `true`, spawn and send operations are rejected (fail-closed mode).
    fail_closed: AtomicBool,
}

/// Tokio-backed runtime implementation.
///
/// `TokioRuntime` implements the `Runtime` trait by spawning each execution
/// as a Tokio task. Messages are delivered via per-execution bounded mpsc
/// channels. Supports fail-closed mode where all spawn/send operations are
/// rejected.
///
/// # Thread Safety
///
/// Internally uses `Arc<TokioRuntimeInner>` with `Mutex`-protected state.
/// Clones share the same underlying state — shutdown or fail-closed on one
/// clone affects all clones.
#[derive(Clone)]
pub struct TokioRuntime {
    /// Shared inner state — execution registry and fail-closed flag.
    inner: Arc<TokioRuntimeInner>,
    /// Handle to the Tokio runtime for spawning tasks.
    tokio: tokio::runtime::Handle,
}

impl TokioRuntime {
    /// Constructor methods for `TokioRuntime`.
    /// Creates a new `TokioRuntime` with a newly allocated multi-threaded
    /// Tokio runtime.
    ///
    /// The Tokio runtime is leaked via `std::mem::forget` to give the
    /// `RuntimeHandle` a static lifetime. This is safe because the runtime
    /// is never dropped before process exit.
    ///
    /// # Panics
    /// Panics if Tokio runtime creation fails (e.g., resource exhaustion).
    pub fn new() -> Self {
        let tokio = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        let handle = tokio.handle().clone();
        let inner = Arc::new(TokioRuntimeInner {
            units: std::sync::Mutex::new(HashMap::new()),
            fail_closed: AtomicBool::new(false),
        });
        std::mem::forget(tokio);
        Self {
            inner,
            tokio: handle,
        }
    }

    /// Returns a `TokioRuntimeBuilder` for configuring runtime options.
    pub fn builder() -> TokioRuntimeBuilder {
        TokioRuntimeBuilder::new()
    }

    /// Sets the fail-closed mode.
    ///
    /// When `true`, all subsequent `spawn` and `send` operations will be
    /// rejected with a `Closed` error. Existing executions are unaffected.
    pub fn set_fail_closed(&self, value: bool) {
        self.inner.fail_closed.store(value, Ordering::Release);
    }

    /// Constructs a `SpawnError` with `Closed` cause.
    fn spawn_error_closed() -> SpawnError {
        SpawnError {
            cause: SpawnErrorKind::Closed,
        }
    }

    /// Constructs a `SpawnError` with `Internal` cause.
    #[allow(dead_code)]
    fn spawn_error_internal() -> SpawnError {
        SpawnError {
            cause: SpawnErrorKind::Internal("internal error".to_string()),
        }
    }

    /// Constructs a `SendError` with `NotFound` cause for a given id.
    fn send_error_not_found(id: ExecutionId) -> SendError {
        SendError {
            id,
            cause: SendErrorKind::NotFound,
        }
    }

    /// Constructs a `SendError` with `Closed` cause for a given id.
    fn send_error_closed(id: ExecutionId) -> SendError {
        SendError {
            id,
            cause: SendErrorKind::Closed,
        }
    }
}

impl Runtime for TokioRuntime {
    /// Spawns a new execution as a Tokio task.
    ///
    /// The user's closure receives a `RuntimeHandle` for self-access. A
    /// bounded mpsc channel (capacity 32) delivers messages to the execution.
    ///
    /// # Errors
    /// Returns `SpawnError::Closed` if fail-closed mode is active.
    fn spawn<F, Fut>(&self, f: F, _name: Option<&str>) -> Result<ExecutionId, SpawnError>
    where
        F: FnOnce(RuntimeHandle) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if self.inner.fail_closed.load(Ordering::Acquire) {
            return Err(Self::spawn_error_closed());
        }

        let id = ExecutionId::new();
        let inner = Arc::clone(&self.inner);

        let (tx, mut rx) = mpsc::channel::<Box<dyn Any + Send + 'static>>(32);

        let handle = RuntimeHandle::new(
            id,
            {
                let inner = Arc::clone(&inner);
                move |exec_id, msg| {
                    let mut units = inner.units.lock().unwrap();
                    let context = units
                        .get_mut(exec_id)
                        .ok_or_else(|| Self::send_error_not_found(*exec_id))?;
                    match &context.state {
                        UnitState::Active(t) => t
                            .try_send(Box::new(msg))
                            .map_err(|_| Self::send_error_closed(*exec_id)),
                        UnitState::Draining => Err(Self::send_error_closed(*exec_id)),
                        UnitState::Terminated => Err(Self::send_error_closed(*exec_id)),
                        UnitState::Failed => Err(Self::send_error_closed(*exec_id)),
                    }
                }
            },
            {
                let inner = Arc::clone(&inner);
                move |exec_id| {
                    if let Some(context) = inner.units.lock().unwrap().get_mut(exec_id) {
                        if matches!(context.state, UnitState::Active(_)) {
                            let _ = std::mem::replace(&mut context.state, UnitState::Draining);
                        }
                    }
                }
            },
            {
                let inner = Arc::clone(&inner);
                move |exec_id| {
                    inner
                        .units
                        .lock()
                        .unwrap()
                        .get(exec_id)
                        .map(|ctx| match &ctx.state {
                            UnitState::Active(_) => ExecutionState::Active,
                            UnitState::Draining => ExecutionState::Draining,
                            UnitState::Terminated => ExecutionState::Terminated,
                            UnitState::Failed => ExecutionState::Failed,
                        })
                }
            },
        );

        let tokio = self.tokio.clone();

        let message_consumer =
            tokio.spawn(async move { while let Some(_msg) = rx.recv().await {} });

        let wrapped = async move {
            let result = tokio.spawn(async move { f(handle).await }).await;

            let mut guard = inner.units.lock().unwrap();
            let unit = guard.get_mut(&id).unwrap();
            match result {
                Ok(()) => {
                    unit.state = UnitState::Terminated;
                }
                Err(e) if e.is_panic() => {
                    unit.state = UnitState::Failed;
                }
                Err(_e) => {
                    unit.state = UnitState::Failed;
                }
            }
        };

        let join_handle = self.tokio.spawn(wrapped);

        let context = UnitContext {
            state: UnitState::Active(tx),
            handle: Some(join_handle),
            #[allow(dead_code)]
            message_consumer: Some(message_consumer),
        };

        self.inner.units.lock().unwrap().insert(id, context);

        Ok(id)
    }

    /// Sends a message to an existing execution.
    ///
    /// # Errors
    /// Returns `SendError::Closed` if fail-closed mode is active, the
    /// execution is draining/terminated/failed, or the channel is full.
    /// Returns `SendError::NotFound` if no execution with the given id exists.
    fn send<M>(&self, id: &ExecutionId, msg: M) -> Result<(), SendError>
    where
        M: Send + 'static,
    {
        if self.inner.fail_closed.load(Ordering::Acquire) {
            return Err(Self::send_error_closed(*id));
        }

        let mut units = self.inner.units.lock().unwrap();
        let context = units
            .get_mut(id)
            .ok_or_else(|| Self::send_error_not_found(*id))?;

        match &context.state {
            UnitState::Active(tx) => {
                tx.try_send(Box::new(msg))
                    .map_err(|_| Self::send_error_closed(*id))?;
                Ok(())
            }
            UnitState::Draining => Err(Self::send_error_closed(*id)),
            UnitState::Terminated => Err(Self::send_error_closed(*id)),
            UnitState::Failed => Err(Self::send_error_closed(*id)),
        }
    }

    /// Requests graceful shutdown of an execution.
    ///
    /// Transitions the execution from `Active` to `Draining`. No-op if
    /// the execution does not exist or is already in a terminal state.
    fn shutdown(&self, id: &ExecutionId) {
        if let Some(context) = self.inner.units.lock().unwrap().get_mut(id) {
            if matches!(context.state, UnitState::Active(_)) {
                let old_state = std::mem::replace(&mut context.state, UnitState::Draining);
                drop(old_state);
            }
        }
    }

    /// Returns the current state of an execution, if it exists.
    fn state(&self, id: &ExecutionId) -> Option<ExecutionState> {
        self.inner
            .units
            .lock()
            .unwrap()
            .get(id)
            .map(|ctx| match &ctx.state {
                UnitState::Active(_) => ExecutionState::Active,
                UnitState::Draining => ExecutionState::Draining,
                UnitState::Terminated => ExecutionState::Terminated,
                UnitState::Failed => ExecutionState::Failed,
            })
    }
}

impl Default for TokioRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for configuring `TokioRuntime` options.
///
/// Allows setting the number of worker threads and choosing between
/// current-thread and multi-threaded scheduler modes.
pub struct TokioRuntimeBuilder {
    /// Number of worker threads (None = Tokio default).
    worker_threads: Option<usize>,
    /// If true, use Tokio's current-thread scheduler.
    current_thread: bool,
}

impl TokioRuntimeBuilder {
    /// Constructor methods for `TokioRuntimeBuilder`.
    /// Creates a new `TokioRuntimeBuilder` with default settings
    /// (multi-threaded, Tokio-default worker count).
    fn new() -> Self {
        Self {
            worker_threads: None,
            current_thread: false,
        }
    }

    /// Sets the number of worker threads for the Tokio runtime.
    ///
    /// Only applies when not using `current_thread()` mode.
    pub fn worker_threads(mut self, n: usize) -> Self {
        self.worker_threads = Some(n);
        self
    }

    /// Configures the Tokio runtime to use the current-thread scheduler.
    ///
    /// All spawned tasks run on the calling thread. Useful for tests or
    /// single-threaded contexts.
    pub fn current_thread(mut self) -> Self {
        self.current_thread = true;
        self
    }

    /// Builds the `TokioRuntime` with the configured options.
    ///
    /// # Panics
    /// Panics if Tokio runtime creation fails.
    pub fn build(self) -> TokioRuntime {
        let mut builder = if self.current_thread {
            tokio::runtime::Builder::new_current_thread()
        } else {
            tokio::runtime::Builder::new_multi_thread()
        };

        let builder = if let Some(threads) = self.worker_threads {
            builder.worker_threads(threads)
        } else {
            &mut builder
        };

        let tokio = builder
            .enable_all()
            .build()
            .expect("failed to create tokio runtime");
        let handle = tokio.handle().clone();
        std::mem::forget(tokio);

        TokioRuntime {
            inner: Arc::new(TokioRuntimeInner {
                units: std::sync::Mutex::new(HashMap::new()),
                fail_closed: AtomicBool::new(false),
            }),
            tokio: handle,
        }
    }
}

/// The default runtime type — an alias for `TokioRuntime`.
pub type DefaultRuntime = TokioRuntime;
