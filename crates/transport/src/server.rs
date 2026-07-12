//! Generic axum server bootstrap (AD-2, AD-7).
//!
//! The server is a host concern, not a `Runtime`-managed resource: no route
//! table lives here — that belongs to the concrete application (e.g.
//! `examples/reference-app`) that builds `router` and calls `serve`.
//!
//! Deviation from tasks.md's literal `Router<AppState>` signature: `axum`
//! only implements `Service<Request>` for `Router<()>` (state already
//! applied via `.with_state(..)`) — `Router<AppState>` does not compile as
//! a `serve` parameter. The caller is expected to call `.with_state(..)`
//! before passing `router` in, matching design.md's AD-7 data flow
//! (`main`'s `router.with_state(rt.clone())`).

use axum::Router;
use tokio::net::TcpListener;

/// Serves `router` on `listener` until `shutdown` resolves, then stops
/// accepting new connections and returns.
pub async fn serve(
    listener: TcpListener,
    router: Router,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router.into_make_service())
        .with_graceful_shutdown(shutdown)
        .await
}
