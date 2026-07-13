//! CORE-018 Phase 9 — full end-to-end acceptance: a real `axum::serve()`
//! socket, a real HTTP client, a real Hs256 JWT (proposal success criterion:
//! "A real HTTP request against a running axum server completes registration
//! end-to-end.").

mod support;

use std::sync::Arc;
use std::time::Duration;

use ego_transport::AppState;
use reference_app::ports::http::build_router;
use reference_app::{build_runtime, AppConfig, BuiltRuntime};
use support::make_token;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn http_post(
    addr: std::net::SocketAddr,
    authorization: Option<&str>,
    body: &str,
) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("connect to server");
    let mut request = format!(
        "POST /register HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(auth) = authorization {
        request.push_str(&format!("Authorization: {auth}\r\n"));
    }
    request.push_str("Connection: close\r\n\r\n");
    request.push_str(body);

    stream.write_all(request.as_bytes()).await.expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).await.expect("read response");
    response
}

#[tokio::test]
async fn real_http_request_without_jwt_returns_401_and_never_reaches_the_operation() {
    let config = AppConfig::default();
    let BuiltRuntime { runtime: rt, authn, read_side: read_side_handles } =
        build_runtime(&config).expect("build_runtime succeeds");
    let rt = Arc::new(rt);
    let state = AppState::new(rt.clone(), authn);
    let router = build_router(state, read_side_handles.query.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown = async move {
        let _ = shutdown_rx.await;
    };
    let server = tokio::spawn(ego_transport::serve(listener, router, shutdown));

    let body = serde_json::json!({
        "user_id": "user-1", "email": "user@example.com", "tenant_id": "tenant-a", "org_name": "Acme"
    })
    .to_string();
    let response = http_post(addr, None, &body).await;
    assert!(response.contains("401"), "expected 401, got: {response}");

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("serve() must return within the bounded timeout")
        .expect("serve task must not panic")
        .expect("serve() must return Ok(())");
}

#[tokio::test]
async fn real_http_request_with_valid_jwt_registers_both_entities_end_to_end() {
    let config = AppConfig::default();
    let BuiltRuntime { runtime: rt, authn, read_side: read_side_handles } =
        build_runtime(&config).expect("build_runtime succeeds");
    let rt = Arc::new(rt);
    let state = AppState::new(rt.clone(), authn);
    let router = build_router(state, read_side_handles.query.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown = async move {
        let _ = shutdown_rx.await;
    };
    let server = tokio::spawn(ego_transport::serve(listener, router, shutdown));

    let token = make_token("user-1", "tenant-a");
    let body = serde_json::json!({
        "user_id": "user-1", "email": "user@example.com", "tenant_id": "tenant-a", "org_name": "Acme"
    })
    .to_string();
    let response = http_post(addr, Some(&format!("Bearer {token}")), &body).await;
    assert!(response.contains("201"), "expected 201, got: {response}");
    assert!(response.contains("\"user_id\":\"user-1\""), "expected body to echo user_id: {response}");

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("serve() must return within the bounded timeout")
        .expect("serve task must not panic")
        .expect("serve() must return Ok(())");
}
