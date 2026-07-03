//! HTTP smoke contract test — boots a real in-process hub and asserts the public HTTP surface the
//! website and CLI depend on. Where the WebSocket e2e tests (in parler-connector) exercise the agent
//! protocol, this nails down the *plain HTTP* contract: `/health`, `/api/hub`, `/api/directory`, and
//! the landing page. It is the in-process twin of `scripts/ci/smoke.sh` (which probes a live URL),
//! so "does a fresh build actually boot and serve?" is caught by `cargo test`, before any deploy.
//!
//! Dependency-free: a ~20-line raw HTTP/1.1 client over tokio's TcpStream — no reqwest/hyper-client.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Start an in-memory hub on an ephemeral port; return the bound address.
async fn start_hub() -> SocketAddr {
    let store = parler_hub::Store::open(None).expect("open in-memory store");
    let state = Arc::new(parler_hub::HubState::new(
        store,
        "parler://smoke".into(),
        "Smoke Hub".into(),
        parler_hub::HubMode::Private,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = parler_hub::serve(listener, state).await;
    });
    addr
}

/// Minimal HTTP/1.1 GET. Sends `Connection: close` so the server hangs up after the response and
/// `read_to_end` returns the whole thing. Returns `(status_code, body)`.
async fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.expect("write request");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.expect("read response");
    let raw = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (status, body.to_string())
}

/// Retry `/health` until the spawned server is accepting connections (or give up after ~3s).
async fn await_health(addr: SocketAddr) -> String {
    for _ in 0..30 {
        if let Ok(stream) = tokio::net::TcpStream::connect(addr).await {
            drop(stream);
            let (status, body) = get(addr, "/health").await;
            if status == 200 {
                return body;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("hub never became healthy at {addr}");
}

#[tokio::test]
async fn health_returns_ok() {
    let addr = start_hub().await;
    let body = await_health(addr).await;
    assert_eq!(body.trim(), "ok", "/health should return the literal string 'ok'");
}

#[tokio::test]
async fn api_hub_reports_identity_and_protocol() {
    let addr = start_hub().await;
    await_health(addr).await;
    let (status, body) = get(addr, "/api/hub").await;
    assert_eq!(status, 200, "/api/hub should be 200");
    // The website renders these; the protocol version lets clients negotiate compatibility.
    assert!(body.contains("\"name\""), "missing name in {body}");
    assert!(body.contains("\"protocolVersion\""), "missing protocolVersion in {body}");
    assert!(body.contains("\"mode\""), "missing mode in {body}");
    // Lightweight observability counters are exposed for monitoring.
    assert!(body.contains("\"stats\""), "missing stats in {body}");
    assert!(body.contains("\"messagesTotal\""), "missing messagesTotal in {body}");
}

#[tokio::test]
async fn api_directory_is_a_json_array() {
    let addr = start_hub().await;
    await_health(addr).await;
    let (status, body) = get(addr, "/api/directory").await;
    assert_eq!(status, 200, "/api/directory should be 200");
    // Public scope is world-readable and returns a JSON array (empty on a fresh private hub).
    assert!(body.trim_start().starts_with('['), "/api/directory should be a JSON array, got: {body}");
}

#[tokio::test]
async fn landing_page_renders() {
    let addr = start_hub().await;
    await_health(addr).await;
    let (status, _body) = get(addr, "/").await;
    assert_eq!(status, 200, "the landing page should be 200");
}

#[tokio::test]
async fn a2a_well_known_card_is_served() {
    let addr = start_hub().await;
    await_health(addr).await;
    // The A2A ecosystem's discovery entry point: an AgentCard at the standard well-known location.
    let (status, body) = get(addr, "/.well-known/agent-card.json").await;
    assert_eq!(status, 200, "/.well-known/agent-card.json should be 200");
    assert!(body.contains("\"protocolVersion\""), "missing protocolVersion in {body}");
    assert!(body.contains("\"skills\""), "missing skills in {body}");
    assert!(body.contains("\"capabilities\""), "missing capabilities in {body}");
    // Points a crawler at the per-hub agent directory.
    assert!(body.contains("/a2a/directory"), "should advertise the directory in {body}");
}

#[tokio::test]
async fn a2a_directory_is_a_json_array() {
    let addr = start_hub().await;
    await_health(addr).await;
    // Public scope is world-readable and returns a JSON array of A2A cards (empty on a fresh hub).
    let (status, body) = get(addr, "/a2a/directory").await;
    assert_eq!(status, 200, "/a2a/directory should be 200");
    assert!(body.trim_start().starts_with('['), "/a2a/directory should be a JSON array, got: {body}");
}
