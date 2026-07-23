//! TASK-009 (RED): `serve()` binds a real ephemeral socket, serves a trivial
//! router, accepts one real client request, then stops within a bounded
//! timeout once its shutdown signal resolves.

use std::time::Duration;

use axum::routing::get;
use axum::Router;
use ego_transport::serve;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn http_get(addr: std::net::SocketAddr) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("connect to server");
    let request = format!("GET / HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .expect("read response");
    response
}

#[tokio::test]
async fn serve_handles_a_request_then_shuts_down_gracefully() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new().route("/", get(|| async { "ok" }));

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown = async move {
        let _ = shutdown_rx.await;
    };

    let server = tokio::spawn(serve(listener, router, shutdown));

    let response = http_get(addr).await;
    assert!(
        response.contains("200 OK"),
        "expected 200 OK, got: {response}"
    );
    assert!(
        response.contains("ok"),
        "expected body 'ok', got: {response}"
    );

    shutdown_tx.send(()).expect("shutdown receiver still alive");

    let outcome = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("serve() must return within the bounded timeout")
        .expect("serve task must not panic");
    assert!(
        outcome.is_ok(),
        "serve() must return Ok(()) after graceful shutdown"
    );
}
