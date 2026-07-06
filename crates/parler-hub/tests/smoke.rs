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
    start_hub_with_http_limit(0).await
}

/// Like [`start_hub`], but override the per-IP HTTP request budget (`0` keeps the default). Used by the
/// rate-limit test to drive a low ceiling deterministically.
async fn start_hub_with_http_limit(max_http_per_min: u32) -> SocketAddr {
    start_hub_full(max_http_per_min).await.0
}

/// Boot an in-memory hub and return both its address and a handle to its store, so a test can assert
/// what a POST actually persisted (the store is cheaply cloneable and shares the same connection).
async fn start_hub_full(max_http_per_min: u32) -> (SocketAddr, parler_hub::Store) {
    let store = parler_hub::Store::open(None).expect("open in-memory store");
    let mut state = parler_hub::HubState::new(
        store.clone(),
        "parler://smoke".into(),
        "Smoke Hub".into(),
        parler_hub::HubMode::Private,
    );
    if max_http_per_min > 0 {
        state.max_http_per_min = max_http_per_min;
    }
    let state = Arc::new(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = parler_hub::serve(listener, state).await;
    });
    (addr, store)
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

/// Minimal HTTP/1.1 POST with a JSON body. Same dependency-free client shape as [`get`]. Returns
/// `(status_code, body)`.
async fn post_json(addr: SocketAddr, path: &str, body: &str) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
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
    // Cumulative estimated communication tokens the hub has relayed since boot.
    assert!(body.contains("\"estimatedTokensTotal\""), "missing estimatedTokensTotal in {body}");
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
async fn http_flood_is_rate_limited_and_health_is_exempt() {
    // A tiny per-IP budget so a handful of requests trips it. All requests here come from 127.0.0.1
    // (no proxy headers), so they share one bucket — exactly the flood shape we want to bound.
    let addr = start_hub_with_http_limit(3).await;
    await_health(addr).await;

    // The budget applies to the public API: after 3 requests in the window, the next is refused.
    let mut statuses = Vec::new();
    for _ in 0..5 {
        statuses.push(get(addr, "/api/hub").await.0);
    }
    assert!(statuses.contains(&429), "a flood past the budget must be throttled, got {statuses:?}");
    assert_eq!(statuses.last(), Some(&429), "requests stay throttled once over budget: {statuses:?}");

    // `/health` must never be throttled — Fly's liveness probe hits it every 15s and an over-budget
    // client must not be able to knock the hub's health check offline.
    let (status, body) = get(addr, "/health").await;
    assert_eq!(status, 200, "/health is exempt from the rate limit even when the IP is over budget");
    assert_eq!(body.trim(), "ok");
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

#[tokio::test]
async fn waitlist_accepts_a_valid_signup_and_persists_it() {
    let (addr, store) = start_hub_full(0).await;
    await_health(addr).await;
    let (status, body) = post_json(addr, "/api/waitlist", r#"{"email":"Alice@Example.com"}"#).await;
    assert_eq!(status, 200, "a valid signup should be 200, got body: {body}");
    assert!(body.contains("\"ok\":true"), "expected ok:true, got {body}");
    // The address is persisted, normalized (trim + lowercase).
    assert_eq!(store.waitlist_count().unwrap(), 1, "the signup was stored");
}

#[tokio::test]
async fn waitlist_duplicate_submit_stays_ok_and_single_row() {
    let (addr, store) = start_hub_full(0).await;
    await_health(addr).await;
    // Two submits of the same address (bit-identical here; normalization is covered by unit tests).
    for _ in 0..2 {
        let (status, body) = post_json(addr, "/api/waitlist", r#"{"email":"dup@example.com"}"#).await;
        assert_eq!(status, 200, "a duplicate must not leak membership — still 200: {body}");
        assert!(body.contains("\"ok\":true"), "expected ok:true, got {body}");
    }
    // INSERT OR IGNORE ⇒ still exactly one row.
    assert_eq!(store.waitlist_count().unwrap(), 1, "a duplicate must not add a second row");
}

#[tokio::test]
async fn waitlist_rejects_an_invalid_email() {
    let (addr, store) = start_hub_full(0).await;
    await_health(addr).await;
    let (status, body) = post_json(addr, "/api/waitlist", r#"{"email":"not-an-email"}"#).await;
    assert_eq!(status, 400, "an invalid address should be 400, got: {body}");
    assert!(body.contains("\"ok\":false"), "expected ok:false, got {body}");
    assert!(body.contains("invalid email"), "expected the invalid-email error, got {body}");
    assert_eq!(store.waitlist_count().unwrap(), 0, "an invalid address is never stored");
}

#[tokio::test]
async fn waitlist_flood_from_one_ip_is_rate_limited() {
    // All requests come from 127.0.0.1 (no proxy headers) so they share one waitlist bucket. The tight
    // per-IP signup budget trips well before the general front-door limit, so a distinct valid address
    // each time still gets throttled once over budget — proving the dedicated waitlist window fires.
    let (addr, _store) = start_hub_full(0).await;
    await_health(addr).await;
    let mut statuses = Vec::new();
    for i in 0..(parler_hub::WAITLIST_MAX_PER_MIN + 3) {
        let body = format!(r#"{{"email":"flood{i}@example.com"}}"#);
        statuses.push(post_json(addr, "/api/waitlist", &body).await.0);
    }
    assert!(statuses.contains(&429), "a signup flood must be throttled, got {statuses:?}");
    assert_eq!(statuses.last(), Some(&429), "requests stay throttled once over budget: {statuses:?}");
}
