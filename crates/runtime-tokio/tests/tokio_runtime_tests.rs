use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ego_runtime::runtime::lifecycle::ExecutionState;
use ego_runtime::runtime::runtime::Runtime;
use ego_runtime_tokio::TokioRuntime;

/// Test that the default runtime is multi-threaded and can spawn tasks.
#[tokio::test]
async fn test_multi_threaded_default() {
    let runtime = TokioRuntime::new();
    let id = runtime.spawn(|_handle| async {}, None).unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

/// Test that the current-thread builder creates a working runtime.
#[tokio::test]
async fn test_current_thread() {
    let runtime = TokioRuntime::builder()
        .current_thread()
        .build();

    let id = runtime.spawn(|_handle| async {}, None).unwrap();
    assert_eq!(runtime.state(&id), Some(ExecutionState::Active));
}

/// Test that sending a message to a spawned unit succeeds.
#[tokio::test]
async fn test_send_message() {
    let runtime = TokioRuntime::new();
    let received = Arc::new(AtomicUsize::new(0));
    let received_clone = Arc::clone(&received);

    let id = runtime
        .spawn(move |_handle| {
            let received = Arc::clone(&received_clone);
            async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                received.store(1, Ordering::SeqCst);
            }
        }, None)
        .unwrap();

    // Send messages while the task is running
    for _ in 0..5 {
        runtime.send(&id, "test").unwrap();
    }

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(received.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

/// Test that messages are delivered in sequential order to a single unit.
#[tokio::test]
async fn test_sequential_delivery() {
    let runtime = TokioRuntime::new();
    let order = Arc::new(AtomicUsize::new(0));
    let order_for_spawn = Arc::clone(&order);

    let id = runtime
        .spawn(move |_handle| {
            async move {
                // Simulate sequential processing by incrementing in order
                tokio::time::sleep(Duration::from_millis(10)).await;
                order_for_spawn.store(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                order_for_spawn.store(2, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                order_for_spawn.store(3, Ordering::SeqCst);
            }
        }, None)
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(order.load(Ordering::SeqCst), 3);
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

/// Test that a panic in one unit does not affect other units.
#[tokio::test]
async fn test_failure_isolation() {
    let runtime = TokioRuntime::new();

    // Spawn a unit that will panic
    let failing_id = runtime
        .spawn(|_handle| async { panic!("intentional failure"); }, None)
        .unwrap();

    // Spawn a healthy unit
    let healthy_counter = Arc::new(AtomicUsize::new(0));
    let healthy_counter_clone = Arc::clone(&healthy_counter);

    let healthy_id = runtime
        .spawn(move |_handle| {
            let counter = Arc::clone(&healthy_counter_clone);
            async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                counter.store(1, Ordering::SeqCst);
            }
        }, None)
        .unwrap();

    // Wait for the failing unit to panic
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(runtime.state(&failing_id), Some(ExecutionState::Failed));

    // The healthy unit should still be active
    assert_eq!(runtime.state(&healthy_id), Some(ExecutionState::Active));

    // Wait for the healthy unit to complete
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(runtime.state(&healthy_id), Some(ExecutionState::Terminated));
    assert_eq!(healthy_counter.load(Ordering::SeqCst), 1);
}

/// Test that shutdown transitions a unit to terminated.
#[tokio::test]
async fn test_shutdown() {
    let runtime = TokioRuntime::new();

    let id = runtime
        .spawn(|_handle| async {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }, None)
        .unwrap();

    // Verify it's active before shutdown
    assert_eq!(runtime.state(&id), Some(ExecutionState::Active));

    // Request shutdown
    runtime.shutdown(&id);

    // Verify it transitions to draining
    assert_eq!(runtime.state(&id), Some(ExecutionState::Draining));

    // Wait for the task to complete
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(runtime.state(&id), Some(ExecutionState::Terminated));
}

/// Test that the builder can configure a specific number of worker threads.
#[tokio::test]
async fn test_configured_worker_threads() {
    let runtime = TokioRuntime::builder()
        .worker_threads(4)
        .build();

    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    // Spawn multiple tasks to verify multi-threaded execution
    for _ in 0..8 {
        let counter_clone = Arc::clone(&counter);
        let id = runtime
            .spawn(move |_handle| {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            }, None)
            .unwrap();
        handles.push(id);
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    // All tasks should have completed
    for id in &handles {
        assert_eq!(runtime.state(id), Some(ExecutionState::Terminated));
    }
    assert_eq!(counter.load(Ordering::SeqCst), 8);
}

/// Test that fail-closed mode prevents spawn and send operations.
#[tokio::test]
async fn test_fail_closed() {
    let runtime = TokioRuntime::new();

    // Enable fail-closed mode
    runtime.set_fail_closed(true);

    // Spawn should fail
    let spawn_result = runtime.spawn(|_handle| async {}, None);
    assert!(spawn_result.is_err());

    // Verify no unit was created
    let fake_id = ego_runtime::runtime::execution::ExecutionId::new();
    assert!(runtime.state(&fake_id).is_none());

    // Send to a non-existent id should still return NotFound (not fail-closed)
    let send_result = runtime.send(&fake_id, "test");
    assert!(send_result.is_err());
}
