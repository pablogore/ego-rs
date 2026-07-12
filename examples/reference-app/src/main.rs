//! Real network entry point for the reference app (design.md AD-7): the axum
//! server sits OUTSIDE `RuntimeBuilder`, strictly bracketing its live window
//! — build the runtime, serve until a shutdown signal, then drain teardown.
//!
//! Complete graceful shutdown sequence (Task 3 extension of AD-7, now that
//! the `UsersByTenant` read-side scheduler also runs alongside the HTTP
//! server):
//!
//! 1. `ego_transport::serve(...)`'s graceful shutdown: stop accepting new
//!    TCP connections, then drain in-flight HTTP requests to completion.
//!    Every `RegisterUser` write that was in flight has, by construction,
//!    already synchronously inserted its events into the read-side store
//!    (`RegisterUserImpl::register` calls the sink before returning), so
//!    the read-side drain below is guaranteed to see them.
//! 2. `rt.shutdown_async()`: the read-side scheduler's stop-and-drain is
//!    registered as an async teardown hook (finding 6 fix — see
//!    `Runtime::register_async_teardown`/`Runtime::shutdown_async` in
//!    `ego-service-sdk`), so this single call runs it first, then drains the
//!    `Runtime`'s own sync teardown stack (logger/security) — the ordering
//!    that used to be hand-sequenced here is now enforced by the framework.
//!
//! Ordering still matters (now enforced, not hand-sequenced): the read-side
//! hook must finish before the sync stack drains, or the logger could
//! flush/close while the scheduler is still mid-batch.

use std::sync::Arc;

use ego_transport::AppState;
use reference_app::ports::http::build_router;
use reference_app::{build_runtime, AppConfig};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::default();
    let (rt, authn, read_side_handles) = build_runtime(&config)?;
    let rt = Arc::new(rt);

    let query = read_side_handles.query.clone();
    let read_side_runtime = read_side_handles.spawn();
    rt.register_async_teardown(read_side_runtime.stop());

    let state = AppState::new(rt.clone(), authn);
    let router = build_router(state, query);

    let listener = TcpListener::bind("127.0.0.1:3000").await?;
    println!("reference-app: listening on {}", listener.local_addr()?);

    // Step 1: stop accepting new connections, drain in-flight HTTP requests.
    ego_transport::serve(listener, router, shutdown_signal()).await?;
    println!("reference-app: HTTP drained, tearing down runtime (read-side scheduler, then logger/security)");

    // Step 2: read-side scheduler stop-and-drain, then sync teardown stack.
    rt.shutdown_async().await?;
    println!("reference-app: shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
