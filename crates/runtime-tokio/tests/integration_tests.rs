use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ego_runtime::runtime::lifecycle::ExecutionState;
use ego_runtime::runtime::runtime::Runtime;
use ego_runtime_tokio::TokioRuntime;

#[tokio::test]
async fn test_spawn_task_completes_and_terminates() {
    let runtime = TokioRuntime::new();
    let id = runtime.spawn(|_handle| async { /* immediate completion */ }, None).unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

#[tokio::test]
async fn test_spawn_task_panics_and_fails() {
    let runtime = TokioRuntime::new();
    let id = runtime.spawn(|_handle| async { panic!("intentional panic"); }, None).unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(runtime.state(&id), Some(ExecutionState::Failed));
}

#[tokio::test]
async fn test_spawn_task_with_duration() {
    let runtime = TokioRuntime::new();
    let id = runtime
        .spawn(|_handle| async {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }, None)
        .unwrap();

    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(runtime.state(&id), Some(ExecutionState::Active));

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

#[tokio::test]
async fn test_multiple_spawns_concurrent() {
    let runtime = TokioRuntime::new();
    let counter = Arc::new(AtomicUsize::new(0));

    let mut ids = Vec::new();
    for _ in 0..5 {
        let counter_clone = Arc::clone(&counter);
        let id = runtime
            .spawn(move |_handle| {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            }, None)
            .unwrap();
        ids.push(id);
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    for id in &ids {
        assert_eq!(runtime.state(id), Some(ExecutionState::Terminated));
    }
    assert_eq!(counter.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn test_runtime_handle_self_shutdown() {
    let runtime = TokioRuntime::new();

    let id = runtime
        .spawn(|handle| async move {
            handle.shutdown();
        }, None)
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

#[tokio::test]
async fn test_runtime_handle_self_state() {
    let runtime = TokioRuntime::new();

    let id = runtime
        .spawn(|handle| async move {
            let state = handle.state();
            assert_eq!(state, Some(ExecutionState::Active));
        }, None)
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

#[tokio::test]
async fn test_external_shutdown_before_completion() {
    let runtime = TokioRuntime::new();

    let id = runtime
        .spawn(|_handle| async {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }, None)
        .unwrap();

    runtime.shutdown(&id);
    assert_eq!(runtime.state(&id), Some(ExecutionState::Draining));

    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

#[tokio::test]
async fn test_external_shutdown_after_completion() {
    let runtime = TokioRuntime::new();

    let id = runtime
        .spawn(|_handle| async {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }, None)
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));

    runtime.shutdown(&id);
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

#[tokio::test]
async fn test_send_to_active_task() {
    let runtime = TokioRuntime::new();
    let message_count = Arc::new(AtomicUsize::new(0));
    let count_clone = Arc::clone(&message_count);

    let id = runtime
        .spawn(move |_handle| {
            let count = Arc::clone(&count_clone);
            async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                count.store(10, Ordering::SeqCst);
            }
        }, None)
        .unwrap();

    for _ in 0..10 {
        let result = runtime.send(&id, "hello");
        assert!(result.is_ok());
    }

    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(message_count.load(Ordering::SeqCst), 10);
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

#[tokio::test]
async fn test_send_after_task_terminated() {
    let runtime = TokioRuntime::new();

    let id = runtime
        .spawn(|_handle| async {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }, None)
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));

    let result = runtime.send(&id, "too late");
    assert!(result.is_err());
}

#[tokio::test]
async fn test_builder_multi_thread_executes() {
    let runtime = TokioRuntime::builder()
        .worker_threads(4)
        .build();

    let id = runtime
        .spawn(|_handle| async {}, None)
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

#[tokio::test]
async fn test_builder_current_thread_executes() {
    let runtime = TokioRuntime::builder()
        .current_thread()
        .build();

    let id = runtime
        .spawn(|_handle| async {}, None)
        .unwrap();

    assert_eq!(runtime.state(&id), Some(ExecutionState::Active));
}

#[tokio::test]
async fn test_failure_isolation_across_units() {
    let runtime = TokioRuntime::new();

    let id1 = runtime
        .spawn(|_handle| async { panic!("fail first"); }, None)
        .unwrap();
    let id2 = runtime
        .spawn(|_handle| async {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }, None)
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(runtime.state(&id1), Some(ExecutionState::Failed));
    assert_eq!(runtime.state(&id2), Some(ExecutionState::Active));

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(runtime.state(&id2), Some(ExecutionState::Terminated));
}
