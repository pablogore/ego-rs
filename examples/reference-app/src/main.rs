//! Real network entry point for the reference app (design.md AD-7, CORE-028
//! Stage 1 AD-6): the axum server sits OUTSIDE `App`'s lifecycle, strictly
//! bracketing its live window — build the app, start it, serve until a
//! shutdown signal, then drain teardown. `App` owns no transport future; the
//! host (this file) sequences transport around `App::start()`/
//! `RunningApp::shutdown()` itself.
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
//! 2. `RunningApp::shutdown()`: the read-side scheduler's stop-and-drain is
//!    registered as a shutdown participant via `App::register_shutdown`
//!    (which wraps the existing `Runtime::register_async_teardown`), so
//!    this single call runs it first, then drains the runtime's own sync
//!    teardown stack (logger/security) — the ordering that used to be
//!    hand-sequenced here is now enforced by the framework.
//!
//! Ordering still matters (now enforced, not hand-sequenced): the read-side
//! hook must finish before the sync stack drains, or the logger could
//! flush/close while the scheduler is still mid-batch.
//!
//! Post-review Finding F-02: `ReadSideRuntime::stop()` returns
//! `Result<(), RuntimeInfraError>`, not `()` — if the scheduler task
//! panicked or was aborted, `RunningApp::shutdown()`'s `?` below propagates
//! that failure and "shutdown complete" is never printed, instead of
//! silently reporting success for a shutdown that didn't actually drain.

use ego_transport::AppState;
use reference_app::ports::http::build_router;
use reference_app::{build_runtime_with, AppConfig, BuiltRuntime, EntityEventStores};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::default();

    // The host owns the pool, the migrations, and the decision to start at all.
    //
    // Every one of these steps is fail-closed. A Postgres that cannot be
    // reached, migrations that will not apply, or a store that refuses to open
    // stops the process here — none of them degrades to an in-memory store.
    // That fallback is exactly what made a restart lose every confirmed receipt,
    // and it is now unreachable by omission: `build_runtime_with` takes the
    // stores it will use.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(config.database.max_connections)
        .connect(&config.database.url)
        .await?;
    ego_persistence::postgres::migrations::run(&pool).await?;
    let stores = EntityEventStores::open(pool).await?;

    let BuiltRuntime {
        app,
        authn,
        read_side: read_side_handles,
        ..
    } = build_runtime_with(&config, stores, None)?;

    let query = read_side_handles.query.clone();
    let read_side_runtime = read_side_handles.spawn();
    // Registered before `start()` (AD-6): `App` tracks the handle for
    // shutdown timing only — the read model above stays application-owned,
    // exactly as stage 0's spec requires.
    app.register_shutdown(read_side_runtime.stop());

    // `App::resolver()` (Stage 1 PR2, narrowed after review): `AppState`
    // predates `App`/`AppBuilder` and needs resolution access for its own
    // generic per-request `resolve::<Tag>()` dispatch — a legitimate
    // integration seam, but only for resolution, not the full `Runtime`
    // lifecycle surface `App`/`RunningApp` own.
    let state = AppState::new(app.resolver(), authn);
    let router = build_router(state, query);

    // `App::start()` (AD-2/AD-6): starts effects (none registered here —
    // zero-cost no-op), owns no transport future. The host still sequences
    // its own transport around it, same as before this migration.
    let running = app.start().await?;

    let listener = TcpListener::bind("127.0.0.1:3000").await?;
    println!("reference-app: listening on {}", listener.local_addr()?);

    // Step 1: stop accepting new connections, drain in-flight HTTP requests.
    ego_transport::serve(listener, router, shutdown_signal()).await?;
    println!("reference-app: HTTP drained, tearing down runtime (read-side scheduler, then logger/security)");

    // Step 2: `RunningApp::shutdown()` — read-side scheduler stop-and-drain,
    // then sync teardown stack, in that order (unchanged ordering, now
    // framework-executed instead of hand-sequenced via `Runtime` directly).
    running.shutdown().await?;
    println!("reference-app: shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
