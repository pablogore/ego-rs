//! Finding 3 (reliability fix): the read-side background poll loop used to
//! discard every poll failure (`let _ = scheduler.start_projection(...).await;`
//! in `read_side/mod.rs`), so a handler/store failure vanished silently —
//! no log, no metric, nothing. `ReadSideHandles::with_logger` now wires a
//! real logger so failures are visible (not panicked on — this is a
//! background task that should keep retrying).
//!
//! Uses `ego_testkit::CapturingLogger` — a real `KITLogger` whose output is
//! captured in memory, not a hand-rolled fake logging trait.

use std::time::Duration;

use chrono::Utc;
use ego_testkit::CapturingLogger;
use reference_app::read_side::{ReadSideHandles, ReadSideSink, SharedReadSideStore};

#[tokio::test]
async fn poll_loop_logs_errors_instead_of_silently_swallowing_them() {
    let store = SharedReadSideStore::new();
    let sink = ReadSideSink::new(store.clone());

    // A genuine poison-event failure through the real `UsersByTenantHandler`
    // (see `read_side::projection`'s `unrecognized event_type` poison-event
    // arm) — not a test-only backdoor, just an event type the handler
    // doesn't recognize.
    sink.record(
        "tenant-x",
        "agg-1",
        "NotARealEventType",
        serde_json::json!({}),
        Utc::now(),
    );

    let capturing = CapturingLogger::new();
    let handles = ReadSideHandles::new(store).with_logger(Some(capturing.logger()));
    let runtime = handles.spawn();

    // One poll tick (50ms interval) is enough to hit the failure at least once.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _ = runtime.stop().await;

    let records = capturing.records();
    assert!(
        records
            .iter()
            .any(|r| r.message.contains("read_side poll failed")
                && r.message.contains("poison event")),
        "expected the poll failure to be logged, got: {records:?}"
    );
}
