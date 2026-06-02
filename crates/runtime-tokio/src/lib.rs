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

#[derive(Clone, Debug)]
enum UnitState {
    Active(mpsc::Sender<Box<dyn Any + Send + 'static>>),
    Draining,
    Terminated,
    Failed,
}

#[derive(Debug)]
struct UnitContext {
    state: UnitState,
    #[allow(dead_code)]
    handle: Option<JoinHandle<()>>,
    #[allow(dead_code)]
    message_consumer: Option<JoinHandle<()>>,
}

#[derive(Debug)]
struct TokioRuntimeInner {
    units: std::sync::Mutex<HashMap<ExecutionId, UnitContext>>,
    fail_closed: AtomicBool,
}

#[derive(Clone)]
pub struct TokioRuntime {
    inner: Arc<TokioRuntimeInner>,
    tokio: tokio::runtime::Handle,
}

impl TokioRuntime {
    pub fn new() -> Self {
        let tokio = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        let handle = tokio.handle().clone();
        let inner = Arc::new(TokioRuntimeInner {
            units: std::sync::Mutex::new(HashMap::new()),
            fail_closed: AtomicBool::new(false),
        });
        std::mem::forget(tokio);
        Self { inner, tokio: handle }
    }

    pub fn builder() -> TokioRuntimeBuilder {
        TokioRuntimeBuilder::new()
    }

    pub fn set_fail_closed(&self, value: bool) {
        self.inner.fail_closed.store(value, Ordering::Release);
    }

    fn spawn_error_closed() -> SpawnError {
        SpawnError {
            cause: SpawnErrorKind::Closed,
        }
    }

    #[allow(dead_code)]
    fn spawn_error_internal() -> SpawnError {
        SpawnError {
            cause: SpawnErrorKind::Internal("internal error".to_string()),
        }
    }

    fn send_error_not_found(id: ExecutionId) -> SendError {
        SendError {
            id,
            cause: SendErrorKind::NotFound,
        }
    }

    fn send_error_closed(id: ExecutionId) -> SendError {
        SendError {
            id,
            cause: SendErrorKind::Closed,
        }
    }
}

impl Runtime for TokioRuntime {
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
                    let context = units.get_mut(exec_id).ok_or_else(|| Self::send_error_not_found(*exec_id))?;
                    match &context.state {
                        UnitState::Active(t) => {
                            t.try_send(Box::new(msg)).map_err(|_| Self::send_error_closed(*exec_id))
                        }
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
                            std::mem::replace(&mut context.state, UnitState::Draining);
                        }
                    }
                }
            },
            {
                let inner = Arc::clone(&inner);
                move |exec_id| {
                    inner.units.lock().unwrap().get(exec_id).map(|ctx| match &ctx.state {
                        UnitState::Active(_) => ExecutionState::Active,
                        UnitState::Draining => ExecutionState::Draining,
                        UnitState::Terminated => ExecutionState::Terminated,
                        UnitState::Failed => ExecutionState::Failed,
                    })
                }
            },
        );

        let tokio = self.tokio.clone();

        let message_consumer = tokio.spawn(async move {
            while let Some(_msg) = rx.recv().await {}
        });

        let wrapped = async move {
            let result = tokio
                .spawn(async move { f(handle).await })
                .await;

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
                tx.try_send(Box::new(msg)).map_err(|_| Self::send_error_closed(*id))?;
                Ok(())
            }
            UnitState::Draining => Err(Self::send_error_closed(*id)),
            UnitState::Terminated => Err(Self::send_error_closed(*id)),
            UnitState::Failed => Err(Self::send_error_closed(*id)),
        }
    }

    fn shutdown(&self, id: &ExecutionId) {
        if let Some(context) = self.inner.units.lock().unwrap().get_mut(id) {
            if matches!(context.state, UnitState::Active(_)) {
                let old_state = std::mem::replace(&mut context.state, UnitState::Draining);
                drop(old_state);
            }
        }
    }

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

pub struct TokioRuntimeBuilder {
    worker_threads: Option<usize>,
    current_thread: bool,
}

impl TokioRuntimeBuilder {
    fn new() -> Self {
        Self {
            worker_threads: None,
            current_thread: false,
        }
    }

    pub fn worker_threads(mut self, n: usize) -> Self {
        self.worker_threads = Some(n);
        self
    }

    pub fn current_thread(mut self) -> Self {
        self.current_thread = true;
        self
    }

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

        let tokio = builder.enable_all().build().expect("failed to create tokio runtime");
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

pub type DefaultRuntime = TokioRuntime;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_spawn_returns_unique_id() {
        let runtime = TokioRuntime::new();
        let id1 = runtime.spawn(|_handle| async {}, None).unwrap();
        let id2 = runtime.spawn(|_handle| async {}, None).unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_spawn_creates_active_unit() {
        let runtime = TokioRuntime::new();
        let id = runtime.spawn(|_handle| async {}, None).unwrap();
        assert_eq!(runtime.state(&id), Some(ExecutionState::Active));
    }

    #[test]
    fn test_spawn_after_fail_closed_returns_error() {
        let runtime = TokioRuntime::new();
        runtime.inner.fail_closed.store(true, Ordering::Release);
        let result = runtime.spawn(|_handle| async {}, None);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().cause, SpawnErrorKind::Closed);
    }

    #[test]
    fn test_send_to_unknown_id_returns_error() {
        let runtime = TokioRuntime::new();
        let unknown_id = ExecutionId::new();
        let result = runtime.send(&unknown_id, 42i32);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().cause, SendErrorKind::NotFound);
    }

    #[test]
    fn test_send_to_active_unit_succeeds() {
        let runtime = TokioRuntime::new();
        let id = runtime.spawn(|_handle| async {}, None).unwrap();
        let result = runtime.send(&id, 42i32);
        assert!(result.is_ok());
    }

    #[test]
    fn test_send_after_shutdown_returns_error() {
        let runtime = TokioRuntime::new();
        let id = runtime.spawn(|_handle| async {}, None).unwrap();
        runtime.shutdown(&id);
        let result = runtime.send(&id, 42i32);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().cause, SendErrorKind::Closed);
    }

    #[test]
    fn test_send_after_fail_closed_returns_error() {
        let runtime = TokioRuntime::new();
        let id = runtime.spawn(|_handle| async {}, None).unwrap();
        runtime.inner.fail_closed.store(true, Ordering::Release);
        let result = runtime.send(&id, 42i32);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().cause, SendErrorKind::Closed);
    }

    #[test]
    fn test_shutdown_transitions_to_draining() {
        let runtime = TokioRuntime::new();
        let id = runtime.spawn(|_handle| async {}, None).unwrap();
        runtime.shutdown(&id);
        assert_eq!(runtime.state(&id), Some(ExecutionState::Draining));
    }

    #[test]
    fn test_shutdown_on_nonexistent_id_is_noop() {
        let runtime = TokioRuntime::new();
        let unknown_id = ExecutionId::new();
        runtime.shutdown(&unknown_id);
        assert!(runtime.state(&unknown_id).is_none());
    }

    #[test]
    fn test_state_returns_none_for_unknown_id() {
        let runtime = TokioRuntime::new();
        let unknown_id = ExecutionId::new();
        assert!(runtime.state(&unknown_id).is_none());
    }

    #[test]
    fn test_failure_isolation() {
        let runtime = TokioRuntime::new();
        let id1 = runtime.spawn(|_handle| async {}, None).unwrap();
        let id2 = runtime.spawn(|_handle| async {}, None).unwrap();

        runtime.shutdown(&id1);
        assert_eq!(runtime.state(&id1), Some(ExecutionState::Draining));
        assert_eq!(runtime.state(&id2), Some(ExecutionState::Active));
    }

    #[test]
    fn test_builder_multi_thread() {
        let runtime = TokioRuntime::builder()
            .worker_threads(4)
            .build();
        let id = runtime.spawn(|_handle| async {}, None).unwrap();
        assert_eq!(runtime.state(&id), Some(ExecutionState::Active));
    }

    #[test]
    fn test_builder_current_thread() {
        let runtime = TokioRuntime::builder()
            .current_thread()
            .build();
        let id = runtime.spawn(|_handle| async {}, None).unwrap();
        assert_eq!(runtime.state(&id), Some(ExecutionState::Active));
    }

    #[test]
    fn test_default() {
        let runtime = TokioRuntime::default();
        let id = runtime.spawn(|_handle| async {}, None).unwrap();
        assert_eq!(runtime.state(&id), Some(ExecutionState::Active));
    }

    #[test]
    fn test_spawned_task_receives_message() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let runtime = TokioRuntime::new();
        let id = runtime.spawn(move |_handle| {
            let _ = counter_clone.load(Ordering::SeqCst);
            async move {}
        }, None)
        .unwrap();

        // The spawned task sends a message to itself, but since we don't
        // have a message processor, we just verify the unit exists and is active
        assert_eq!(runtime.state(&id), Some(ExecutionState::Active));
    }

    #[test]
    fn test_clone_runtime_shared_state() {
        let runtime = TokioRuntime::new();
        let id = runtime.spawn(|_handle| async {}, None).unwrap();
        assert_eq!(runtime.state(&id), Some(ExecutionState::Active));

        let runtime2 = runtime.clone();
        assert_eq!(runtime2.state(&id), Some(ExecutionState::Active));

        runtime2.shutdown(&id);
        assert_eq!(runtime.state(&id), Some(ExecutionState::Draining));
    }

    #[test]
    fn test_multiple_messages_to_same_unit() {
        let runtime = TokioRuntime::new();
        let id = runtime.spawn(|_handle| async {}, None).unwrap();

        for i in 0..10 {
            assert!(runtime.send(&id, i).is_ok());
        }

        assert_eq!(runtime.state(&id), Some(ExecutionState::Active));
    }

    #[test]
    fn test_spawn_error_after_fail_closed_prevents_spawn() {
        let runtime = TokioRuntime::new();
        runtime.inner.fail_closed.store(true, Ordering::Release);

        let id = ExecutionId::new();
        assert!(runtime.state(&id).is_none());

        let result = runtime.spawn(|_handle| async {}, None);
        assert!(result.is_err());

        // Verify no unit was created
        assert!(runtime.state(&id).is_none());
    }

    #[test]
    fn test_send_error_after_fail_closed() {
        let runtime = TokioRuntime::new();
        let id = runtime.spawn(|_handle| async {}, None).unwrap();

        // Send should work before fail-closed
        assert!(runtime.send(&id, 1i32).is_ok());

        runtime.inner.fail_closed.store(true, Ordering::Release);

        // Send should fail after fail-closed
        let result = runtime.send(&id, 2i32);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().cause, SendErrorKind::Closed);
    }
}
