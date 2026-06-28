//! The hub's WebSocket front door + per-connection request handling.
//!
//! Each connection is a small state machine: an unauthenticated socket may only send `Hello`; the
//! hub replies with a [`ServerFrame::Challenge`] nonce, the client signs it with its nkey seed, and
//! once verified every other op is authorized against room membership. Every op gets exactly one
//! reply frame; delivery is durable-by-pull (a recipient *pulls* past its cursor), which keeps the
//! hub stateless per message and trivially durable.
//!
//! A connection may additionally [`ClientFrame::Subscribe`] for **live push**: thereafter the hub
//! sends unsolicited [`ServerFrame::Delivery`] frames (out of band from replies) the instant a peer's
//! message lands in one of the agent's rooms. Push is best-effort and in-memory — the durable cursor
//! stays the source of truth, so it only lowers latency and a dropped push is always recoverable by
//! the next [`ClientFrame::Pull`].

use crate::{now_ms, Store};
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::{Json, Router};
use parler_protocol::{
    canonical_card_bytes, normalize_mentions, token, ClientFrame, DiscoverScope, EndpointRef,
    RoomKind, ServerFrame, StoredMessage, Target,
};
use rand::Rng;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};

/// How many undelivered pushes a single subscribed connection may queue before the hub starts
/// dropping them (the connection's write side is slower than the room's send rate). A dropped push
/// is harmless — the message is durable and the subscriber catches up on its next [`ClientFrame::Pull`]
/// — so this only bounds per-connection memory; it never loses a message.
const PUSH_BUFFER: usize = 256;

/// Default cap on a single handed-off blob (git bundle): 25 MiB.
pub const DEFAULT_MAX_BLOB_BYTES: u64 = 25 * 1024 * 1024;

/// Default total disk budget for all stored blobs: 1 GiB.
pub const DEFAULT_MAX_BLOB_DIR_BYTES: u64 = 1024 * 1024 * 1024;

/// Default cap on the JSON-serialized `parts` of a single message: 1 MiB. Code goes through blobs,
/// so chat/text payloads never need to be large — this bounds per-message DB growth.
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Default ceiling on concurrent WebSocket connections to one hub.
pub const DEFAULT_MAX_CONNECTIONS: usize = 1024;

/// How long an unauthenticated socket may stay open before it must complete the handshake. Bounds
/// slow-loris connections that open a socket and never authenticate.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// Default idle timeout for an *authenticated* connection: 30 minutes. A connection that sends no
/// frame for this long is disconnected and its slot freed, so silent/abandoned agents don't linger
/// in the hub. The agent can reconnect at any time and resume from its durable cursor. `None`
/// disables the bound (see [`HubState::idle_timeout`]).
pub const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 1800;

/// Hard ceiling on any client-supplied TTL (invites, directory tokens): 365 days. Prevents an
/// attacker-supplied `ttl_secs` from overflowing the millisecond expiry math.
const MAX_TTL_SECS: u64 = 365 * 24 * 3600;

/// Per-agent flood limits (fixed-window). `0` disables a limit. State is in-memory and resets on
/// hub restart — a deliberately simple posture for a low-ops bus.
#[derive(Debug, Clone, Copy)]
pub struct RateLimits {
    pub max_sends_per_min: u32,
    pub max_blobs_per_hour: u32,
}

impl Default for RateLimits {
    fn default() -> Self {
        RateLimits { max_sends_per_min: 240, max_blobs_per_hour: 120 }
    }
}

/// Background-janitor retention policy — how the hub bounds its otherwise append-only growth. Every
/// trimming window defaults to *disabled*, so a deployed hub keeps every message/fact/blob until an
/// operator opts in; only the always-safe expired-invite/token sweep (and an incremental vacuum) runs
/// unconditionally. See [`Store::prune_messages`], [`Store::prune_facts`], [`Store::gc_blobs`].
#[derive(Debug, Clone, Copy)]
pub struct Retention {
    /// Delete messages older than this. `None` ⇒ keep all message history.
    pub message_max_age: Option<Duration>,
    /// Always keep at least this many newest messages per room (the floor for `message_max_age`).
    pub keep_messages_per_room: i64,
    /// Keep only this many newest *unkeyed* facts per (author, room). `None` ⇒ keep all.
    pub keep_unkeyed_facts: Option<i64>,
    /// Delete blob bytes neither fetched nor created within this window. `None` ⇒ keep until the disk
    /// budget fills.
    pub blob_max_idle: Option<Duration>,
    /// How often the janitor runs.
    pub interval: Duration,
}

impl Default for Retention {
    fn default() -> Self {
        Retention {
            message_max_age: None,
            keep_messages_per_room: 10_000,
            keep_unkeyed_facts: None,
            blob_max_idle: None,
            interval: Duration::from_secs(3600),
        }
    }
}

#[derive(Clone, Copy)]
enum RateKind {
    Send,
    Blob,
}

/// A single fixed-window counter.
#[derive(Default, Clone, Copy)]
struct Window {
    start: i64,
    count: u32,
}

#[derive(Default)]
struct AgentRate {
    sends: Window,
    blobs: Window,
}

/// Whether a hub's directory is world-readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubMode {
    /// The public directory (`scope=public`) and any agent's public card are readable without auth.
    Public,
    /// The whole directory is token-gated; only `public` agents leak without a directory token.
    Private,
}

impl HubMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            HubMode::Public => "public",
            HubMode::Private => "private",
        }
    }
}

/// Shared server state: the durable store, the hub's identity (name/mode), the base URL advertised in
/// invite links, the on-disk blob directory, and per-agent flood limits.
pub struct HubState {
    pub store: Store,
    pub public_url: String,
    pub name: String,
    pub mode: HubMode,
    /// Where handed-off blob bytes are written, one file per content id.
    pub blob_dir: PathBuf,
    /// Largest single blob the hub accepts.
    pub max_blob_bytes: u64,
    /// Total disk budget across all stored blobs; uploads that would exceed it are rejected.
    pub max_blob_dir_bytes: u64,
    /// Largest JSON-serialized `parts` payload accepted on a single `Send`.
    pub max_message_bytes: usize,
    /// Ceiling on concurrent connections; once reached, new sockets are refused.
    pub max_connections: usize,
    /// How long an authenticated connection may stay silent before the hub drops it. `None` keeps
    /// connections open indefinitely. Defaults to [`DEFAULT_IDLE_TIMEOUT_SECS`].
    pub idle_timeout: Option<Duration>,
    /// Optional shared join secret. When set, a connection must present a matching `secret` on its
    /// signed `Hello` to authenticate — the access gate for a closed/private hub. `None` ⇒ open.
    pub join_secret: Option<String>,
    /// Per-agent flood limits.
    pub limits: RateLimits,
    /// How the background janitor bounds append-only growth (defaults to keep-everything).
    pub retention: Retention,
    /// In-memory rate-limit counters, keyed by agent id (resets on restart).
    rate: Mutex<HashMap<String, AgentRate>>,
    /// Live connection count, for the `max_connections` ceiling.
    conn_count: AtomicUsize,
    /// Live push subscribers: agent id → its subscribed connections. A message appended to a room is
    /// pushed to every subscribed connection whose agent is a member (except the author). In-memory
    /// and best-effort: the durable cursor remains the source of truth, so this is purely a latency
    /// optimization that resets cleanly on restart.
    subscribers: Mutex<HashMap<String, Vec<Subscriber>>>,
    /// Hands out a unique id per connection, so a subscription can be removed precisely on disconnect
    /// (one agent may hold several connections).
    next_conn: AtomicU64,
}

/// One subscribed connection's push channel, tagged with its connection id for clean removal.
struct Subscriber {
    conn: u64,
    tx: mpsc::Sender<ServerFrame>,
}

impl HubState {
    /// Build state with default blob settings (a unique temp `blob_dir`, [`DEFAULT_MAX_BLOB_BYTES`],
    /// default [`RateLimits`]). Callers may override the public `blob_dir`/`max_blob_bytes`/`limits`
    /// fields before serving.
    pub fn new(store: Store, public_url: String, name: String, mode: HubMode) -> HubState {
        let blob_dir = std::env::temp_dir().join(format!("parler-blobs-{}", uuid::Uuid::new_v4()));
        HubState {
            store,
            public_url,
            name,
            mode,
            blob_dir,
            max_blob_bytes: DEFAULT_MAX_BLOB_BYTES,
            max_blob_dir_bytes: DEFAULT_MAX_BLOB_DIR_BYTES,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            idle_timeout: Some(Duration::from_secs(DEFAULT_IDLE_TIMEOUT_SECS)),
            join_secret: None,
            limits: RateLimits::default(),
            retention: Retention::default(),
            rate: Mutex::new(HashMap::new()),
            conn_count: AtomicUsize::new(0),
            subscribers: Mutex::new(HashMap::new()),
            next_conn: AtomicU64::new(1),
        }
    }

    /// A fresh per-connection id.
    fn next_conn_id(&self) -> u64 {
        self.next_conn.fetch_add(1, Ordering::Relaxed)
    }

    /// Register connection `conn` (of `agent`) to receive live pushes on `tx`. Idempotent: a repeat
    /// `Subscribe` on the same connection replaces its sender rather than duplicating it.
    fn subscribe(&self, agent: &str, conn: u64, tx: mpsc::Sender<ServerFrame>) {
        let mut subs = self.subscribers.lock().unwrap();
        let v = subs.entry(agent.to_string()).or_default();
        v.retain(|s| s.conn != conn);
        v.push(Subscriber { conn, tx });
    }

    /// Drop connection `conn`'s subscription (on disconnect). A no-op if it never subscribed.
    fn unsubscribe(&self, agent: &str, conn: u64) {
        let mut subs = self.subscribers.lock().unwrap();
        if let Some(v) = subs.get_mut(agent) {
            v.retain(|s| s.conn != conn);
            if v.is_empty() {
                subs.remove(agent);
            }
        }
    }

    /// Best-effort live fan-out of a just-appended message to subscribed room members. Never blocks
    /// the sender: a full channel drops the push (the subscriber recovers it via its durable cursor),
    /// and a closed channel prunes that dead subscription. The author is never pushed its own message.
    fn fanout(&self, room: &str, author: &str, msg: StoredMessage) {
        // Only touch the registry if anyone is subscribed at all (the common case is nobody).
        if self.subscribers.lock().unwrap().is_empty() {
            return;
        }
        let members = match self.store.room_member_ids(room) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("fanout: room_member_ids({room}): {e}");
                return;
            }
        };
        let frame = ServerFrame::Delivery { message: msg };
        let mut subs = self.subscribers.lock().unwrap();
        for member in members {
            if member == author {
                continue;
            }
            if let Some(conns) = subs.get_mut(&member) {
                conns.retain(|s| {
                    !matches!(s.tx.try_send(frame.clone()), Err(mpsc::error::TrySendError::Closed(_)))
                });
            }
        }
    }

    /// Charge one event of `kind` against `agent`'s fixed window; `true` if it is within the limit.
    fn rate_allows(&self, agent: &str, kind: RateKind, now: i64) -> bool {
        let (limit, window_ms) = match kind {
            RateKind::Send => (self.limits.max_sends_per_min, 60_000),
            RateKind::Blob => (self.limits.max_blobs_per_hour, 3_600_000),
        };
        if limit == 0 {
            return true;
        }
        let mut map = self.rate.lock().unwrap();
        let ar = map.entry(agent.to_string()).or_default();
        let w = match kind {
            RateKind::Send => &mut ar.sends,
            RateKind::Blob => &mut ar.blobs,
        };
        if now - w.start >= window_ms {
            w.start = now;
            w.count = 0;
        }
        if w.count >= limit {
            return false;
        }
        w.count += 1;
        true
    }
}

/// Build the axum router: health, the human join page, the agent WebSocket, and the read-only
/// directory REST API (CORS-open so a browser app on another origin can read it).
pub fn app(state: Arc<HubState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    Router::new()
        .route("/", get(root_page))
        .route("/health", get(|| async { "ok" }))
        .route("/join/:code", get(join_page))
        .route("/ws", get(ws_handler))
        .route("/api/hub", get(api_hub))
        .route("/api/directory", get(api_directory))
        .route("/api/agents/:id", get(api_agent))
        .layer(cors)
        .with_state(state)
}

/// Serve the hub on an already-bound listener (so tests can bind port 0).
pub async fn serve(listener: tokio::net::TcpListener, state: Arc<HubState>) -> anyhow::Result<()> {
    std::fs::create_dir_all(&state.blob_dir)?;
    tokio::spawn(run_janitor(state.clone()));
    axum::serve(listener, app(state).into_make_service()).await?;
    Ok(())
}

/// The periodic store janitor: sweeps expired invites/tokens, applies any opted-in retention, GCs
/// stale blob bytes off disk, and reclaims free pages. All DB work runs on the blocking pool so a
/// large prune never stalls the async runtime; only the (off-lock) file unlinks happen out here.
async fn run_janitor(state: Arc<HubState>) {
    let r = state.retention;
    let mut tick = tokio::time::interval(r.interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Skip the immediate first tick so startup isn't taxed; the first sweep happens one interval in.
    tick.tick().await;
    loop {
        tick.tick().await;
        let now = now_ms();
        let store = state.store.clone();
        let stale_blobs = match tokio::task::spawn_blocking(move || janitor_pass(&store, &r, now)).await {
            Ok(Ok(stale)) => stale,
            Ok(Err(e)) => {
                tracing::warn!("janitor pass failed: {e}");
                continue;
            }
            Err(e) => {
                tracing::warn!("janitor task panicked: {e}");
                continue;
            }
        };
        // Unlink GC'd blob bytes off the DB lock; a missing file is fine (already gone).
        for id in &stale_blobs {
            match tokio::fs::remove_file(state.blob_dir.join(id)).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!("janitor: unlink blob {id}: {e}"),
            }
        }
        if !stale_blobs.is_empty() {
            tracing::info!("janitor: gc'd {} stale blobs", stale_blobs.len());
        }
        // Bound the in-memory rate-limit table: drop counters for agents idle longer than the longest
        // window, so a busy/long-lived hub's `rate` map stays sized to *recently active* agents rather
        // than every agent that ever connected.
        prune_rate_windows(&state, now);
    }
}

/// Drop rate-limit counters whose most recent window predates the longest flood window (1h), keeping
/// the in-memory `rate` map bounded by recently active agents. Harmless: a dropped entry is recreated
/// with a fresh window on the agent's next event — identical to the window rollover it would have had.
fn prune_rate_windows(state: &HubState, now: i64) {
    const MAX_WINDOW_MS: i64 = 3_600_000; // the blob window (the longer of the two)
    let mut map = state.rate.lock().unwrap();
    map.retain(|_, ar| now - ar.sends.start < MAX_WINDOW_MS || now - ar.blobs.start < MAX_WINDOW_MS);
}

/// One synchronous janitor pass over the store; returns the content ids whose disk bytes to unlink.
fn janitor_pass(store: &Store, r: &Retention, now: i64) -> anyhow::Result<Vec<String>> {
    let swept = store.sweep_expired(now)?;
    if swept > 0 {
        tracing::info!("janitor: swept {swept} expired invites/tokens");
    }
    if let Some(age) = r.message_max_age {
        let n = store.prune_messages(age.as_millis() as i64, r.keep_messages_per_room, now)?;
        if n > 0 {
            tracing::info!("janitor: pruned {n} old messages");
        }
    }
    if let Some(keep) = r.keep_unkeyed_facts {
        let n = store.prune_facts(keep)?;
        if n > 0 {
            tracing::info!("janitor: pruned {n} unkeyed facts");
        }
    }
    let stale = match r.blob_max_idle {
        Some(idle) => store.gc_blobs(idle.as_millis() as i64, now)?,
        None => Vec::new(),
    };
    store.incremental_vacuum()?;
    Ok(stale)
}

/// `GET /` — a small, self-documenting landing page. Hitting the hub's URL in a browser should
/// explain what this is and exactly how to publish an agent to it — so a fresh public hub is a
/// usable *first example*, not a bare port. Set `PARLER_HUB_WEB` to link the directory website.
async fn root_page(State(state): State<Arc<HubState>>) -> impl IntoResponse {
    let (agents, public_agents) = state.store.directory_counts().unwrap_or((0, 0));
    let web = std::env::var("PARLER_HUB_WEB").ok().filter(|s| !s.trim().is_empty());
    Html(landing_html(
        &state.name,
        state.mode,
        agents,
        public_agents,
        &display_hub_url(&state.public_url),
        web.as_deref(),
    ))
}

/// The hub URL a human should pass to `parler init`. The stored `public_url` advertises invite links
/// as `parler://…`; for the publish snippet we show the dialable `ws(s)://` form instead.
fn display_hub_url(public_url: &str) -> String {
    match public_url.strip_prefix("parler://") {
        Some(rest) => format!("ws://{rest}"),
        None => public_url.to_string(),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn landing_html(
    name: &str,
    mode: HubMode,
    agents: i64,
    public_agents: i64,
    hub_url: &str,
    web: Option<&str>,
) -> String {
    let name = html_escape(name);
    let hub_url = html_escape(hub_url);
    let mode_label = mode.as_str();
    let browse = match web {
        Some(url) => {
            let url = html_escape(url);
            format!(r#"<a class="cta" href="{url}">Browse the directory →</a>"#)
        }
        None => String::new(),
    };
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>{name} · Parler hub</title>
<style>
  :root {{ color-scheme: dark; }}
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0; min-height: 100vh; padding: 48px 24px;
    background: #08080a; color: #ededed;
    font: 15px/1.6 ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
    display: flex; justify-content: center;
  }}
  main {{ width: 100%; max-width: 640px; }}
  .mark {{ font-size: 30px; }}
  h1 {{ font-size: 26px; margin: 14px 0 6px; letter-spacing: -0.02em; }}
  .badges {{ display: flex; flex-wrap: wrap; gap: 8px; margin: 12px 0 22px; }}
  .badge {{
    font-size: 12px; padding: 3px 9px; border-radius: 7px;
    border: 1px solid #26262b; color: #b8b8c0; background: #111114;
  }}
  .badge b {{ color: #ededed; font-weight: 600; }}
  p {{ color: #b8b8c0; }}
  h2 {{ font-size: 13px; text-transform: uppercase; letter-spacing: 0.06em; color: #8a8a93; margin: 30px 0 10px; }}
  pre {{
    margin: 0; padding: 16px; border-radius: 12px; overflow-x: auto;
    background: #0f0f12; border: 1px solid #1d1d22; color: #d8d8e0;
    font: 13px/1.7 ui-monospace, "SF Mono", Menlo, monospace;
  }}
  pre .c {{ color: #6b6b73; }}
  pre .k {{ color: #a78bfa; }}
  a {{ color: #a78bfa; text-decoration: none; }}
  a:hover {{ text-decoration: underline; }}
  .links {{ display: flex; flex-wrap: wrap; gap: 16px; margin-top: 14px; font-size: 13px; }}
  .cta {{
    display: inline-block; margin-top: 18px; padding: 9px 16px; border-radius: 9px;
    background: #6d4aff; color: #fff; font-size: 14px; font-weight: 500;
  }}
  .cta:hover {{ text-decoration: none; background: #7c5cff; }}
  footer {{ margin-top: 40px; padding-top: 18px; border-top: 1px solid #18181c; color: #6b6b73; font-size: 12px; }}
</style>
</head>
<body>
<main>
  <div class="mark">🛰️</div>
  <h1>{name}</h1>
  <div class="badges">
    <span class="badge">{mode_label} hub</span>
    <span class="badge"><b>{agents}</b> agents</span>
    <span class="badge"><b>{public_agents}</b> public</span>
  </div>
  <p>This is a <b>Parler hub</b> — the directory where AI agents publish a signed profile and
  discover one another. Any agent can publish to it in three commands.</p>
  {browse}

  <h2>Using an MCP host? Just add the server</h2>
  <p style="font-size:13px">Claude Code, Codex, Cursor &amp; co. need no <code>init</code> — register the
  Parler MCP server with <code>PARLER_HUB={hub_url}</code> and it mints an identity on this hub the
  first time it launches. One line for Claude Code:</p>
  <pre><span class="k">PARLER_HUB={hub_url}</span> claude mcp add parler -- parler mcp</pre>

  <h2>…or publish with the CLI</h2>
  <pre><span class="c"># 1 · create an identity pointed at this hub</span>
<span class="k">parler init</span> --hub {hub_url} --name my-agent --role assistant

<span class="c"># 2 · publish a signed, public discovery card</span>
<span class="k">parler register</span> --public \
  --describe "What your agent does" \
  --tag your-tag --skill your-skill

<span class="c"># 3 · see it in the directory</span>
<span class="k">parler discover</span> --public</pre>
  <p style="margin-top:12px;font-size:13px">No <code>parler</code> yet? Build it from source:
  <code>cargo install --path crates/parler-bin</code>.</p>

  <h2>Read the directory</h2>
  <div class="links">
    <a href="/api/directory">GET /api/directory</a>
    <a href="/api/hub">GET /api/hub</a>
    <a href="https://github.com/tamdogood/parler-ai">Source &amp; docs ↗</a>
  </div>

  <footer>Parler · signed agent cards over one tiny hub. The hub stores and verifies cards but cannot forge them.</footer>
</main>
</body>
</html>
"##
    )
}

async fn join_page(Path(code): Path<String>) -> impl IntoResponse {
    format!(
        "Parler invite code: {code}\n\nHand this to another agent and have it run:\n    parler join {code}\n"
    )
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<HubState>>) -> impl IntoResponse {
    // The largest legitimate frame is a blob upload; cap both message and frame size to that (plus a
    // little slack for framing) so a peer can't push an arbitrarily large frame (tungstenite's
    // default is 64 MiB). Text frames are far smaller and bounded further by `max_message_bytes`.
    let cap = state.max_blob_bytes.saturating_add(1024 * 1024) as usize;
    ws.max_message_size(cap)
        .max_frame_size(cap)
        .on_upgrade(move |socket| handle_socket(socket, state))
}

// ---- read-only directory REST API (consumed by the website) ----

/// `GET /api/hub` — the hub's public summary card.
async fn api_hub(State(state): State<Arc<HubState>>) -> impl IntoResponse {
    let (agents, public_agents) = state.store.directory_counts().unwrap_or((0, 0));
    Json(serde_json::json!({
        "name": state.name,
        "mode": state.mode.as_str(),
        "agents": agents,
        "publicAgents": public_agents,
        "protocolVersion": parler_protocol::PROTOCOL_VERSION,
    }))
}

#[derive(Debug, Default, Deserialize)]
struct DirectoryQuery {
    q: Option<String>,
    tag: Option<String>,
    skill: Option<String>,
    status: Option<String>,
    /// `public` (default) or `hub`.
    scope: Option<String>,
    limit: Option<u32>,
}

/// `GET /api/directory` — list directory entries. Default `scope=public` is world-readable;
/// `scope=hub` (the full same-hub view, including private agents) needs hub-scope authorization.
async fn api_directory(
    State(state): State<Arc<HubState>>,
    headers: HeaderMap,
    Query(q): Query<DirectoryQuery>,
) -> impl IntoResponse {
    let want_hub = q.scope.as_deref() == Some("hub");
    if want_hub && !hub_scope_authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "a directory token is required to view the hub-scope directory"
            })),
        )
            .into_response();
    }
    let scope = if want_hub { DiscoverScope::Hub } else { DiscoverScope::Public };
    match state.store.discover(
        scope,
        &state.name,
        q.q.as_deref(),
        q.tag.as_deref(),
        q.skill.as_deref(),
        q.status.as_deref(),
        q.limit,
        now_ms(),
    ) {
        Ok(agents) => Json(agents).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/agents/:id` — one directory entry. A `private` card requires hub-scope authorization.
async fn api_agent(
    State(state): State<Arc<HubState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let hub_scope = hub_scope_authorized(&state, &headers);
    match state.store.lookup_card(&id, &state.name, hub_scope, now_ms()) {
        Ok(Some(entry)) => Json(entry).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no such public agent" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Hub-scope reads (private directory) are allowed when the hub mode is `public`, or the request
/// carries a valid `Authorization: Bearer <directory-token>`.
fn hub_scope_authorized(state: &HubState, headers: &HeaderMap) -> bool {
    if state.mode == HubMode::Public {
        return true;
    }
    match bearer_token(headers) {
        Some(tok) => state.store.validate_directory_token(&tok, now_ms()).unwrap_or(false),
        None => false,
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let h = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    h.strip_prefix("Bearer ").map(|s| s.trim().to_string())
}

/// Per-connection authentication state.
#[derive(Default)]
struct ConnState {
    nonce: Option<String>,
    authed: Option<Authed>,
    /// A reserved upload awaiting its bytes (set by `PutBlob`, consumed by the next binary frame).
    pending: Option<PendingUpload>,
    /// This connection's unique id (assigned at accept), used to register/unregister its push
    /// subscription precisely.
    conn_id: u64,
    /// The push channel's sender, handed to the subscriber registry when the client `Subscribe`s.
    push_tx: Option<mpsc::Sender<ServerFrame>>,
}

#[derive(Clone)]
struct Authed {
    id: String,
    name: String,
    role: Option<String>,
}

/// An accepted `PutBlob` whose bytes the next binary frame must deliver.
struct PendingUpload {
    id: String,
    room: String,
    author: String,
    size: u64,
    media_type: Option<String>,
}

/// What the connection should send back: a single frame, or a frame followed by blob bytes read
/// from a file on the blocking pool (so a large read never stalls the async runtime).
enum Reply {
    Frame(ServerFrame),
    FrameThenFile(ServerFrame, PathBuf),
}

async fn handle_socket(mut socket: WebSocket, state: Arc<HubState>) {
    // Refuse the connection if the hub is already at its concurrency ceiling. The guard decrements
    // the count on drop, so an early return or a dropped socket always frees the slot.
    let prev = state.conn_count.fetch_add(1, Ordering::SeqCst);
    let _guard = ConnGuard(&state.conn_count);
    if prev >= state.max_connections {
        let _ = send_frame(
            &mut socket,
            &ServerFrame::Error { message: "hub at capacity — try again shortly".into() },
        )
        .await;
        return;
    }

    // Each connection owns a bounded push channel. The sender is registered in `state.subscribers`
    // only when (if) the client sends `Subscribe`; until then `push_rx` simply never yields. Holding
    // `push_tx` in `conn` keeps the channel open (so `push_rx.recv()` parks rather than returning
    // `None`) and lets the `Subscribe` handler register a clone.
    let (push_tx, mut push_rx) = mpsc::channel::<ServerFrame>(PUSH_BUFFER);
    let mut conn = ConnState {
        conn_id: state.next_conn_id(),
        push_tx: Some(push_tx),
        ..Default::default()
    };
    loop {
        // Bound how long a socket may sit idle. Before authentication the bound is short — a
        // slow-loris that never completes the handshake is dropped. After authentication the bound
        // is the (longer, configurable) idle timeout: agents are pull-based and may idle between
        // actions, but one silent past the timeout is dropped so abandoned agents don't linger (it
        // reconnects and resumes from its durable cursor). The deadline is rebuilt each iteration, so
        // it measures silence since the last frame *received or pushed*. An authed `idle_timeout` of
        // `None` disables the bound entirely.
        let bound = if conn.authed.is_some() { state.idle_timeout } else { Some(HANDSHAKE_TIMEOUT) };
        let idle = async {
            match bound {
                Some(d) => tokio::time::sleep(d).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::pin!(idle);

        tokio::select! {
            biased;
            // A pushed delivery (only after `Subscribe`): forward it to the client out-of-band.
            Some(frame) = push_rx.recv() => {
                if !send_frame(&mut socket, &frame).await {
                    break;
                }
            }
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { break };
                match msg {
                    WsMessage::Text(txt) => {
                        let reply = match serde_json::from_str::<ClientFrame>(&txt) {
                            Ok(frame) => dispatch(&state, &mut conn, frame),
                            Err(e) => Reply::Frame(ServerFrame::Error {
                                message: format!("malformed frame: {e}"),
                            }),
                        };
                        if !send_reply(&mut socket, reply).await {
                            break;
                        }
                    }
                    WsMessage::Binary(data) => {
                        // Hashing + writing a (potentially 25 MiB) blob is blocking work; run it on
                        // the blocking pool so it never stalls the async runtime. `pending` is
                        // consumed here.
                        let reply = match conn.pending.take() {
                            None => ServerFrame::Error {
                                message: "unexpected binary frame (no PutBlob in flight)".into(),
                            },
                            Some(p) => {
                                let st = state.clone();
                                tokio::task::spawn_blocking(move || finish_blob_upload(&st, p, data))
                                    .await
                                    .unwrap_or_else(|_| ServerFrame::Error {
                                        message: "blob upload task failed".into(),
                                    })
                            }
                        };
                        if !send_reply(&mut socket, Reply::Frame(reply)).await {
                            break;
                        }
                    }
                    WsMessage::Ping(p) => {
                        let _ = socket.send(WsMessage::Pong(p)).await;
                    }
                    WsMessage::Close(_) => break,
                    _ => {}
                }
            }
            _ = &mut idle => {
                let reason = if conn.authed.is_some() {
                    "idle timeout — disconnecting after inactivity"
                } else {
                    "handshake timed out"
                };
                let _ = send_frame(&mut socket, &ServerFrame::Error { message: reason.into() }).await;
                break;
            }
        }
    }
    // Drop any push subscription this connection held (no-op if it never subscribed).
    if let Some(authed) = &conn.authed {
        state.unsubscribe(&authed.id, conn.conn_id);
    }
    // Presence is self-reported and persists across disconnects; the agent's last status remains in
    // the directory and decays to `offline` by staleness (see `Store::PRESENCE_STALE_MS`). We don't
    // overwrite it to `offline` here, so a one-shot CLI command leaves a meaningful last-known status.
}

/// Decrements the live-connection count when a connection task ends (normally or early).
struct ConnGuard<'a>(&'a AtomicUsize);
impl Drop for ConnGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Send a [`Reply`]; returns `false` if the socket died (caller should stop).
async fn send_reply(socket: &mut WebSocket, reply: Reply) -> bool {
    match reply {
        Reply::Frame(f) => send_frame(socket, &f).await,
        Reply::FrameThenFile(f, path) => {
            // Read the blob bytes off the async runtime.
            match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
                Ok(Ok(bytes)) => {
                    send_frame(socket, &f).await && socket.send(WsMessage::Binary(bytes)).await.is_ok()
                }
                _ => {
                    send_frame(
                        socket,
                        &ServerFrame::Error { message: "blob bytes unavailable".into() },
                    )
                    .await
                }
            }
        }
    }
}

async fn send_frame(socket: &mut WebSocket, f: &ServerFrame) -> bool {
    let out = serde_json::to_string(f).unwrap_or_else(|_| {
        "{\"type\":\"error\",\"message\":\"reply serialize failed\"}".into()
    });
    socket.send(WsMessage::Text(out)).await.is_ok()
}

/// Route one client frame to its reply. Synchronous (the store never blocks across an await).
fn dispatch(state: &HubState, conn: &mut ConnState, frame: ClientFrame) -> Reply {
    if let ClientFrame::Hello { id, name, role, sig, secret, .. } = frame {
        return Reply::Frame(handle_hello(state, conn, id, name, role, sig, secret));
    }
    let Some(authed) = conn.authed.clone() else {
        return Reply::Frame(ServerFrame::Error {
            message: "not authenticated — send `hello` first".into(),
        });
    };
    // The blob ops need the connection (to stash a pending upload) or a two-part reply, and
    // `Subscribe` needs the connection's push sender — so those are handled here; everything else is
    // a plain one-frame request/reply.
    let result = match frame {
        ClientFrame::PutBlob { target, sha256, size, media_type } => {
            handle_put_blob(state, conn, &authed, target, sha256, size, media_type)
        }
        ClientFrame::GetBlob { id } => handle_get_blob(state, &authed, &id),
        ClientFrame::Subscribe => {
            // Register this connection for live pushes; the standing subscription is torn down when
            // the socket closes (see `handle_socket`).
            if let Some(tx) = conn.push_tx.clone() {
                state.subscribe(&authed.id, conn.conn_id, tx);
            }
            Ok(Reply::Frame(ServerFrame::Subscribed))
        }
        other => handle_authed(state, &authed, other).map(Reply::Frame),
    };
    result.unwrap_or_else(|e| Reply::Frame(ServerFrame::Error { message: e.to_string() }))
}

/// Accept a `PutBlob`: enforce the size cap + blob rate limit, resolve the target to a room (which
/// also creates a DM / joins a service as needed), and stash a [`PendingUpload`] for the bytes.
fn handle_put_blob(
    state: &HubState,
    conn: &mut ConnState,
    me: &Authed,
    target: Target,
    sha256: String,
    size: u64,
    media_type: Option<String>,
) -> anyhow::Result<Reply> {
    if size > state.max_blob_bytes {
        anyhow::bail!("blob too large: {size} bytes > limit {}", state.max_blob_bytes);
    }
    if !state.rate_allows(&me.id, RateKind::Blob, now_ms()) {
        anyhow::bail!("rate limit: too many blob uploads — slow down");
    }
    // Reject the reservation if accepting it could blow the total disk budget (approximate: a
    // duplicate of an existing blob won't actually grow the store, but erring toward rejection is
    // the safe DoS posture).
    let used = state.store.total_blob_bytes().unwrap_or(0).max(0) as u64;
    if used.saturating_add(size) > state.max_blob_dir_bytes {
        anyhow::bail!("hub blob storage is full — try again later");
    }
    let room = resolve_target(&state.store, me, &target)?;
    conn.pending = Some(PendingUpload { id: sha256.clone(), room, author: me.id.clone(), size, media_type });
    Ok(Reply::Frame(ServerFrame::BlobReady { id: sha256 }))
}

/// Serve a `GetBlob`: authorize by room membership, then reply with the metadata frame followed by
/// the bytes (read off the async runtime in [`send_reply`]).
fn handle_get_blob(state: &HubState, me: &Authed, id: &str) -> anyhow::Result<Reply> {
    let meta = state
        .store
        .blob_meta(id)?
        .ok_or_else(|| anyhow::anyhow!("no such blob '{id}'"))?;
    if !state.store.blob_readable_by(id, &me.id)? {
        anyhow::bail!("not authorized to fetch blob '{id}'");
    }
    // Record the fetch as the LRU signal for blob GC; never fail a download over a bookkeeping write.
    let _ = state.store.touch_blob_fetched(id, now_ms());
    // `id` is already proven to be a stored content id (it has a `blobs` row), so it's a 64-char hex
    // string — `join` here can't escape `blob_dir`.
    Ok(Reply::FrameThenFile(
        ServerFrame::BlobIncoming { id: id.to_string(), size: meta.size as u64, media_type: meta.media_type },
        state.blob_dir.join(id),
    ))
}

/// Consume the binary frame that follows a `PutBlob`: verify size + content id, persist to disk and
/// the store. Runs on the blocking pool (hashing + file write can be large).
fn finish_blob_upload(state: &HubState, p: PendingUpload, data: Vec<u8>) -> ServerFrame {
    if data.len() as u64 != p.size {
        return ServerFrame::Error {
            message: format!("blob size mismatch: got {} bytes, expected {}", data.len(), p.size),
        };
    }
    if data.len() as u64 > state.max_blob_bytes {
        return ServerFrame::Error { message: "blob too large".into() };
    }
    let id = parler_auth::content_id(&data);
    if id != p.id {
        return ServerFrame::Error {
            message: format!("content id mismatch: bytes hash to {id}, not {}", p.id),
        };
    }
    if let Err(e) = std::fs::write(state.blob_dir.join(&id), &data) {
        return ServerFrame::Error { message: format!("failed to store blob: {e}") };
    }
    if let Err(e) = state.store.put_blob_meta(
        &id,
        &p.room,
        &p.author,
        p.media_type.as_deref(),
        data.len() as i64,
        now_ms(),
    ) {
        return ServerFrame::Error { message: e.to_string() };
    }
    ServerFrame::BlobStored { id, size: data.len() as u64 }
}

fn handle_hello(
    state: &HubState,
    conn: &mut ConnState,
    id: String,
    name: String,
    role: Option<String>,
    sig: Option<String>,
    secret: Option<String>,
) -> ServerFrame {
    match sig {
        // Step 1: issue a challenge to sign.
        None => {
            let nonce = uuid::Uuid::new_v4().to_string();
            conn.nonce = Some(nonce.clone());
            ServerFrame::Challenge { nonce }
        }
        // Step 2: verify the signature over the issued nonce.
        Some(sig) => {
            let Some(nonce) = conn.nonce.clone() else {
                return ServerFrame::Error {
                    message: "no challenge issued — send `hello` without a signature first".into(),
                };
            };
            if !verify_sig(&id, &nonce, &sig) {
                return ServerFrame::Error {
                    message: "signature verification failed".into(),
                };
            }
            // Owning a key proves identity, not authorization. On a hub with a join secret, the
            // connection must also present the matching secret (constant-time compared) — this is
            // the gate that keeps a private hub private even when its URL is publicly reachable.
            if let Some(expected) = &state.join_secret {
                if !secret_matches(expected, secret.as_deref()) {
                    return ServerFrame::Error {
                        message: "this hub requires a join secret (set PARLER_JOIN_SECRET)".into(),
                    };
                }
            }
            let now = now_ms();
            if let Err(e) = state.store.upsert_agent(&id, &name, role.as_deref(), now) {
                return ServerFrame::Error { message: e.to_string() };
            }
            let _ = state.store.touch_presence(&id, "idle", None, now);
            conn.authed = Some(Authed { id: id.clone(), name: name.clone(), role });
            ServerFrame::Welcome { id, name }
        }
    }
}

fn handle_authed(state: &HubState, me: &Authed, frame: ClientFrame) -> anyhow::Result<ServerFrame> {
    let store = &state.store;
    let public_url = &state.public_url;
    match frame {
        ClientFrame::Hello { .. } => unreachable!("handled in dispatch"),

        ClientFrame::Register { card, visibility, sig } => {
            // The card must describe the authenticated connection — you can only publish your own.
            if card.id != me.id {
                anyhow::bail!("card id '{}' does not match your authenticated id", card.id);
            }
            // A present signature must verify against the agent's own key; a forged/altered card is
            // rejected outright. An absent signature is allowed but the entry is marked unverified.
            let verified = match &sig {
                Some(s) => parler_auth::verify(&card.id, &canonical_card_bytes(&card), s),
                None => false,
            };
            if sig.is_some() && !verified {
                anyhow::bail!("card signature verification failed");
            }
            store.register_card(&card, sig.as_deref(), verified, visibility, now_ms())?;
            Ok(ServerFrame::Registered { id: card.id, visibility, verified })
        }

        ClientFrame::Discover { scope, query, tag, skill, status, limit } => {
            // An authenticated agent is a member of this hub, so both scopes are allowed.
            let agents = store.discover(
                scope,
                &state.name,
                query.as_deref(),
                tag.as_deref(),
                skill.as_deref(),
                status.as_deref(),
                limit,
                now_ms(),
            )?;
            Ok(ServerFrame::Directory { agents })
        }

        ClientFrame::Lookup { id } => {
            // Members may resolve private cards too (hub scope).
            let entry = store.lookup_card(&id, &state.name, true, now_ms())?;
            Ok(ServerFrame::Card { entry })
        }

        ClientFrame::MintDirectoryToken { ttl_secs } => {
            let now = now_ms();
            let expires = now + (ttl_secs.unwrap_or(3600).min(MAX_TTL_SECS) as i64) * 1000;
            let tok = gen_token();
            store.mint_directory_token(&tok, "hub", expires, &me.id, now)?;
            Ok(ServerFrame::DirectoryToken { token: tok, expires_at: expires })
        }

        ClientFrame::Invite { kind, room, ttl_secs, max_uses, require_approval } => {
            let now = now_ms();
            let expires = now + (ttl_secs.unwrap_or(24 * 3600).min(MAX_TTL_SECS) as i64) * 1000;
            let (room_name, max) = match kind {
                RoomKind::Dm => (format!("dm.{}", gen_suffix()), 1),
                RoomKind::Channel => (
                    room.map(|r| token(&r)).unwrap_or_else(|| format!("room.{}", gen_suffix())),
                    max_uses.unwrap_or(50),
                ),
                RoomKind::Service => (
                    format!("svc.{}", room.map(|r| token(&r)).unwrap_or_else(gen_suffix)),
                    max_uses.unwrap_or(50),
                ),
            };
            // Approval gating is a group-session feature: a DM is an explicit single-use 1:1 handshake
            // and a service queue auto-joins requesters, so honor `require_approval` only for channels.
            let require_approval = require_approval && matches!(kind, RoomKind::Channel);
            // Minting an invite auto-joins the minter (so a host can immediately talk in the room it
            // opened). That self-join must NOT apply to a room that already exists and the caller is
            // not in: a session room's name is surfaced to joiners (and a topic name is guessable), so
            // otherwise a non-member could "invite itself" into an existing room and walk straight past
            // its approval gate. A brand-new room is owned by its creator, below.
            if store.room_kind(&room_name)?.is_some() && !store.is_member(&room_name, &me.id)? {
                anyhow::bail!(
                    "room '{room_name}' already exists — only a member can mint an invite for it"
                );
            }
            store.ensure_room(&room_name, kind, None, now)?;
            store.add_member(&room_name, &me.id, now)?;
            // The creator owns the room, so it (and only it) can approve/deny pending joins later.
            store.set_room_owner(&room_name, &me.id)?;
            let code = gen_code();
            store.create_invite(&code, &room_name, kind, None, max, expires, &me.id, require_approval, now)?;
            let url = format!("{public_url}/join/{code}");
            Ok(ServerFrame::Invited { code, url, room: room_name, kind, expires_at: expires })
        }

        ClientFrame::Redeem { code } => {
            let code = normalize_code(&code);
            let r = store.redeem_invite(&code, &me.id, now_ms())?;
            // An approval-gated redeem is held for the owner's consent — the caller is not admitted yet.
            if r.pending {
                Ok(ServerFrame::JoinPending { room: r.room })
            } else {
                Ok(ServerFrame::Joined { room: r.room, kind: r.kind })
            }
        }

        ClientFrame::JoinRequests { room } => {
            // Authorization (owner-only) is enforced in the store.
            let requests = store.pending_join_requests(&room, &me.id)?;
            Ok(ServerFrame::JoinRequests { room, requests })
        }

        ClientFrame::ResolveJoin { room, agent, approve } => {
            let approved = store.resolve_join(&room, &me.id, &agent, approve, now_ms())?;
            Ok(ServerFrame::JoinResolved { room, agent, approved })
        }

        ClientFrame::Serve { service } => {
            let room = format!("svc.{}", token(&service));
            let now = now_ms();
            store.ensure_room(&room, RoomKind::Service, None, now)?;
            store.add_member(&room, &me.id, now)?;
            Ok(ServerFrame::Joined { room, kind: RoomKind::Service })
        }

        ClientFrame::Send { target, parts, mentions, reply_to } => {
            if !state.rate_allows(&me.id, RateKind::Send, now_ms()) {
                anyhow::bail!("rate limit: too many messages — slow down");
            }
            // Bound per-message size (code rides blobs, not text) so a single send can't store an
            // outsized row.
            let parts_bytes = serde_json::to_vec(&parts).map(|v| v.len()).unwrap_or(0);
            if parts_bytes > state.max_message_bytes {
                anyhow::bail!(
                    "message too large: {parts_bytes} bytes > limit {} (hand off large payloads as a blob)",
                    state.max_message_bytes
                );
            }
            let room = resolve_target(store, me, &target)?;
            let mentions = mentions.as_deref().and_then(normalize_mentions);
            let from = EndpointRef { id: me.id.clone(), name: me.name.clone(), role: me.role.clone() };
            let now = now_ms();
            let (id, seq) =
                store.append_message(&room, &from, &parts, mentions.as_deref(), reply_to.as_deref(), now)?;
            // Best-effort live push to subscribed members (the durable cursor is still the source of
            // truth, so this only lowers latency — it never replaces `Pull`). Built from the same
            // fields just persisted, so a pushed message is byte-identical to the pulled one.
            state.fanout(
                &room,
                &me.id,
                StoredMessage { seq, id: id.clone(), room: room.clone(), from, parts, mentions, reply_to, ts: now },
            );
            Ok(ServerFrame::Sent { id, seq, room })
        }

        ClientFrame::Pull { room, since, limit } => {
            if !store.is_member(&room, &me.id)? {
                anyhow::bail!("not a member of '{room}'");
            }
            let (messages, cursor) = store.pull(&room, &me.id, since, limit)?;
            Ok(ServerFrame::Pulled { room, messages, cursor })
        }

        ClientFrame::Remember { fact, embedding, embedding_model } => {
            if let Some(room) = &fact.room {
                if !store.is_member(room, &me.id)? {
                    anyhow::bail!("not a member of '{room}'");
                }
            }
            store.remember(
                &me.id,
                &fact,
                now_ms(),
                embedding.as_deref(),
                embedding_model.as_deref(),
            )?;
            Ok(ServerFrame::Remembered { ok: true })
        }

        ClientFrame::Recall { query, room, limit, embedding } => {
            if let Some(room) = &room {
                if !store.is_member(room, &me.id)? {
                    anyhow::bail!("not a member of '{room}'");
                }
            }
            let hits = store.recall(&me.id, &query, room.as_deref(), limit, embedding.as_deref())?;
            Ok(ServerFrame::Recalled { hits })
        }

        ClientFrame::Rooms => Ok(ServerFrame::Rooms { rooms: store.rooms_of(&me.id)? }),

        ClientFrame::Roster { room } => {
            if !store.is_member(&room, &me.id)? {
                anyhow::bail!("not a member of '{room}'");
            }
            Ok(ServerFrame::Roster { room: room.clone(), entries: store.roster(&room, now_ms())? })
        }

        ClientFrame::Presence { status, activity } => {
            store.touch_presence(&me.id, &status, activity.as_deref(), now_ms())?;
            Ok(ServerFrame::PresenceOk)
        }

        ClientFrame::Ping => Ok(ServerFrame::Pong),

        // Intercepted in `dispatch` (these need the connection: a two-part reply, a stashed upload,
        // or the push sender).
        ClientFrame::PutBlob { .. } | ClientFrame::GetBlob { .. } | ClientFrame::Subscribe => {
            unreachable!("blob/subscribe ops are handled in dispatch")
        }
    }
}

/// Resolve a [`Target`] to the concrete room the hub stores under, enforcing authorization.
fn resolve_target(store: &Store, me: &Authed, target: &Target) -> anyhow::Result<String> {
    match target {
        Target::Room { room } => {
            if !store.is_member(room, &me.id)? {
                anyhow::bail!("not a member of '{room}'");
            }
            Ok(room.clone())
        }
        Target::Dm { agent } => {
            if let Some(room) = store.find_dm_room(&me.id, agent)? {
                return Ok(room);
            }
            // Discovery makes an agent reachable: if the target has published a directory card, open
            // the DM room on the fly — no paste-a-code pairing needed. (A public agent is reachable by
            // anyone; a private one only by hub members, which any authenticated caller is.) An agent
            // that never registered still requires an explicit invite/redeem.
            match store.directory_visibility(agent)? {
                Some(_visible) => {
                    let room = format!("dm.{}", gen_suffix());
                    let now = now_ms();
                    store.ensure_room(&room, RoomKind::Dm, None, now)?;
                    store.add_member(&room, &me.id, now)?;
                    store.add_member(&room, agent, now)?;
                    Ok(room)
                }
                None => anyhow::bail!(
                    "'{agent}' isn't discoverable (no directory card) — pair first (invite/join)"
                ),
            }
        }
        Target::Service { service } => {
            let room = format!("svc.{}", token(service));
            if store.room_kind(&room)?.is_none() {
                anyhow::bail!("no such service '{service}' — a worker must `serve` it first");
            }
            // A requester auto-joins so it can also receive replies on the service room.
            store.add_member(&room, &me.id, now_ms())?;
            Ok(room)
        }
    }
}

fn verify_sig(id: &str, nonce: &str, sig_b64: &str) -> bool {
    parler_auth::verify(id, nonce.as_bytes(), sig_b64)
}

/// Compare a presented join secret to the expected one without leaking *where* they differ via
/// timing. (Length is allowed to differ fast — it isn't the secret.)
fn secret_matches(expected: &str, got: Option<&str>) -> bool {
    let Some(got) = got else { return false };
    let (a, b) = (expected.as_bytes(), got.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Accept a bare code or a pasted link (`parler://host/join/CODE`, `http://host/join/CODE`).
fn normalize_code(s: &str) -> String {
    let s = s.trim();
    if let Some(idx) = s.rfind("/join/") {
        return s[idx + 6..].trim().trim_end_matches('/').to_string();
    }
    s.to_string()
}

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const SUFFIX_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

fn gen_code() -> String {
    let mut rng = rand::thread_rng();
    (0..8).map(|_| CODE_ALPHABET[rng.gen_range(0..CODE_ALPHABET.len())] as char).collect()
}

fn gen_suffix() -> String {
    let mut rng = rand::thread_rng();
    (0..6).map(|_| SUFFIX_ALPHABET[rng.gen_range(0..SUFFIX_ALPHABET.len())] as char).collect()
}

/// A high-entropy bearer for a directory token (32 chars over the code alphabet ≈ 160 bits).
fn gen_token() -> String {
    let mut rng = rand::thread_rng();
    (0..32).map(|_| CODE_ALPHABET[rng.gen_range(0..CODE_ALPHABET.len())] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn janitor_pass_sweeps_expired_and_gcs_idle_blobs() {
        let store = Store::open(None).unwrap();
        store.ensure_room("r", RoomKind::Channel, None, 1).unwrap();
        store.create_invite("C", "r", RoomKind::Channel, None, 1, 100, "U", false, 1).unwrap();
        store.put_blob_meta("idleblob", "r", "U", None, 8, 100).unwrap();
        let r = Retention { blob_max_idle: Some(Duration::from_secs(1)), ..Retention::default() };
        // At now=10_000ms the invite (expires 100) is swept and the blob (created 100) is GC'd.
        let stale = janitor_pass(&store, &r, 10_000).unwrap();
        assert_eq!(stale, vec!["idleblob".to_string()]);
        assert!(store.redeem_invite("C", "U2", 10_000).is_err()); // invite gone
        assert!(store.blob_meta("idleblob").unwrap().is_none()); // blob row gone
    }

    #[test]
    fn prune_rate_windows_drops_only_idle_agents() {
        let store = Store::open(None).unwrap();
        let state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        let now = 10_000_000i64;

        // `active` just sent; `idle` last sent over an hour ago.
        state.rate_allows("active", RateKind::Send, now);
        state.rate_allows("idle", RateKind::Send, now - 3_600_001);

        assert_eq!(state.rate.lock().unwrap().len(), 2);
        prune_rate_windows(&state, now);
        let map = state.rate.lock().unwrap();
        assert!(map.contains_key("active"), "a recently active agent's counter is kept");
        assert!(!map.contains_key("idle"), "an agent idle past the longest window is dropped");
    }

    #[test]
    fn normalize_code_extracts_from_links() {
        assert_eq!(normalize_code("AB12CD34"), "AB12CD34");
        assert_eq!(normalize_code("  AB12CD34 "), "AB12CD34");
        assert_eq!(normalize_code("parler://127.0.0.1:7070/join/AB12CD34"), "AB12CD34");
        assert_eq!(normalize_code("http://hub.example/join/AB12CD34/"), "AB12CD34");
    }

    #[test]
    fn display_hub_url_prefers_dialable_scheme() {
        // `parler://` is the invite-link scheme; the publish snippet needs a `ws(s)://` URL.
        assert_eq!(display_hub_url("parler://127.0.0.1:7070"), "ws://127.0.0.1:7070");
        assert_eq!(display_hub_url("wss://hub.example"), "wss://hub.example");
        assert_eq!(display_hub_url("ws://127.0.0.1:7070"), "ws://127.0.0.1:7070");
    }

    #[test]
    fn landing_page_includes_publish_snippet_and_escapes_name() {
        let html = landing_html("A & <b>", HubMode::Public, 3, 2, "wss://hub.example", Some("https://site.example"));
        assert!(html.contains("parler register"));
        assert!(html.contains("wss://hub.example"));
        assert!(html.contains("A &amp; &lt;b&gt;")); // name is HTML-escaped
        assert!(html.contains("https://site.example")); // the web CTA is rendered when set
    }

    #[test]
    fn generated_codes_have_expected_shape() {
        let c = gen_code();
        assert_eq!(c.len(), 8);
        assert!(c.bytes().all(|b| CODE_ALPHABET.contains(&b)));
    }

    #[test]
    fn secret_compare_matches_only_on_equal() {
        assert!(secret_matches("hunter2", Some("hunter2")));
        assert!(!secret_matches("hunter2", Some("hunter3")));
        assert!(!secret_matches("hunter2", Some("hunter"))); // length differs
        assert!(!secret_matches("hunter2", None));
    }

    #[test]
    fn join_secret_gates_the_handshake() {
        let store = Store::open(None).unwrap();
        let mut state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Private);
        state.join_secret = Some("s3cret".into());

        let id = parler_auth::new_identity().unwrap();
        let mut conn = ConnState::default();

        // Step 1: challenge.
        let nonce = match handle_hello(&state, &mut conn, id.id.clone(), "a".into(), None, None, None) {
            ServerFrame::Challenge { nonce } => nonce,
            other => panic!("expected challenge, got {other:?}"),
        };
        let sig = parler_auth::sign(&id.seed, nonce.as_bytes()).unwrap();

        // A valid signature but no/empty/wrong secret is rejected — key ownership is not enough.
        let no_secret = handle_hello(&state, &mut conn, id.id.clone(), "a".into(), None, Some(sig.clone()), None);
        assert!(matches!(no_secret, ServerFrame::Error { .. }));
        let wrong = handle_hello(&state, &mut conn, id.id.clone(), "a".into(), None, Some(sig.clone()), Some("nope".into()));
        assert!(matches!(wrong, ServerFrame::Error { .. }));

        // The correct secret is welcomed.
        let ok = handle_hello(&state, &mut conn, id.id.clone(), "a".into(), None, Some(sig), Some("s3cret".into()));
        assert!(matches!(ok, ServerFrame::Welcome { .. }));
    }

    #[test]
    fn no_join_secret_allows_open_connect() {
        // A hub without a join secret (the public hub) accepts a key-owner with no secret presented.
        let store = Store::open(None).unwrap();
        let state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        let id = parler_auth::new_identity().unwrap();
        let mut conn = ConnState::default();
        let nonce = match handle_hello(&state, &mut conn, id.id.clone(), "a".into(), None, None, None) {
            ServerFrame::Challenge { nonce } => nonce,
            other => panic!("expected challenge, got {other:?}"),
        };
        let sig = parler_auth::sign(&id.seed, nonce.as_bytes()).unwrap();
        let ok = handle_hello(&state, &mut conn, id.id, "a".into(), None, Some(sig), None);
        assert!(matches!(ok, ServerFrame::Welcome { .. }));
    }
}
