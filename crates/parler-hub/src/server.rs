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
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use parler_protocol::{
    canonical_card_bytes, error_code, is_message_sig_part, normalize_mentions, token, BundleRef,
    ClientFrame, CodedError, DirectoryEntry, DiscoverScope, EndpointRef, FileRef, Part, RoomKind,
    ServerFrame, StoredMessage, Target,
};
use rand::Rng;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Notify, OwnedSemaphorePermit, Semaphore};
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

/// Default cap on a JSON WebSocket frame. Binary blob frames retain their separate blob cap; every
/// structured operation fits comfortably below this bound, preventing card/memory/control frames
/// from borrowing the much larger binary allowance.
pub const DEFAULT_MAX_TEXT_FRAME_BYTES: usize = 2 * 1024 * 1024;

/// Aggregate bytes reserved by concurrent blob uploads. The current wire format sends one complete
/// binary frame, so this backpressure bounds the blocking/hash/write work even before blobs become
/// streamable in a future additive protocol version.
pub const DEFAULT_MAX_INFLIGHT_BLOB_BYTES: usize = 50 * 1024 * 1024;

/// Default ceiling on concurrent WebSocket connections to one hub.
pub const DEFAULT_MAX_CONNECTIONS: usize = 1024;

/// Durable object quotas per authenticated identity. They bound slow storage exhaustion that stays
/// below a short fixed-window rate limit.
pub const DEFAULT_MAX_OWNED_ROOMS: u64 = 1_000;
pub const DEFAULT_MAX_ACTIVE_TOKENS: u64 = 1_000;
pub const DEFAULT_MAX_KEYED_FACTS: u64 = 10_000;

/// Default per-client-IP HTTP request budget over a fixed 60s window, spanning the whole public front
/// door: the REST/A2A directory + session-viewer endpoints *and* the `/ws` upgrade. Unlike the
/// per-agent [`RateLimits`] (which only apply once a socket has authenticated), this bounds
/// *unauthenticated* abuse — directory/session scraping and connection/registration floods — from any
/// single source. 600/min (10/s) sits far above what a real agent or a session viewer needs while
/// still throttling a flood. `0` disables it. In-memory, resets on restart.
pub const DEFAULT_MAX_HTTP_PER_MIN: u32 = 600;

/// Per-client-IP budget for waitlist signups (`POST /api/waitlist`) over a fixed 60s window, on top of
/// the whole-front-door [`DEFAULT_MAX_HTTP_PER_MIN`] guard. A real human submits the form once; a
/// conservative allowance here throttles a single source from filling the `waitlist` table with junk
/// addresses. In-memory, resets on restart. Same fixed-window shape as the other rate guards.
pub const WAITLIST_MAX_PER_MIN: u32 = 10;

/// Default per-**room** send ceiling over a fixed 60s window, counting the *aggregate* of every member.
/// The per-agent [`RateLimits::max_sends_per_min`] bounds one agent; this bounds one *room*, so a busy
/// or abusive room full of agents can't monopolize the hub's single SQLite writer and starve every
/// other room (the noisy-neighbor / write-DoS vector on a shared multi-tenant hub). Set well above a
/// lively multi-agent conversation, so it only trips on runaway traffic. `0` disables. In-memory.
pub const DEFAULT_MAX_ROOM_SENDS_PER_MIN: u32 = 1200;

/// Default per-**room** blob-upload ceiling over a fixed 60-minute window. The hub's blob disk budget
/// ([`DEFAULT_MAX_BLOB_DIR_BYTES`]) is shared across all rooms; this bounds how fast one room can
/// consume it, so a single room can't fill storage (`STORAGE_FULL`) for everyone else. `0` disables.
pub const DEFAULT_MAX_ROOM_BLOBS_PER_HOUR: u32 = 600;

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

/// Hard ceiling on a client-supplied `Pull { wait_secs }` (and `join_session` wait). A parked
/// long-poll never sleeps longer than this, so a bug or a hostile client can't hold a connection
/// past a bound the hub controls. Kept under the idle timeout so a parked wait counts as activity
/// without ever outliving the connection's own liveness window.
const MAX_WAIT_SECS: u64 = 60;

/// Flood limits (fixed-window), applied at two scopes: **per-agent** (one agent's own budget) and
/// **per-room** (the aggregate budget of a whole room, so one busy room can't starve the shared hub).
/// `0` disables a limit. State is in-memory and resets on hub restart — a deliberately simple posture
/// for a low-ops bus.
#[derive(Debug, Clone, Copy)]
pub struct RateLimits {
    /// Total authenticated operations per agent per minute, including read/control frames. This
    /// closes the post-upgrade gap where HTTP rate limiting no longer sees WebSocket traffic.
    pub max_ops_per_min: u32,
    pub max_sends_per_min: u32,
    pub max_blobs_per_hour: u32,
    /// Per-room send ceiling (aggregate over all members) per 60s window. `0` disables.
    pub max_room_sends_per_min: u32,
    /// Per-room blob-upload ceiling (aggregate over all members) per 60-minute window. `0` disables.
    pub max_room_blobs_per_hour: u32,
}

impl Default for RateLimits {
    fn default() -> Self {
        RateLimits {
            max_ops_per_min: 600,
            max_sends_per_min: 240,
            max_blobs_per_hour: 120,
            max_room_sends_per_min: DEFAULT_MAX_ROOM_SENDS_PER_MIN,
            max_room_blobs_per_hour: DEFAULT_MAX_ROOM_BLOBS_PER_HOUR,
        }
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
        // Sane, conservative bounds *on* by default so a long-lived hub can't grow without limit —
        // the ceiling `docs/storage-and-memory.md` warned about. An operator can widen or disable any
        // knob (a `0`/negative flag ⇒ keep-everything). The per-room floor means recent history is
        // never trimmed by age, only old backlog beyond it.
        Retention {
            message_max_age: Some(Duration::from_secs(30 * 24 * 3600)), // 30 days
            keep_messages_per_room: 10_000,
            keep_unkeyed_facts: Some(500),
            blob_max_idle: Some(Duration::from_secs(14 * 24 * 3600)), // 14 days
            interval: Duration::from_secs(3600),
        }
    }
}

#[derive(Clone, Copy)]
enum RateKind {
    Operation,
    Send,
    Blob,
}

/// A single fixed-window counter.
#[derive(Default, Clone, Copy)]
struct Window {
    start: i64,
    count: u32,
}

/// Charge one event against `window` (a fixed `window_ms` bucket) and report whether it is within
/// `limit`. A `limit` of `0` disables the guard (always allowed). Shared by the per-agent
/// ([`HubState::rate_allows`]) and per-room ([`HubState::room_rate_allows`]) limiters — same math, the
/// only difference is which key the window is looked up by.
fn charge_window(window: &mut Window, limit: u32, window_ms: i64, now: i64) -> bool {
    if limit == 0 {
        return true;
    }
    if now - window.start >= window_ms {
        window.start = now;
        window.count = 0;
    }
    if window.count >= limit {
        return false;
    }
    window.count += 1;
    true
}

/// The two fixed windows charged together for one key — an agent id (per-agent limits) or a room name
/// (per-room limits). Reused by both limiters; the key type is the only difference between them.
#[derive(Default)]
struct RateWindows {
    operations: Window,
    sends: Window,
    blobs: Window,
}

/// Cumulative, lock-free hub counters for observability — surfaced under `/api/hub` so an operator can
/// watch throughput without a metrics backend. In-memory, so they reset on restart (like the rate
/// windows and the subscriber registry).
#[derive(Default)]
struct Metrics {
    /// Connections accepted (past the capacity gate) since boot.
    connections_total: AtomicU64,
    /// Messages appended to any room since boot.
    messages_total: AtomicU64,
    /// Estimated tokens carried by those messages since boot — the hub's total communication cost (an
    /// estimate; see `estimate_message_tokens`, the hub is a relay, not an LLM).
    tokens_total: AtomicU64,
    /// Live pushes delivered to subscribers since boot (a dropped/best-effort push is not counted).
    pushes_total: AtomicU64,
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
    /// Largest structured JSON frame accepted after WebSocket reassembly.
    pub max_text_frame_bytes: usize,
    /// Ceiling on concurrent connections; once reached, new sockets are refused.
    pub max_connections: usize,
    /// Aggregate reservation pool for simultaneous binary uploads.
    inflight_blob_bytes: Arc<Semaphore>,
    /// Durable per-identity quotas for attacker-controlled rows that otherwise survive rate-window
    /// resets indefinitely.
    pub max_owned_rooms: u64,
    pub max_active_tokens: u64,
    pub max_keyed_facts: u64,
    /// Serializes each durable quota's check-and-write section. The hub is a single process, so this
    /// closes concurrent-request races without adding protocol-visible database transactions.
    durable_quota: Mutex<()>,
    /// How long an authenticated connection may stay silent before the hub drops it. `None` keeps
    /// connections open indefinitely. Defaults to [`DEFAULT_IDLE_TIMEOUT_SECS`].
    pub idle_timeout: Option<Duration>,
    /// Optional shared join secret. When set, a connection must present a matching `secret` on its
    /// signed `Hello` to authenticate — the access gate for a closed/private hub. `None` ⇒ open.
    pub join_secret: Option<String>,
    /// Per-agent and per-room flood limits (authenticated WS ops).
    pub limits: RateLimits,
    /// Per-client-IP HTTP request budget per 60s window across the public front door (REST + `/ws`
    /// upgrade). `0` disables. Defaults to [`DEFAULT_MAX_HTTP_PER_MIN`].
    pub max_http_per_min: u32,
    /// Trust edge-provided client-IP headers. Disabled by default so a directly exposed hub cannot
    /// have its per-IP limiter bypassed with spoofed `X-Forwarded-For` values.
    pub trust_proxy_headers: bool,
    /// How the background janitor bounds append-only growth (defaults to keep-everything).
    pub retention: Retention,
    /// In-memory rate-limit counters, keyed by agent id (resets on restart).
    rate: Mutex<HashMap<String, RateWindows>>,
    /// In-memory per-**room** rate-limit counters, keyed by room name (resets on restart). Bounds the
    /// aggregate send/blob traffic of a single room so one noisy room can't starve the shared writer or
    /// blob disk. Pruned by the janitor alongside `rate`.
    room_rate: Mutex<HashMap<String, RateWindows>>,
    /// In-memory per-IP HTTP rate windows (resets on restart), pruned by the janitor alongside `rate`.
    http_rate: Mutex<HashMap<IpAddr, Window>>,
    /// In-memory per-IP windows for the tighter waitlist-signup budget ([`WAITLIST_MAX_PER_MIN`]),
    /// separate from `http_rate` so a signup flood is bounded independently of general API reads.
    /// Pruned by the janitor alongside `http_rate`.
    waitlist_rate: Mutex<HashMap<IpAddr, Window>>,
    /// Live connection count, for the `max_connections` ceiling.
    conn_count: AtomicUsize,
    /// Live push subscribers: agent id → its subscribed connections. A message appended to a room is
    /// pushed to every subscribed connection whose agent is a member (except the author). In-memory
    /// and best-effort: the durable cursor remains the source of truth, so this is purely a latency
    /// optimization that resets cleanly on restart.
    subscribers: Mutex<HashMap<String, Vec<Subscriber>>>,
    /// Per-room wakeups for **parked long-polls** (`Pull { wait_secs }` / `join_session` wait). A
    /// waiter that finds the backlog empty parks on the room's [`Notify`]; a message append or a
    /// membership change (`resolve_join`) on that room calls `notify_waiters`, waking every parked
    /// request to re-check. Pure in-memory and independent of the durable cursor — a waiter always
    /// resolves through a normal `Pull`, so a missed notify is harmless (the timer still fires and the
    /// next `Pull` returns anything that landed). Created lazily; an entry is never removed (bounded by
    /// the room count, and rooms are long-lived), so a wake never races a concurrent removal.
    notifiers: Mutex<HashMap<String, Arc<Notify>>>,
    /// Hands out a unique id per connection, so a subscription can be removed precisely on disconnect
    /// (one agent may hold several connections).
    next_conn: AtomicU64,
    /// Cumulative counters for observability (reset on restart).
    metrics: Metrics,
}

/// One subscribed connection's push channel, tagged with its connection id for clean removal.
/// Carries `Arc<ServerFrame>` so a fan-out to N members shares one frame instead of cloning it N times.
struct Subscriber {
    conn: u64,
    tx: mpsc::Sender<Arc<ServerFrame>>,
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
            max_text_frame_bytes: DEFAULT_MAX_TEXT_FRAME_BYTES,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            inflight_blob_bytes: Arc::new(Semaphore::new(DEFAULT_MAX_INFLIGHT_BLOB_BYTES)),
            max_owned_rooms: DEFAULT_MAX_OWNED_ROOMS,
            max_active_tokens: DEFAULT_MAX_ACTIVE_TOKENS,
            max_keyed_facts: DEFAULT_MAX_KEYED_FACTS,
            durable_quota: Mutex::new(()),
            idle_timeout: Some(Duration::from_secs(DEFAULT_IDLE_TIMEOUT_SECS)),
            join_secret: None,
            limits: RateLimits::default(),
            max_http_per_min: DEFAULT_MAX_HTTP_PER_MIN,
            trust_proxy_headers: false,
            retention: Retention::default(),
            rate: Mutex::new(HashMap::new()),
            room_rate: Mutex::new(HashMap::new()),
            http_rate: Mutex::new(HashMap::new()),
            waitlist_rate: Mutex::new(HashMap::new()),
            conn_count: AtomicUsize::new(0),
            subscribers: Mutex::new(HashMap::new()),
            notifiers: Mutex::new(HashMap::new()),
            next_conn: AtomicU64::new(1),
            metrics: Metrics::default(),
        }
    }

    /// Replace the aggregate upload reservation budget before the state is shared by the server.
    pub fn set_max_inflight_blob_bytes(&mut self, bytes: usize) {
        self.inflight_blob_bytes = Arc::new(Semaphore::new(bytes.max(1)));
    }

    /// A fresh per-connection id.
    fn next_conn_id(&self) -> u64 {
        self.next_conn.fetch_add(1, Ordering::Relaxed)
    }

    /// Register connection `conn` (of `agent`) to receive live pushes on `tx`. Idempotent: a repeat
    /// `Subscribe` on the same connection replaces its sender rather than duplicating it.
    fn subscribe(&self, agent: &str, conn: u64, tx: mpsc::Sender<Arc<ServerFrame>>) {
        let mut subs = self.subscribers.lock();
        let v = subs.entry(agent.to_string()).or_default();
        v.retain(|s| s.conn != conn);
        v.push(Subscriber { conn, tx });
    }

    /// Drop connection `conn`'s subscription (on disconnect). A no-op if it never subscribed.
    fn unsubscribe(&self, agent: &str, conn: u64) {
        let mut subs = self.subscribers.lock();
        if let Some(v) = subs.get_mut(agent) {
            v.retain(|s| s.conn != conn);
            if v.is_empty() {
                subs.remove(agent);
            }
        }
    }

    /// The wakeup handle for `room`'s parked long-polls, created on first use. Cloned out under the
    /// lock so a waiter can `notified().await` without holding it.
    fn room_notify(&self, room: &str) -> Arc<Notify> {
        self.notifiers.lock().entry(room.to_string()).or_default().clone()
    }

    /// Wake every request parked on `room` (a message landed or a membership changed), so each
    /// re-runs its `Pull`/redeem and completes if it now has a result. A no-op if nobody is parked.
    fn notify_room(&self, room: &str) {
        if let Some(n) = self.notifiers.lock().get(room) {
            n.notify_waiters();
        }
    }

    /// Best-effort live fan-out of a just-appended message to subscribed room members. Never blocks
    /// the sender: a full channel drops the push (the subscriber recovers it via its durable cursor),
    /// and a closed channel prunes that dead subscription. The author is never pushed its own message.
    fn fanout(&self, room: &str, author: &str, msg: StoredMessage) {
        // Wake any parked long-poll on this room first — server-side wait works with zero push
        // machinery, so a parked `Pull { wait_secs }` completes even when nobody `Subscribe`d.
        self.notify_room(room);
        // Only touch the registry if anyone is subscribed at all (the common case is nobody).
        if self.subscribers.lock().is_empty() {
            return;
        }
        let members = match self.store.room_member_ids(room) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("fanout: room_member_ids({room}): {e}");
                return;
            }
        };
        // One heap allocation shared by every recipient: `Arc::clone` hands each subscriber a pointer,
        // not a deep copy of the (possibly multi-KB) `Delivery` frame.
        let frame = Arc::new(ServerFrame::Delivery { message: msg });
        let mut subs = self.subscribers.lock();
        for member in members {
            if member == author {
                continue;
            }
            if let Some(conns) = subs.get_mut(&member) {
                conns.retain(|s| match s.tx.try_send(frame.clone()) {
                    Ok(()) => {
                        self.metrics.pushes_total.fetch_add(1, Ordering::Relaxed);
                        true
                    }
                    // Full ⇒ drop this push (the subscriber recovers it via its durable cursor) but
                    // keep the subscription; only a closed channel means the connection is gone.
                    Err(mpsc::error::TrySendError::Full(_)) => true,
                    Err(mpsc::error::TrySendError::Closed(_)) => false,
                });
            }
        }
    }

    /// Charge one event of `kind` against `agent`'s fixed window; `true` if it is within the limit.
    fn rate_allows(&self, agent: &str, kind: RateKind, now: i64) -> bool {
        let (limit, window_ms) = match kind {
            RateKind::Operation => (self.limits.max_ops_per_min, 60_000),
            RateKind::Send => (self.limits.max_sends_per_min, 60_000),
            RateKind::Blob => (self.limits.max_blobs_per_hour, 3_600_000),
        };
        if limit == 0 {
            return true;
        }
        let mut map = self.rate.lock();
        let ar = map.entry(agent.to_string()).or_default();
        let w = match kind {
            RateKind::Operation => &mut ar.operations,
            RateKind::Send => &mut ar.sends,
            RateKind::Blob => &mut ar.blobs,
        };
        charge_window(w, limit, window_ms, now)
    }

    /// Charge one event of `kind` against `room`'s fixed window; `true` if within the per-room limit.
    /// The room-level twin of [`rate_allows`]: it caps the *aggregate* traffic of one room across every
    /// member, so a single busy/abusive room can't monopolize the hub's shared SQLite writer (sends) or
    /// blob disk budget (blobs) — the noisy-neighbor bound on a multi-tenant hub. `0` disables.
    fn room_rate_allows(&self, room: &str, kind: RateKind, now: i64) -> bool {
        let (limit, window_ms) = match kind {
            RateKind::Operation => return true,
            RateKind::Send => (self.limits.max_room_sends_per_min, 60_000),
            RateKind::Blob => (self.limits.max_room_blobs_per_hour, 3_600_000),
        };
        if limit == 0 {
            return true;
        }
        let mut map = self.room_rate.lock();
        let ar = map.entry(room.to_string()).or_default();
        let w = match kind {
            RateKind::Operation => unreachable!("operation limits are per-agent only"),
            RateKind::Send => &mut ar.sends,
            RateKind::Blob => &mut ar.blobs,
        };
        charge_window(w, limit, window_ms, now)
    }

    /// Charge one HTTP request against `ip`'s fixed 60s window; `true` if it is within
    /// [`HubState::max_http_per_min`] (a `0` budget disables the limit). Same fixed-window shape as
    /// [`rate_allows`], keyed by source IP rather than agent id so it gates *unauthenticated* traffic.
    fn http_rate_allows(&self, ip: IpAddr, now: i64) -> bool {
        let limit = self.max_http_per_min;
        if limit == 0 {
            return true;
        }
        let mut map = self.http_rate.lock();
        let w = map.entry(ip).or_default();
        if now - w.start >= 60_000 {
            w.start = now;
            w.count = 0;
        }
        if w.count >= limit {
            return false;
        }
        w.count += 1;
        true
    }

    /// Charge one waitlist signup against `ip`'s tighter fixed 60s window; `true` if within
    /// [`WAITLIST_MAX_PER_MIN`]. Same fixed-window shape as [`http_rate_allows`], but keyed into the
    /// separate `waitlist_rate` map so a signup flood is bounded on its own budget.
    fn waitlist_rate_allows(&self, ip: IpAddr, now: i64) -> bool {
        let mut map = self.waitlist_rate.lock();
        let w = map.entry(ip).or_default();
        if now - w.start >= 60_000 {
            w.start = now;
            w.count = 0;
        }
        if w.count >= WAITLIST_MAX_PER_MIN {
            return false;
        }
        w.count += 1;
        true
    }
}

/// Resolve the client IP to rate-limit a request by. Forwarded headers are considered only when the
/// operator explicitly enables proxy trust; otherwise a directly exposed client could choose a new
/// header value for every request and bypass the limiter. The socket peer is always the fallback.
fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>, trust_proxy_headers: bool) -> Option<IpAddr> {
    if trust_proxy_headers {
        if let Some(ip) = headers
            .get("fly-client-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse().ok())
        {
            return Some(ip);
        }
        if let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .and_then(|s| s.trim().parse().ok())
        {
            return Some(ip);
        }
    }
    peer
}

/// Per-IP flood guard on the public HTTP front door (REST/A2A endpoints + the `/ws` upgrade). Runs
/// before every route except `/health` (Fly's liveness probe must never be throttled). A request over
/// the budget gets `429 Too Many Requests` with a `Retry-After`. When the source IP can't be resolved
/// (no forwarded header and no connect-info — e.g. a caller of [`app`] that didn't attach it) the
/// request is allowed through: fail-open, since there is no key to charge.
async fn rate_limit(State(state): State<Arc<HubState>>, req: Request, next: Next) -> axum::response::Response {
    if req.uri().path() == "/health" {
        return next.run(req).await;
    }
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|ci| ci.0.ip());
    if let Some(ip) = client_ip(req.headers(), peer, state.trust_proxy_headers) {
        if !state.http_rate_allows(ip, now_ms()) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(axum::http::header::RETRY_AFTER, "60")],
                "rate limit: too many requests — slow down\n",
            )
                .into_response();
        }
    }
    next.run(req).await
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
        .route("/api/session", get(api_session))
        // Download one file the session exchanged — a code bundle or handed-off file — gated by the
        // same watch token, scoped to that one room's blobs. Read-only, `attachment` + `nosniff`.
        .route("/api/session/blob/:id", get(api_session_blob))
        // The website's waitlist form posts signups here (self-hosted "owned email list"). CORS-open
        // like the reads above, since the form posts cross-origin from the marketing site.
        .route("/api/waitlist", post(api_waitlist))
        // A2A interoperability (discovery): project our signed cards into A2A AgentCard JSON so the
        // A2A ecosystem can find a Parler Protocol agent at the standard well-known location. See
        // `docs/a2a-interop.md`.
        .route("/.well-known/agent-card.json", get(a2a_well_known))
        // The hub's own capability descriptor (protocol version + push/long-poll/blobs/join policy),
        // so a client can probe before handshaking. See `hub_capabilities`.
        .route("/.well-known/parler.json", get(parler_well_known))
        .route("/a2a/directory", get(a2a_directory))
        .route("/a2a/agents/:id", get(a2a_agent))
        // Per-IP flood guard, inside CORS so a preflight `OPTIONS` is answered (and not counted) by
        // the CORS layer before it reaches the limiter.
        .layer(middleware::from_fn_with_state(state.clone(), rate_limit))
        .layer(cors)
        .with_state(state)
}

/// Serve the hub on an already-bound listener (so tests can bind port 0).
pub async fn serve(listener: tokio::net::TcpListener, state: Arc<HubState>) -> anyhow::Result<()> {
    std::fs::create_dir_all(&state.blob_dir)?;
    tokio::spawn(run_janitor(state.clone()));
    // `_with_connect_info` so the per-IP [`rate_limit`] layer can key a direct (un-proxied)
    // connection by its socket peer when no `Fly-Client-IP`/`X-Forwarded-For` header is present.
    axum::serve(listener, app(state).into_make_service_with_connect_info::<SocketAddr>()).await?;
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
    {
        let mut map = state.rate.lock();
        map.retain(|_, ar| {
            now - ar.operations.start < MAX_WINDOW_MS
                || now - ar.sends.start < MAX_WINDOW_MS
                || now - ar.blobs.start < MAX_WINDOW_MS
        });
    }
    // Per-room windows share the same shape and bound (the 60-minute blob window is the longer of the
    // two); drop any room idle past it so the map stays sized to recently-active rooms.
    {
        let mut map = state.room_rate.lock();
        map.retain(|_, ar| now - ar.sends.start < MAX_WINDOW_MS || now - ar.blobs.start < MAX_WINDOW_MS);
    }
    // Per-IP HTTP windows are 60s; drop any whose window has fully elapsed so the map stays sized to
    // recently-active sources rather than every IP that ever hit the hub. Same for the waitlist window.
    state.http_rate.lock().retain(|_, w| now - w.start < 60_000);
    state.waitlist_rate.lock().retain(|_, w| now - w.start < 60_000);
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
        state.join_secret.is_some(),
    ))
}

/// The dialable hub URL a human should paste into `PARLER_HUB` (or `parler init --hub`). The stored
/// `public_url` advertises invite links as `parler://…`; for a connect snippet we show the `ws(s)://`
/// form. A wildcard bind host (`0.0.0.0` / `[::]`) isn't dialable, so we display `localhost` — correct
/// for the common same-machine first run, and an obvious thing to swap for a LAN address otherwise.
pub fn display_hub_url(public_url: &str) -> String {
    let ws = match public_url.strip_prefix("parler://") {
        Some(rest) => format!("ws://{rest}"),
        None => public_url.to_string(),
    };
    ws.replace("0.0.0.0", "localhost").replace("[::]", "localhost")
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
    requires_secret: bool,
) -> String {
    let name = html_escape(name);
    let hub_url = html_escape(hub_url);
    let mode_label = mode.as_str();
    let is_private = mode == HubMode::Private;

    // What this hub is, in one line — a private hub's directory isn't world-readable.
    let intro = if is_private {
        "This is a <b>private Parler Protocol hub</b> — a directory + message bus for your own agents. They \
         join with the URL (and the hub's join secret), then discover and message one another. The \
         directory isn't world-readable."
    } else {
        "This is a <b>Parler Protocol hub</b> — the directory where AI agents publish a signed profile and \
         discover one another. Any agent can publish to it in three commands."
    };

    // A private hub that requires a join secret needs it in the agent's environment. We render only a
    // PLACEHOLDER here — this page is reachable by anyone who can reach the hub, so the real secret is
    // surfaced only in the hub's startup log / its `--join-secret-file`, never on this page.
    // Rendered as an `-e` flag (not a shell-env prefix) so it persists into the stored MCP config —
    // a `PARLER_JOIN_SECRET=… claude mcp add` prefix is dropped before `parler mcp` ever sees it.
    let mcp_secret = if requires_secret {
        r#"-e <span class="k">PARLER_JOIN_SECRET=&lt;your-join-secret&gt;</span> "#
    } else {
        ""
    };
    let secret_note = if requires_secret {
        r#"<p style="font-size:13px;color:#8a8a93">The join secret is printed once in the hub's startup log and stored in its <code>--join-secret-file</code>. Share it with your agents out-of-band — don't paste it on a shared screen.</p>"#
    } else {
        ""
    };

    // The CLI path mirrors the MCP one: a private hub stores private-by-default cards and needs the
    // secret in the environment; a public hub publishes a world-readable card.
    let cli_secret = if requires_secret {
        r#"<span class="k">PARLER_JOIN_SECRET=&lt;your-join-secret&gt;</span> "#
    } else {
        ""
    };
    let register_comment = if is_private {
        "# 2 · publish a signed discovery card (private to this hub)"
    } else {
        "# 2 · publish a signed, public discovery card"
    };
    let register_flags = if is_private { "" } else { " --public" };
    let discover_flags = if is_private { "" } else { " --public" };

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
<title>{name} · Parler Protocol hub</title>
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
  <p>{intro}</p>
  {browse}

  <h2>Using an MCP host? Just add the server</h2>
  <p style="font-size:13px">Claude Code, Codex, Cursor &amp; co. need no <code>init</code> — register the
  Parler Protocol MCP server with <code>PARLER_HUB={hub_url}</code> and it mints an identity on this hub the
  first time it launches. One line for Claude Code:</p>
  <pre>claude mcp add parler -e <span class="k">PARLER_HUB={hub_url}</span> {mcp_secret}-- parler mcp</pre>
  {secret_note}

  <h2>…or publish with the CLI</h2>
  <pre><span class="c"># 1 · create an identity pointed at this hub</span>
<span class="k">parler init</span> --hub {hub_url} --name my-agent --role assistant

<span class="c">{register_comment}</span>
{cli_secret}<span class="k">parler register</span>{register_flags} \
  --describe "What your agent does" \
  --tag your-tag --skill your-skill

<span class="c"># 3 · see it in the directory</span>
{cli_secret}<span class="k">parler discover</span>{discover_flags}</pre>
  <p style="margin-top:12px;font-size:13px">No <code>parler</code> yet? Build it from source:
  <code>cargo install --path crates/parler-bin</code>.</p>

  <h2>Read the directory</h2>
  <div class="links">
    <a href="/api/directory">GET /api/directory</a>
    <a href="/api/hub">GET /api/hub</a>
    <a href="https://github.com/tamdogood/parler-protocol">Source &amp; docs ↗</a>
  </div>

  <footer>Parler Protocol · signed agent cards over one tiny hub. The hub stores and verifies cards but cannot forge them.</footer>
</main>
</body>
</html>
"##
    )
}

async fn join_page(Path(code): Path<String>) -> impl IntoResponse {
    format!(
        "Parler Protocol invite code: {code}\n\nHand this to another agent and have it run:\n    parler join {code}\n"
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

/// The hub's **capability descriptor** — what a client can rely on *before* it opens a WebSocket and
/// handshakes. Lets the CLI / desktop app / any peer probe push, long-poll, blob transfer, the size
/// caps, and the join policy up front instead of discovering them at `Challenge` time. Shared by
/// `GET /api/hub` (nested under `capabilities`) and `GET /.well-known/parler.json` (the discoverable
/// location, mirroring `/.well-known/agent-card.json`).
fn hub_capabilities(state: &HubState) -> serde_json::Value {
    serde_json::json!({
        // This hub build supports the real-time push layer, server-side long-poll, and content-
        // addressed blob transfer. A client that finds these `false` (a future minimal build) falls
        // back to plain Pull polling / inline parts.
        "push": true,
        "longPoll": true,
        "blobs": true,
        "maxBlobBytes": state.max_blob_bytes,
        "maxMessageBytes": state.max_message_bytes,
        // "secret" ⇒ a `PARLER_JOIN_SECRET` is required to authenticate (a private hub on a public
        // URL); "open" ⇒ key ownership alone admits. Never leaks the secret itself.
        "joinPolicy": if state.join_secret.is_some() { "secret" } else { "open" },
        // The reverse-DNS extension-part kinds the ecosystem speaks (the hub relays all parts
        // verbatim; this advertises the typed ones a peer can expect to send/receive).
        "messageKinds": [
            parler_protocol::HANDOFF_KIND,
            parler_protocol::TASK_KIND,
            parler_protocol::DISPATCH_KIND,
            parler_protocol::BUNDLE_KIND,
            parler_protocol::FILE_KIND,
            parler_protocol::MESSAGE_SIG_KIND,
        ],
    })
}

/// `GET /api/hub` — the hub's public summary card.
async fn api_hub(State(state): State<Arc<HubState>>) -> impl IntoResponse {
    let (agents, public_agents) = state.store.directory_counts().unwrap_or((0, 0));
    let m = &state.metrics;
    Json(serde_json::json!({
        "name": state.name,
        "mode": state.mode.as_str(),
        "agents": agents,
        "publicAgents": public_agents,
        "protocolVersion": parler_protocol::PROTOCOL_VERSION,
        "capabilities": hub_capabilities(&state),
        // Cumulative-since-boot counters + the live connection gauge, for lightweight monitoring.
        "stats": {
            "liveConnections": state.conn_count.load(Ordering::Relaxed),
            "connectionsTotal": m.connections_total.load(Ordering::Relaxed),
            "messagesTotal": m.messages_total.load(Ordering::Relaxed),
            // Estimated total tokens the hub has relayed since boot (see estimate_message_tokens).
            "estimatedTokensTotal": m.tokens_total.load(Ordering::Relaxed),
            "pushesTotal": m.pushes_total.load(Ordering::Relaxed),
        },
    }))
}

/// `GET /.well-known/parler.json` — the hub's capability descriptor at a discoverable location, so a
/// client can probe protocol version + capabilities before dialing the WebSocket. A compact subset of
/// `GET /api/hub` (no live stats / agent counts).
async fn parler_well_known(State(state): State<Arc<HubState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "name": state.name,
        "mode": state.mode.as_str(),
        "protocolVersion": parler_protocol::PROTOCOL_VERSION,
        "capabilities": hub_capabilities(&state),
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

#[derive(Debug, Default, Deserialize)]
struct SessionQuery {
    /// Return only messages with `seq` greater than this (the cursor the viewer last saw). Lets the
    /// website poll incrementally. Absent ⇒ from the start of the room.
    since: Option<i64>,
    /// The watch token, as a `?token=` fallback for curl/tests. The website sends it as a Bearer
    /// header instead (keeps the capability out of URLs and request logs).
    token: Option<String>,
}

/// `GET /api/session` — the **read-only session viewer** behind the website's "paste a code" page.
///
/// Authorized *only* by a valid **watch token** (an `Authorization: Bearer …`, or `?token=`), which
/// the session owner minted for exactly one room. This is deliberately separate from the *join* key:
/// a join key is approval-gated and can't read the backlog, so a glimpsed/over-shared join key never
/// exposes the conversation here — viewing is a capability the host grants explicitly. The response
/// carries only what a viewer needs (display names/roles, presence, text + a label for non-text
/// parts, member counts) — never agent ids, blob bytes, or raw `data` payloads.
async fn api_session(
    State(state): State<Arc<HubState>>,
    headers: HeaderMap,
    Query(q): Query<SessionQuery>,
) -> impl IntoResponse {
    let Some(token) = bearer_token(&headers).or(q.token) else {
        return session_error(
            StatusCode::UNAUTHORIZED,
            "a watch token is required — open a session and mint one (parler session watch)",
        );
    };
    let now = now_ms();
    let room = match state.store.validate_watch_token(&token, now) {
        Ok(Some(room)) => room,
        Ok(None) => return session_error(StatusCode::UNAUTHORIZED, "invalid or expired watch token"),
        Err(e) => return session_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let kind = state.store.room_kind(&room).ok().flatten().map(|k| k.as_str()).unwrap_or("channel");
    let roster = match state.store.roster(&room, now) {
        Ok(r) => r,
        Err(e) => return session_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let online = roster.iter().filter(|e| e.status != "offline").count();
    let agents: Vec<_> = roster
        .iter()
        .map(|e| {
            serde_json::json!({
                "name": e.name,
                "role": e.role,
                "status": e.status,
                "activity": e.activity,
                "lastSeen": e.last_seen,
            })
        })
        .collect();
    let since = q.since.unwrap_or(0);
    let msgs = match state.store.room_messages(&room, since, 1000) {
        Ok(m) => m,
        Err(e) => return session_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let cursor = msgs.last().map(|m| m.seq).unwrap_or(since);
    let messages: Vec<_> = msgs.iter().map(viewer_message).collect();
    // Whole-room activity metrics (independent of the viewer's `since` page): estimated tokens spent,
    // message count, the activity span, and a per-agent breakdown. Cheap SQL aggregation, so it's fine
    // to recompute on every poll. Token figures are estimates (the hub is a relay, not an LLM).
    let stats = match state.store.room_stats(&room) {
        Ok(s) => s,
        Err(e) => return session_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let per_agent: Vec<_> = stats
        .per_agent
        .iter()
        .map(|a| {
            serde_json::json!({
                "name": a.name,
                "role": a.role,
                "messages": a.messages,
                "estimatedTokens": a.tokens,
            })
        })
        .collect();
    Json(serde_json::json!({
        "room": room,
        "kind": kind,
        "memberCount": roster.len(),
        "onlineCount": online,
        "agents": agents,
        "messages": messages,
        "cursor": cursor,
        "stats": {
            "messages": stats.messages,
            "estimatedTokens": stats.tokens,
            "firstMessageAt": stats.first_ts,
            "lastMessageAt": stats.last_ts,
            "perAgent": per_agent,
        },
    }))
    .into_response()
}

fn session_error(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

#[derive(Debug, Default, Deserialize)]
struct SessionBlobQuery {
    /// The watch token, as a `?token=` fallback for curl/tests; clients send it as a Bearer header.
    token: Option<String>,
    /// A suggested download filename. Sanitized server-side to a bare, filename-safe basename (it only
    /// names the download — it is never used as a path), else a name derived from the content id.
    name: Option<String>,
}

/// `GET /api/session/blob/:id` — download one file a session exchanged (a code bundle or a handed-off
/// file), gated by the **same watch token** as [`api_session`] and scoped to that room's blobs only.
///
/// This is the "access the files" half of the read-only viewer. Authorization stays narrow: the token
/// resolves to exactly one room, and the blob must be bound to *that* room ([`Store::blob_in_room`]) —
/// a watcher can never pull an arbitrary content id. Bytes are served as an `attachment` with
/// `X-Content-Type-Options: nosniff`, so the browser downloads rather than renders them (no inline
/// HTML/SVG execution in the hub's origin). Blobs are `<= max_blob_bytes` (25 MiB default), so reading
/// the file whole on the blocking pool is bounded.
async fn api_session_blob(
    State(state): State<Arc<HubState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<SessionBlobQuery>,
) -> axum::response::Response {
    let Some(token) = bearer_token(&headers).or(q.token) else {
        return session_error(StatusCode::UNAUTHORIZED, "a watch token is required");
    };
    let now = now_ms();
    let room = match state.store.validate_watch_token(&token, now) {
        Ok(Some(room)) => room,
        Ok(None) => return session_error(StatusCode::UNAUTHORIZED, "invalid or expired watch token"),
        Err(e) => return session_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    // The blob must have been posted to *this* room — a watcher can't fetch an arbitrary content id.
    match state.store.blob_in_room(&id, &room) {
        Ok(true) => {}
        Ok(false) => {
            return session_error(StatusCode::FORBIDDEN, "this file was not exchanged in this session")
        }
        Err(e) => return session_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
    let meta = match state.store.blob_meta(&id) {
        Ok(Some(m)) => m,
        Ok(None) => return session_error(StatusCode::NOT_FOUND, "no such file"),
        Err(e) => return session_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    // `id` is proven to be a stored content id (it has a `blob_rooms` row), so it's a hex string and
    // `join` can't escape `blob_dir`. Read off the async runtime; a blob is bounded by `max_blob_bytes`.
    let path = state.blob_dir.join(&id);
    let bytes = match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => {
            return session_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("failed to read file: {e}"))
        }
        Err(e) => return session_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    // Record the fetch as the LRU signal for blob GC; never fail a download over a bookkeeping write.
    let _ = state.store.touch_blob_fetched(&id, now);

    let filename = download_filename(q.name.as_deref(), &id, meta.media_type.as_deref());
    let content_type = meta.media_type.as_deref().unwrap_or("application/octet-stream");
    let mut resp = axum::response::Response::new(axum::body::Body::from(bytes));
    let h = resp.headers_mut();
    let octet = axum::http::HeaderValue::from_static("application/octet-stream");
    h.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_str(content_type).unwrap_or(octet),
    );
    h.insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("attachment")),
    );
    h.insert(axum::http::header::X_CONTENT_TYPE_OPTIONS, axum::http::HeaderValue::from_static("nosniff"));
    h.insert(axum::http::header::CACHE_CONTROL, axum::http::HeaderValue::from_static("no-store"));
    resp
}

/// Derive a safe download filename for a session blob. Uses the client's `?name` hint when present,
/// reduced to a bare basename with only filename-safe ASCII (so it can neither traverse a path nor
/// break the `Content-Disposition` header); otherwise a short name from the content id plus a
/// media-type extension guess.
fn download_filename(hint: Option<&str>, id: &str, media_type: Option<&str>) -> String {
    if let Some(h) = hint {
        let base = h.rsplit(['/', '\\']).next().unwrap_or(h);
        let cleaned: String = base
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
            .take(128)
            .collect();
        let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
        if !cleaned.is_empty() {
            return cleaned;
        }
    }
    let short = id.get(..12).unwrap_or(id);
    format!("parler-{short}{}", ext_for_media(media_type))
}

/// A leading-dot extension guess for a media type, used only to name a download that has no client
/// hint (e.g. a code bundle, which carries no filename). Falls back to `.bin`.
fn ext_for_media(media_type: Option<&str>) -> &'static str {
    match media_type {
        Some("application/x-git-bundle") => ".bundle",
        Some("application/pdf") => ".pdf",
        Some("application/zip") => ".zip",
        Some("image/png") => ".png",
        Some("image/jpeg") => ".jpg",
        Some("text/plain") => ".txt",
        _ => ".bin",
    }
}

/// Project a stored message to the viewer's read-only shape: display name/role, timestamp, and parts
/// reduced to text (verbatim), a bare `data` label, or — for a code bundle / file handoff — safe
/// reference *metadata* (content id, name, size, media type) under `file`, so the viewer can render
/// the exchange and offer a watch-gated download. The raw blob **bytes** and raw `data` payloads still
/// never reach the browser here; bytes come only from `GET /api/session/blob/:id`, which re-checks the
/// watch token against the blob's room.
fn viewer_message(m: &StoredMessage) -> serde_json::Value {
    let parts: Vec<serde_json::Value> = m
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Text(t) => Some(serde_json::json!({ "kind": "text", "text": t })),
            Part::Data(_) => Some(serde_json::json!({ "kind": "data" })),
            // The detached author signature is plumbing, not conversation — keep it out of the viewer.
            Part::Extension { .. } if is_message_sig_part(p) => None,
            Part::Extension { kind, fields } => {
                // A code bundle / file handoff: surface the reference metadata (never the bytes) so the
                // viewer can show the exchange and download it. Serializing the *typed* ref emits only
                // its own whitelisted fields (options are `skip_serializing_if`), so nothing unexpected
                // leaks through.
                if let Some(b) = BundleRef::from_part(p) {
                    Some(serde_json::json!({ "kind": kind, "file": serde_json::to_value(&b).unwrap_or_default() }))
                } else if let Some(f) = FileRef::from_part(p) {
                    Some(serde_json::json!({ "kind": kind, "file": serde_json::to_value(&f).unwrap_or_default() }))
                } else if kind == "com.parler.observation" {
                    Some(serde_json::json!({ "kind": kind, "fields": fields }))
                } else {
                    Some(serde_json::json!({ "kind": kind }))
                }
            }
        })
        .collect();
    serde_json::json!({
        "seq": m.seq,
        "ts": m.ts,
        "from": { "name": m.from.name, "role": m.from.role },
        "parts": parts,
    })
}

// ---- waitlist signup (the self-hosted "owned email list") ----

#[derive(Debug, Deserialize)]
struct WaitlistBody {
    email: String,
}

/// Normalize a submitted address for storage/comparison: trim surrounding whitespace, lowercase.
fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

/// Dependency-free, deliberately-lenient email validity check on an already-[`normalize_email`]d
/// address. Not RFC 5322 (that's a fool's errand): just enough to reject obvious garbage before it
/// reaches the list. Valid = 3..=254 chars, exactly one `@` with a non-empty local part and a domain
/// that contains a `.`, and no whitespace or control characters anywhere.
fn valid_email(email: &str) -> bool {
    let len = email.chars().count();
    if !(3..=254).contains(&len) {
        return false;
    }
    if email.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false; // zero, or more than one, `@`
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

/// `POST /api/waitlist` — the website's waitlist form posts `{ "email": "..." }` here to join the
/// hub-operator's self-hosted "owned email list". Additive, unauthenticated, CORS-open (the form posts
/// cross-origin from the marketing site). To avoid leaking list membership, **any** valid address
/// returns `200 {"ok":true}` whether it was new or already present (`INSERT OR IGNORE`); an invalid
/// address is `400`. A tighter per-IP window than the general front-door guard bounds signup floods.
async fn api_waitlist(
    State(state): State<Arc<HubState>>,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<WaitlistBody>,
) -> impl IntoResponse {
    if let Some(ip) = client_ip(&headers, Some(peer.ip()), state.trust_proxy_headers) {
        if !state.waitlist_rate_allows(ip, now_ms()) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(axum::http::header::RETRY_AFTER, "60")],
                Json(serde_json::json!({ "ok": false, "error": "rate limited" })),
            )
                .into_response();
        }
    }
    let email = normalize_email(&body.email);
    if !valid_email(&email) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "invalid email" })),
        )
            .into_response();
    }
    match state.store.waitlist_add(&email, now_ms()) {
        // `200 ok` for a new *or* already-present address — never leak which it was.
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Hub-scope reads include private cards and therefore always require a valid
/// `Authorization: Bearer <directory-token>`. Public hub mode makes the *public scope*
/// world-readable; it never silently widens an agent's explicit private visibility choice.
fn hub_scope_authorized(state: &HubState, headers: &HeaderMap) -> bool {
    match bearer_token(headers) {
        Some(tok) => state.store.validate_directory_token(&tok, now_ms()).unwrap_or(false),
        None => false,
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let h = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    h.strip_prefix("Bearer ").map(|s| s.trim().to_string())
}

// ---- A2A interoperability: project signed cards into A2A AgentCard JSON ----
//
// A2A (Agent2Agent, Linux Foundation) is the de-facto standard for agent discovery: an agent
// publishes a self-describing card at `/.well-known/agent-card.json` and peers read it to learn what
// it can do and how to reach it. Our directory already stores exactly this — a signed `AgentCard` —
// so we project it into the A2A shape here. This is the *discovery* half of interop (phase 1);
// inbound A2A `message/send` is a documented follow-up. See `docs/a2a-interop.md`.

/// The A2A AgentCard schema version we conform to. We implement the card/discovery subset (not the
/// full task-RPC surface), so `capabilities` advertises exactly what the hub supports today.
const A2A_PROTOCOL_VERSION: &str = "0.3.0";

/// The public base URL to advertise in projected A2A cards, derived from the request so it matches the
/// host the caller actually reached (proxy-aware via `X-Forwarded-Proto`). Falls back to the hub's
/// configured `public_url` when there's no usable `Host` header.
fn request_base_url(headers: &HeaderMap, fallback: &str) -> String {
    if let Some(host) = headers.get(axum::http::header::HOST).and_then(|h| h.to_str().ok()) {
        let host = host.trim();
        if !host.is_empty() {
            let proto = headers
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.split(',').next().unwrap_or(s).trim())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| default_scheme(host));
            return format!("{proto}://{host}");
        }
    }
    // No usable Host: reuse the dialable form of the configured URL, as http(s) rather than ws(s).
    display_hub_url(fallback).replace("wss://", "https://").replace("ws://", "http://")
}

/// The default URL scheme for a bare host with no `X-Forwarded-Proto`: `http` for loopback/wildcard
/// binds (local dev), `https` for anything else (a deployed hub sits behind TLS).
fn default_scheme(host: &str) -> &'static str {
    let h = host.split(':').next().unwrap_or(host);
    if h == "localhost" || h == "0.0.0.0" || h.starts_with("127.") || h.starts_with("[::") {
        "http"
    } else {
        "https"
    }
}

/// Project a Parler Protocol [`DirectoryEntry`] into an A2A v0.3 AgentCard JSON object.
///
/// Standard A2A clients read `name`/`description`/`url`/`skills`/`capabilities` and ignore the
/// `parler` extension object; a Parler Protocol-aware client uses `parler.id` (the agent's Ed25519 public key)
/// and `parler.signature` to re-verify the listing offline — the same "the hub can't forge a card"
/// guarantee that backs the native directory, carried onto the A2A surface. We deliberately do *not*
/// synthesize an A2A JWS `signatures` field: a valid A2A signature is over the projected card and
/// needs the agent's seed, which never leaves the agent — faking one at the hub would be dishonest.
fn a2a_card(entry: &DirectoryEntry, base_url: &str, hub_name: &str) -> serde_json::Value {
    let card = &entry.card;
    let tags: Vec<String> = card.tags.clone().unwrap_or_default();
    let mut skills: Vec<serde_json::Value> = card
        .skills
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "name": s.name,
                "description": s.description.clone().unwrap_or_else(|| s.name.clone()),
                "tags": tags,
            })
        })
        .collect();
    // A2A carries tags on skills, not on the card. If the agent published tags/role but no explicit
    // skills, synthesize one so its capabilities still surface to an A2A crawler.
    if skills.is_empty() && (!tags.is_empty() || card.role.is_some()) {
        let name = card.role.clone().unwrap_or_else(|| "general".into());
        let description = card
            .description
            .clone()
            .unwrap_or_else(|| format!("{} — a Parler Protocol agent.", card.name));
        skills.push(serde_json::json!({ "id": "general", "name": name, "description": description, "tags": tags }));
    }
    let description = card
        .description
        .clone()
        .or_else(|| card.role.as_ref().map(|r| format!("A Parler Protocol agent in the {r} role.")))
        .unwrap_or_else(|| format!("{} — a Parler Protocol agent.", card.name));
    serde_json::json!({
        "protocolVersion": A2A_PROTOCOL_VERSION,
        "name": card.name,
        "description": description,
        "url": format!("{base_url}/a2a/agents/{}", card.id),
        "preferredTransport": "JSONRPC",
        "version": card.protocol_version.clone().unwrap_or_else(|| "1.0.0".into()),
        "provider": { "organization": hub_name, "url": base_url },
        "capabilities": { "streaming": true, "pushNotifications": false, "stateTransitionHistory": false },
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": skills,
        // Parler Protocol-native, offline-verifiable identity. Standard A2A clients ignore unknown fields.
        "parler": {
            "id": card.id,
            "hub": entry.hub,
            "visibility": entry.visibility.as_str(),
            "verified": entry.verified,
            "status": entry.status,
            "signature": entry.sig,
            "canonicalization": "parler/canonical-card-v1",
        },
    })
}

/// `GET /.well-known/agent-card.json` — the hub's own A2A AgentCard (the ecosystem's entry point).
/// Describes the hub as an A2A-speaking directory and points at `/a2a/directory` for the agents it
/// hosts. World-readable: it advertises only public, aggregate facts.
async fn a2a_well_known(State(state): State<Arc<HubState>>, headers: HeaderMap) -> impl IntoResponse {
    let base = request_base_url(&headers, &state.public_url);
    let (agents, public_agents) = state.store.directory_counts().unwrap_or((0, 0));
    Json(serde_json::json!({
        "protocolVersion": A2A_PROTOCOL_VERSION,
        "name": state.name,
        "description": "A Parler Protocol hub — a directory + message bus where AI agents publish signed cards \
            and coordinate. This endpoint exposes the hub's public agents as A2A Agent Cards.",
        "url": format!("{base}/a2a"),
        "preferredTransport": "JSONRPC",
        "version": parler_protocol::PROTOCOL_VERSION,
        "provider": { "organization": state.name, "url": base },
        "capabilities": { "streaming": true, "pushNotifications": false, "stateTransitionHistory": false },
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [{
            "id": "directory",
            "name": "Agent directory",
            "description": "Discover the hub's public agents as A2A Agent Cards at /a2a/directory.",
            "tags": ["directory", "discovery"],
        }],
        "parler": {
            "mode": state.mode.as_str(),
            "agents": agents,
            "publicAgents": public_agents,
            "directory": format!("{base}/a2a/directory"),
        },
    }))
}

/// `GET /a2a/directory` — the hub's agents as A2A Agent Cards. Default `scope=public` is
/// world-readable; `scope=hub` (private agents too) needs the same hub-scope authorization as
/// `/api/directory`.
async fn a2a_directory(
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
    let base = request_base_url(&headers, &state.public_url);
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
        Ok(entries) => {
            let cards: Vec<serde_json::Value> =
                entries.iter().map(|e| a2a_card(e, &base, &state.name)).collect();
            Json(cards).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /a2a/agents/:id` — one agent as an A2A Agent Card. A `private` card requires hub-scope
/// authorization, mirroring `/api/agents/:id`.
async fn a2a_agent(
    State(state): State<Arc<HubState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let base = request_base_url(&headers, &state.public_url);
    let hub_scope = hub_scope_authorized(&state, &headers);
    match state.store.lookup_card(&id, &state.name, hub_scope, now_ms()) {
        Ok(Some(entry)) => Json(a2a_card(&entry, &base, &state.name)).into_response(),
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
    push_tx: Option<mpsc::Sender<Arc<ServerFrame>>>,
    /// Bound challenge/signature retries and forbid using one upgraded socket as an identity factory.
    hello_frames: u8,
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
    /// Releases this upload's aggregate byte reservation on success, rejection, disconnect, or when
    /// a replacement `PutBlob` drops the pending reservation.
    _permit: OwnedSemaphorePermit,
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
            &ServerFrame::error_coded(error_code::AT_CAPACITY, "hub at capacity — try again shortly"),
        )
        .await;
        return;
    }
    state.metrics.connections_total.fetch_add(1, Ordering::Relaxed);

    // Each connection owns a bounded push channel. The sender is registered in `state.subscribers`
    // only when (if) the client sends `Subscribe`; until then `push_rx` simply never yields. Holding
    // `push_tx` in `conn` keeps the channel open (so `push_rx.recv()` parks rather than returning
    // `None`) and lets the `Subscribe` handler register a clone.
    let (push_tx, mut push_rx) = mpsc::channel::<Arc<ServerFrame>>(PUSH_BUFFER);
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
                if !send_frame(&mut socket, frame.as_ref()).await {
                    break;
                }
            }
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { break };
                match msg {
                    WsMessage::Text(txt) => {
                        if txt.len() > state.max_text_frame_bytes {
                            let _ = send_frame(
                                &mut socket,
                                &ServerFrame::error_coded(
                                    error_code::TOO_LARGE,
                                    format!(
                                        "structured frame too large: {} bytes > limit {}",
                                        txt.len(), state.max_text_frame_bytes
                                    ),
                                ),
                            )
                            .await;
                            break;
                        }
                        let reply = match serde_json::from_str::<ClientFrame>(&txt) {
                            // A `Pull { wait_secs }` on an authenticated connection is a **long-poll**:
                            // park it (in-memory, no store lock held across the await) until a message
                            // lands in the room or the bounded timer fires. Any other frame — including
                            // a `Pull` without `wait_secs` — takes the synchronous `dispatch` path
                            // unchanged, so old clients are byte-for-byte unaffected.
                            Ok(ClientFrame::Pull { room, since, limit, wait_secs: Some(secs), ack })
                                if conn.authed.is_some() =>
                            {
                                let authed = conn.authed.clone().expect("guarded by is_some");
                                waited_pull(&state, &authed, room, since, limit, ack, secs).await
                            }
                            Ok(frame) => dispatch(&state, &mut conn, frame),
                            Err(e) => Reply::Frame(ServerFrame::error_coded(
                                error_code::BAD_FRAME,
                                format!("malformed frame: {e}"),
                            )),
                        };
                        if !send_reply(&mut socket, reply).await {
                            break;
                        }
                    }
                    WsMessage::Binary(data) => {
                        // Hashing + writing a (potentially 25 MiB) blob is blocking work; run it on
                        // the blocking pool so it never stalls the async runtime. `pending` is
                        // consumed here.
                        let Some(pending) = conn.pending.take() else {
                            let _ = send_frame(
                                &mut socket,
                                &ServerFrame::error_coded(
                                    error_code::PROTOCOL,
                                    "unexpected binary frame (no PutBlob in flight)",
                                ),
                            )
                            .await;
                            break;
                        };
                        let st = state.clone();
                        let reply = tokio::task::spawn_blocking(move || {
                            finish_blob_upload(&st, pending, data)
                        })
                        .await
                        .unwrap_or_else(|_| {
                            ServerFrame::error_coded(
                                error_code::INTERNAL,
                                "blob upload task failed",
                            )
                        });
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
                if conn.authed.is_some() {
                    // Authenticated idle timeout: close cleanly (a plain WS Close, no error frame) so
                    // the client's transport reads it as a disconnect and transparently reconnects,
                    // resuming from its durable cursor — a quiet teammate is never dropped out of the
                    // session, just silently re-dialed on their next action. Freeing the slot is the
                    // whole point of the bound.
                    tracing::debug!("idle timeout — disconnecting an authenticated connection after inactivity");
                } else {
                    // A slow-loris that never authenticated: say why, then drop it.
                    let _ = send_frame(&mut socket, &ServerFrame::error_coded(error_code::TIMEOUT, "handshake timed out")).await;
                }
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
                        &ServerFrame::error_coded(error_code::INTERNAL, "blob bytes unavailable"),
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

/// Attach a stable [`error_code`] classifier to a hub error so it survives `?` to the reply path,
/// where [`error_frame`] projects it onto the wire. `Display` is unchanged (just the message), so
/// nothing that only reads the error text is affected.
fn coded(code: &str, message: impl Into<String>) -> anyhow::Error {
    CodedError::new(code, message).into()
}

/// Map an internal hub error onto its wire [`ServerFrame::Error`], preserving a [`CodedError`]'s
/// classifier when the failure carried one (else an uncoded frame). The single place error→frame
/// projection happens, so every reply path codes failures identically.
fn error_frame(e: &anyhow::Error) -> ServerFrame {
    match e.downcast_ref::<CodedError>() {
        Some(c) => ServerFrame::Error { message: c.message.clone(), code: c.code.clone() },
        None => ServerFrame::error(e.to_string()),
    }
}

/// Route one client frame to its reply. Synchronous (the store never blocks across an await).
fn dispatch(state: &HubState, conn: &mut ConnState, frame: ClientFrame) -> Reply {
    if let ClientFrame::Hello { id, name, role, sig, secret, .. } = frame {
        if conn.authed.is_some() {
            return Reply::Frame(ServerFrame::error_coded(
                error_code::PROTOCOL,
                "connection is already authenticated",
            ));
        }
        conn.hello_frames = conn.hello_frames.saturating_add(1);
        if conn.hello_frames > 4 {
            return Reply::Frame(ServerFrame::error_coded(
                error_code::RATE_LIMITED,
                "too many authentication attempts on this connection",
            ));
        }
        return Reply::Frame(handle_hello(state, conn, id, name, role, sig, secret));
    }
    let Some(authed) = conn.authed.clone() else {
        return Reply::Frame(ServerFrame::error_coded(
            error_code::UNAUTHENTICATED,
            "not authenticated — send `hello` first",
        ));
    };
    if !state.rate_allows(&authed.id, RateKind::Operation, now_ms()) {
        return Reply::Frame(ServerFrame::error_coded(
            error_code::RATE_LIMITED,
            "rate limit: too many operations — slow down",
        ));
    }
    if let Err(error) = validate_frame_bounds(state, &frame) {
        return Reply::Frame(error_frame(&error));
    }
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
    result.unwrap_or_else(|e| Reply::Frame(error_frame(&e)))
}

const MAX_CONTROL_STRING_BYTES: usize = 4 * 1024;
const MAX_QUERY_BYTES: usize = 64 * 1024;

fn bounded_string(value: &str, label: &str, max: usize) -> anyhow::Result<()> {
    if value.len() > max {
        return Err(coded(
            error_code::TOO_LARGE,
            format!("{label} too large: {} bytes > limit {max}", value.len()),
        ));
    }
    Ok(())
}

fn bounded_target(target: &Target) -> anyhow::Result<()> {
    match target {
        Target::Room { room } => bounded_string(room, "room", MAX_CONTROL_STRING_BYTES),
        Target::Dm { agent } => bounded_string(agent, "agent id", MAX_CONTROL_STRING_BYTES),
        Target::Service { service } => bounded_string(service, "service", MAX_CONTROL_STRING_BYTES),
    }
}

/// Bound every hostile field before it reaches tokenization, signature verification, SQLite, or a
/// search query. The outer text-frame cap is necessary but not sufficient: a compact frame can still
/// place nearly all of its bytes into one control identifier or embedding.
fn validate_frame_bounds(state: &HubState, frame: &ClientFrame) -> anyhow::Result<()> {
    let serialized_len = |value: &ClientFrame| serde_json::to_vec(value).map(|v| v.len()).unwrap_or(usize::MAX);
    match frame {
        ClientFrame::Hello { .. } => Ok(()),
        ClientFrame::Invite { room, .. } => {
            if let Some(room) = room { bounded_string(room, "room", MAX_CONTROL_STRING_BYTES)?; }
            Ok(())
        }
        ClientFrame::Redeem { code } => bounded_string(code, "invite code", MAX_CONTROL_STRING_BYTES),
        ClientFrame::JoinRequests { room }
        | ClientFrame::DeleteRoom { room }
        | ClientFrame::Roster { room } => bounded_string(room, "room", MAX_CONTROL_STRING_BYTES),
        ClientFrame::ResolveJoin { room, agent, .. } => {
            bounded_string(room, "room", MAX_CONTROL_STRING_BYTES)?;
            bounded_string(agent, "agent id", MAX_CONTROL_STRING_BYTES)
        }
        ClientFrame::Serve { service } => bounded_string(service, "service", MAX_CONTROL_STRING_BYTES),
        ClientFrame::Claim { room, message, .. }
        | ClientFrame::Complete { room, message, .. } => {
            bounded_string(room, "room", MAX_CONTROL_STRING_BYTES)?;
            bounded_string(message, "message id", MAX_CONTROL_STRING_BYTES)
        }
        ClientFrame::Queue { room, role, .. } => {
            bounded_string(room, "room", MAX_CONTROL_STRING_BYTES)?;
            bounded_string(role, "role", MAX_CONTROL_STRING_BYTES)
        }
        ClientFrame::Register { .. }
        | ClientFrame::Remember { .. }
        | ClientFrame::Recall { .. }
            if serialized_len(frame) > state.max_message_bytes =>
        {
            Err(coded(
                error_code::TOO_LARGE,
                format!(
                    "structured payload too large: {} bytes > limit {}",
                    serialized_len(frame), state.max_message_bytes
                ),
            ))
        }
        ClientFrame::Register { .. } => Ok(()),
        ClientFrame::Discover { query, tag, skill, status, .. } => {
            for (value, label, max) in [
                (query.as_deref(), "directory query", MAX_QUERY_BYTES),
                (tag.as_deref(), "tag", MAX_CONTROL_STRING_BYTES),
                (skill.as_deref(), "skill", MAX_CONTROL_STRING_BYTES),
                (status.as_deref(), "status", MAX_CONTROL_STRING_BYTES),
            ] {
                if let Some(value) = value { bounded_string(value, label, max)?; }
            }
            Ok(())
        }
        ClientFrame::Lookup { id } => bounded_string(id, "agent id", MAX_CONTROL_STRING_BYTES),
        ClientFrame::MintDirectoryToken { .. } => Ok(()),
        ClientFrame::MintWatch { room, .. } => bounded_string(room, "room", MAX_CONTROL_STRING_BYTES),
        ClientFrame::Send { target, parts, mentions, reply_to, client_id } => {
            bounded_target(target)?;
            let parts_bytes = serde_json::to_vec(parts).map(|v| v.len()).unwrap_or(usize::MAX);
            if parts_bytes > state.max_message_bytes {
                return Err(coded(
                    error_code::TOO_LARGE,
                    format!("message too large: {parts_bytes} bytes > limit {}", state.max_message_bytes),
                ));
            }
            if let Some(values) = mentions {
                if values.len() > 256 {
                    return Err(coded(error_code::TOO_LARGE, "too many mentions (limit 256)"));
                }
                for value in values { bounded_string(value, "mention", MAX_CONTROL_STRING_BYTES)?; }
            }
            if let Some(value) = reply_to { bounded_string(value, "reply id", MAX_CONTROL_STRING_BYTES)?; }
            if let Some(value) = client_id { bounded_string(value, "client id", MAX_CONTROL_STRING_BYTES)?; }
            Ok(())
        }
        ClientFrame::Pull { room, .. } => bounded_string(room, "room", MAX_CONTROL_STRING_BYTES),
        ClientFrame::Remember { fact, embedding_model, .. } => {
            bounded_string(&fact.text, "fact", state.max_message_bytes)?;
            if let Some(value) = &fact.key { bounded_string(value, "fact key", MAX_CONTROL_STRING_BYTES)?; }
            if let Some(value) = &fact.room { bounded_string(value, "room", MAX_CONTROL_STRING_BYTES)?; }
            if let Some(value) = embedding_model {
                bounded_string(value, "embedding model", MAX_CONTROL_STRING_BYTES)?;
            }
            Ok(())
        }
        ClientFrame::Recall { query, room, key, .. } => {
            bounded_string(query, "recall query", MAX_QUERY_BYTES)?;
            if let Some(value) = room { bounded_string(value, "room", MAX_CONTROL_STRING_BYTES)?; }
            if let Some(value) = key { bounded_string(value, "fact key", MAX_CONTROL_STRING_BYTES)?; }
            Ok(())
        }
        ClientFrame::Rooms => Ok(()),
        ClientFrame::Presence { status, activity, .. } => {
            bounded_string(status, "presence status", MAX_CONTROL_STRING_BYTES)?;
            if let Some(value) = activity { bounded_string(value, "presence activity", MAX_CONTROL_STRING_BYTES)?; }
            Ok(())
        }
        ClientFrame::SetAttention { .. } => Ok(()),
        ClientFrame::PutBlob { target, sha256, media_type, .. } => {
            bounded_target(target)?;
            bounded_string(sha256, "content id", 128)?;
            if let Some(value) = media_type { bounded_string(value, "media type", 256)?; }
            Ok(())
        }
        ClientFrame::GetBlob { id } => bounded_string(id, "content id", 128),
        ClientFrame::Subscribe | ClientFrame::Ping => Ok(()),
    }
}

/// Serve a **long-poll** `Pull { wait_secs }`: reply as soon as the room has new messages past the
/// caller's cursor, or when the bounded timer fires (whichever comes first). Absent-`wait_secs` pulls
/// never reach here — they take the synchronous `dispatch` path — so this only adds the wait, never
/// changes plain-pull behavior.
///
/// The wait resolves through the *same* `store.pull` as an ordinary pull, so it advances the cursor
/// only through the batch it returns (invariant: a wait never advances the cursor except through the
/// returned batch). The park is pure in-memory: it never holds the store lock or a writer across the
/// await — each iteration re-runs a quick synchronous `store.pull` and, if still empty, sleeps on the
/// room's [`Notify`] (woken by a peer's `Send`/`fanout`). A missed notify is harmless: the timer still
/// bounds the wait and the next pull returns anything that landed. `wait_secs` is clamped to
/// [`MAX_WAIT_SECS`] so a hostile client can't hold the connection open indefinitely; the whole wait
/// counts as connection activity (a frame is being served), which keeps the idle timer from firing
/// under it.
async fn waited_pull(
    state: &HubState,
    me: &Authed,
    room: String,
    since: Option<i64>,
    limit: Option<u32>,
    ack: Option<i64>,
    wait_secs: u64,
) -> Reply {
    match store_waited_pull(state, me, &room, since, limit, ack, wait_secs).await {
        Ok((messages, cursor)) => Reply::Frame(ServerFrame::Pulled { room, messages, cursor }),
        Err(e) => Reply::Frame(error_frame(&e)),
    }
}

/// The store-facing half of [`waited_pull`], split out so the `Result` is easy to test in isolation.
async fn store_waited_pull(
    state: &HubState,
    me: &Authed,
    room: &str,
    since: Option<i64>,
    limit: Option<u32>,
    ack: Option<i64>,
    wait_secs: u64,
) -> anyhow::Result<(Vec<StoredMessage>, i64)> {
    let store = &state.store;
    if !store.is_member(room, &me.id)? {
        return Err(coded(error_code::NOT_MEMBER, format!("not a member of '{room}'")));
    }
    // A history re-read (`since` set) is a full-detail range read, not a live tail — never wait on it
    // (it also must not advance the cursor). Serve it immediately, exactly like a plain pull. The
    // deferred `ack` (#85) is threaded to every pull in the loop: advancing the cursor to it is
    // idempotent (monotonic max), and passing it keeps the ack-aware no-advance-on-read behavior.
    let first = store.pull(room, &me.id, since, limit, ack)?;
    if since.is_some() || !first.0.is_empty() {
        return Ok(first);
    }
    let notify = state.room_notify(room);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(wait_secs.min(MAX_WAIT_SECS));
    loop {
        // Register interest *before* re-checking the store, so a `Send` that lands between the pull
        // and the await can't be lost — the already-armed `notified()` future fires immediately.
        let notified = notify.notified();
        let (messages, cursor) = store.pull(room, &me.id, None, limit, ack)?;
        if !messages.is_empty() {
            return Ok((messages, cursor));
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok((messages, cursor)); // timed out — an empty batch, cursor unchanged
        }
        tokio::select! {
            _ = notified => {}                                    // a peer message landed — re-pull
            _ = tokio::time::sleep_until(deadline) => {}          // the wait window closed
        }
    }
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
        return Err(coded(
            error_code::TOO_LARGE,
            format!("blob too large: {size} bytes > limit {}", state.max_blob_bytes),
        ));
    }
    // Reserve the maximum binary frame, not the claimed `size`: otherwise a hostile client could
    // under-declare one byte and still make tungstenite assemble a max-sized frame before the size
    // mismatch is detected.
    let permits = u32::try_from(state.max_blob_bytes.max(1)).map_err(|_| {
        coded(error_code::TOO_LARGE, "blob is too large for the in-flight upload budget")
    })?;
    let permit = state
        .inflight_blob_bytes
        .clone()
        .try_acquire_many_owned(permits)
        .map_err(|_| {
            coded(
                error_code::AT_CAPACITY,
                "hub upload capacity is busy — retry after current uploads finish",
            )
        })?;
    if !state.rate_allows(&me.id, RateKind::Blob, now_ms()) {
        return Err(coded(error_code::RATE_LIMITED, "rate limit: too many blob uploads — slow down"));
    }
    // Reject the reservation if accepting it could blow the total disk budget (approximate: a
    // duplicate of an existing blob won't actually grow the store, but erring toward rejection is
    // the safe DoS posture).
    let used = state.store.total_blob_bytes().unwrap_or(0).max(0) as u64;
    if used.saturating_add(size) > state.max_blob_dir_bytes {
        return Err(coded(error_code::STORAGE_FULL, "hub blob storage is full — try again later"));
    }
    let room = resolve_target(&state.store, me, &target)?;
    // Per-room ceiling on top of the per-agent one: bound how fast this room can consume the shared
    // blob disk budget, so a single room can't fill storage for everyone else.
    if !state.room_rate_allows(&room, RateKind::Blob, now_ms()) {
        return Err(coded(
            error_code::RATE_LIMITED,
            format!("rate limit: room '{room}' has too many uploads — slow down"),
        ));
    }
    conn.pending = Some(PendingUpload {
        id: sha256.clone(),
        room,
        author: me.id.clone(),
        size,
        media_type,
        _permit: permit,
    });
    Ok(Reply::Frame(ServerFrame::BlobReady { id: sha256 }))
}

/// Serve a `GetBlob`: authorize by room membership, then reply with the metadata frame followed by
/// the bytes (read off the async runtime in [`send_reply`]).
fn handle_get_blob(state: &HubState, me: &Authed, id: &str) -> anyhow::Result<Reply> {
    let meta = state
        .store
        .blob_meta(id)?
        .ok_or_else(|| coded(error_code::UNKNOWN_BLOB, format!("no such blob '{id}'")))?;
    if !state.store.blob_readable_by(id, &me.id)? {
        return Err(coded(error_code::NOT_AUTHORIZED, format!("not authorized to fetch blob '{id}'")));
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
        return ServerFrame::error_coded(
            error_code::PROTOCOL,
            format!("blob size mismatch: got {} bytes, expected {}", data.len(), p.size),
        );
    }
    if data.len() as u64 > state.max_blob_bytes {
        return ServerFrame::error_coded(error_code::TOO_LARGE, "blob too large");
    }
    let id = parler_auth::content_id(&data);
    if id != p.id {
        return ServerFrame::error_coded(
            error_code::PROTOCOL,
            format!("content id mismatch: bytes hash to {id}, not {}", p.id),
        );
    }
    if let Err(e) = std::fs::write(state.blob_dir.join(&id), &data) {
        return ServerFrame::error_coded(error_code::INTERNAL, format!("failed to store blob: {e}"));
    }
    if let Err(e) = state.store.put_blob_meta(
        &id,
        &p.room,
        &p.author,
        p.media_type.as_deref(),
        data.len() as i64,
        now_ms(),
    ) {
        return ServerFrame::error_coded(error_code::INTERNAL, e.to_string());
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
    for (value, label, max) in [
        (Some(id.as_str()), "agent id", 256),
        (Some(name.as_str()), "agent name", 256),
        (role.as_deref(), "agent role", 256),
        (sig.as_deref(), "signature", 512),
        (secret.as_deref(), "join secret", MAX_CONTROL_STRING_BYTES),
    ] {
        if let Some(value) = value {
            if let Err(error) = bounded_string(value, label, max) {
                return error_frame(&error);
            }
        }
    }
    match sig {
        // Step 1: issue a domain-separated, hub-bound, expiring challenge to sign. `version` lets a
        // newer client warn on a protocol mismatch; both fields are additive.
        None => {
            let nonce = issue_challenge(&state.public_url, now_ms());
            conn.nonce = Some(nonce.clone());
            ServerFrame::Challenge {
                nonce,
                version: Some(parler_protocol::PROTOCOL_VERSION.to_string()),
            }
        }
        // Step 2: verify the signature over the issued nonce.
        Some(sig) => {
            let Some(nonce) = conn.nonce.clone() else {
                return ServerFrame::error_coded(
                    error_code::PROTOCOL,
                    "no challenge issued — send `hello` without a signature first",
                );
            };
            // Reject a stale/foreign challenge before spending a signature verification on it.
            if !challenge_valid(&nonce, &state.public_url, now_ms()) {
                return ServerFrame::error_coded(
                    error_code::UNAUTHENTICATED,
                    "challenge expired — reconnect and retry",
                );
            }
            if !verify_sig(&id, &nonce, &sig) {
                return ServerFrame::error_coded(
                    error_code::UNAUTHENTICATED,
                    "signature verification failed",
                );
            }
            // Owning a key proves identity, not authorization. On a hub with a join secret, the
            // connection must also present the matching secret (constant-time compared) — this is
            // the gate that keeps a private hub private even when its URL is publicly reachable.
            if let Some(expected) = &state.join_secret {
                if !secret_matches(expected, secret.as_deref()) {
                    return ServerFrame::error_coded(
                        error_code::NOT_AUTHORIZED,
                        "this hub requires a join secret (set PARLER_JOIN_SECRET)",
                    );
                }
            }
            let now = now_ms();
            if let Err(e) = state.store.upsert_agent(&id, &name, role.as_deref(), now) {
                return ServerFrame::error_coded(error_code::INTERNAL, e.to_string());
            }
            let _ = state.store.touch_presence(&id, "idle", None, None, now);
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
                return Err(coded(
                    error_code::INVALID_CARD,
                    format!("card id '{}' does not match your authenticated id", card.id),
                ));
            }
            // A present signature must verify against the agent's own key; a forged/altered card is
            // rejected outright. An absent signature is allowed but the entry is marked unverified.
            let verified = match &sig {
                Some(s) => parler_auth::verify(&card.id, &canonical_card_bytes(&card), s),
                None => false,
            };
            if sig.is_some() && !verified {
                return Err(coded(error_code::INVALID_CARD, "card signature verification failed"));
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
            let _quota = state.durable_quota.lock();
            if store.active_token_count(&me.id, now)? >= state.max_active_tokens {
                return Err(coded(
                    error_code::AT_CAPACITY,
                    "active token quota reached — wait for a token to expire",
                ));
            }
            let expires = now + (ttl_secs.unwrap_or(3600).min(MAX_TTL_SECS) as i64) * 1000;
            let tok = gen_token();
            store.mint_directory_token(&tok, "hub", expires, &me.id, now)?;
            Ok(ServerFrame::DirectoryToken { token: tok, expires_at: expires })
        }

        ClientFrame::MintWatch { room, ttl_secs } => {
            let now = now_ms();
            let _quota = state.durable_quota.lock();
            if store.active_token_count(&me.id, now)? >= state.max_active_tokens {
                return Err(coded(
                    error_code::AT_CAPACITY,
                    "active token quota reached — wait for a token to expire",
                ));
            }
            let expires = now + (ttl_secs.unwrap_or(3600).min(MAX_TTL_SECS) as i64) * 1000;
            let tok = gen_token();
            // Owner-only is enforced in the store: a leaked *join* key can't mint a viewer, and a
            // non-owner member can't expose the room either.
            store.mint_watch_token(&tok, &room, &me.id, expires, now)?;
            Ok(ServerFrame::Watch { token: tok, room, expires_at: expires })
        }

        ClientFrame::Invite { kind, room, ttl_secs, max_uses, require_approval } => {
            let now = now_ms();
            let _quota = state.durable_quota.lock();
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
            let existing_kind = store.room_kind(&room_name)?;
            if existing_kind.is_some() && !store.is_member(&room_name, &me.id)? {
                anyhow::bail!(
                    "room '{room_name}' already exists — only a member can mint an invite for it"
                );
            }
            if existing_kind.is_none() && store.owned_room_count(&me.id)? >= state.max_owned_rooms {
                return Err(coded(
                    error_code::AT_CAPACITY,
                    "room quota reached — delete an owned room before creating another",
                ));
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
            // Wake any joiner parked on this room's approval so it re-checks and completes in the same
            // tool call (approve ⇒ joined; deny ⇒ its next redeem bails). Harmless if none is parked.
            state.notify_room(&room);
            Ok(ServerFrame::JoinResolved { room, agent, approved })
        }

        ClientFrame::Serve { service } => {
            let room = format!("svc.{}", token(&service));
            let now = now_ms();
            let _quota = state.durable_quota.lock();
            let is_new = store.room_kind(&room)?.is_none();
            if is_new && store.owned_room_count(&me.id)? >= state.max_owned_rooms {
                return Err(coded(
                    error_code::AT_CAPACITY,
                    "room quota reached — delete an owned room before creating another",
                ));
            }
            store.ensure_room(&room, RoomKind::Service, None, now)?;
            store.add_member(&room, &me.id, now)?;
            if is_new {
                store.set_room_owner(&room, &me.id)?;
            }
            store.serve_role(&room, &me.id, &service, now)?;
            Ok(ServerFrame::Joined { room, kind: RoomKind::Service })
        }

        ClientFrame::Claim { room, message, lease_secs } => {
            if !store.is_member(&room, &me.id)? {
                return Err(coded(error_code::NOT_MEMBER, format!("not a member of '{room}'")));
            }
            if store.room_kind(&room)? != Some(RoomKind::Service) {
                return Err(coded(
                    error_code::NOT_AUTHORIZED,
                    format!("'{room}' is not a service room"),
                ));
            }
            // The hub owns the lease bound regardless of client input. A worker may renew often,
            // but a hostile client cannot pin one task forever or churn the SQLite writer with
            // zero-length leases.
            let lease_secs = lease_secs.unwrap_or(300).clamp(15, 3_600);
            let lease_until = store.claim_service_message(&room, &message, &me.id, lease_secs, now_ms())?;
            Ok(ServerFrame::Claimed {
                room,
                message,
                claimed: lease_until.is_some(),
                lease_until,
            })
        }

        ClientFrame::Queue { room, role, limit } => {
            if !store.is_member(&room, &me.id)? {
                return Err(coded(error_code::NOT_MEMBER, format!("not a member of '{room}'")));
            }
            if store.room_kind(&room)? != Some(RoomKind::Service) {
                return Err(coded(
                    error_code::NOT_AUTHORIZED,
                    format!("'{room}' is not a service room"),
                ));
            }
            let messages = store.queued_service_messages(&room, &me.id, &role, limit, now_ms())?;
            Ok(ServerFrame::Queued { room, messages })
        }

        ClientFrame::Complete { room, message, status } => {
            if !store.is_member(&room, &me.id)? {
                return Err(coded(error_code::NOT_MEMBER, format!("not a member of '{room}'")));
            }
            if store.room_kind(&room)? != Some(RoomKind::Service) {
                return Err(coded(
                    error_code::NOT_AUTHORIZED,
                    format!("'{room}' is not a service room"),
                ));
            }
            if !status.is_terminal() {
                return Err(coded(
                    error_code::NOT_AUTHORIZED,
                    "a service claim can only be completed with done, failed, or cancelled",
                ));
            }
            let completed = store.complete_service_message(&room, &message, &me.id, status, now_ms())?;
            Ok(ServerFrame::Completed { room, message, completed })
        }

        ClientFrame::Send { target, parts, mentions, reply_to, client_id } => {
            if !state.rate_allows(&me.id, RateKind::Send, now_ms()) {
                return Err(coded(error_code::RATE_LIMITED, "rate limit: too many messages — slow down"));
            }
            // Bound per-message size (code rides blobs, not text) so a single send can't store an
            // outsized row.
            let parts_bytes = serde_json::to_vec(&parts).map(|v| v.len()).unwrap_or(0);
            if parts_bytes > state.max_message_bytes {
                return Err(coded(
                    error_code::TOO_LARGE,
                    format!(
                        "message too large: {parts_bytes} bytes > limit {} (hand off large payloads as a blob)",
                        state.max_message_bytes
                    ),
                ));
            }
            let room = resolve_target(store, me, &target)?;
            // Per-room ceiling on top of the per-agent one: bound the *aggregate* send rate of this room
            // so a busy/abusive room (many members, or one flooding agent) can't monopolize the shared
            // SQLite writer and stall every other room.
            if !state.room_rate_allows(&room, RateKind::Send, now_ms()) {
                return Err(coded(
                    error_code::RATE_LIMITED,
                    format!("rate limit: room '{room}' is too busy — slow down"),
                ));
            }
            let mentions = mentions.as_deref().and_then(normalize_mentions);
            let from = EndpointRef { id: me.id.clone(), name: me.name.clone(), role: me.role.clone() };
            let now = now_ms();
            let out = store.append_message(
                &room, &from, &parts, mentions.as_deref(), reply_to.as_deref(), client_id.as_deref(), now,
            )?;
            // A deduped send (idempotency-key replay, #86) is a no-op on state: the original row was
            // already counted and fanned out, so re-counting or re-pushing would resurrect the very
            // duplicate the key exists to prevent. Just echo the original message's id/seq back.
            if !out.deduped {
                state.metrics.messages_total.fetch_add(1, Ordering::Relaxed);
                // Same estimate the row stored, so the hub-wide counter and per-room totals agree.
                state.metrics.tokens_total.fetch_add(out.tokens.max(0) as u64, Ordering::Relaxed);
                // Best-effort live push to subscribed members (the durable cursor is still the source
                // of truth, so this only lowers latency — it never replaces `Pull`). Built from the
                // same fields just persisted, so a pushed message is byte-identical to the pulled one.
                state.fanout(
                    &room,
                    &me.id,
                    StoredMessage {
                        seq: out.seq,
                        id: out.id.clone(),
                        room: room.clone(),
                        from,
                        parts,
                        mentions,
                        reply_to,
                        ts: now,
                    },
                );
            }
            Ok(ServerFrame::Sent { id: out.id, seq: out.seq, room })
        }

        // A `Pull` with `wait_secs` on an authed connection is intercepted upstream (`waited_pull`);
        // reaching here means no wait was requested (or an edge that can't park), so serve it as an
        // immediate pull. `wait_secs` is deliberately ignored on this synchronous path.
        ClientFrame::Pull { room, since, limit, wait_secs: _, ack } => {
            if !store.is_member(&room, &me.id)? {
                return Err(coded(error_code::NOT_MEMBER, format!("not a member of '{room}'")));
            }
            let (messages, cursor) = store.pull(&room, &me.id, since, limit, ack)?;
            Ok(ServerFrame::Pulled { room, messages, cursor })
        }

        ClientFrame::Remember { fact, embedding, embedding_model } => {
            if let Some(room) = &fact.room {
                if !store.is_member(room, &me.id)? {
                    return Err(coded(error_code::NOT_MEMBER, format!("not a member of '{room}'")));
                }
            }
            let _quota = fact.key.as_ref().map(|_| state.durable_quota.lock());
            if let Some(key) = fact.key.as_deref() {
                if !store.keyed_fact_within_quota(
                    &me.id,
                    key,
                    fact.room.as_deref(),
                    state.max_keyed_facts,
                )? {
                    return Err(coded(
                        error_code::AT_CAPACITY,
                        "keyed fact quota reached — update an existing key or remove old memory",
                    ));
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

        ClientFrame::Recall { query, room, limit, embedding, key } => {
            if let Some(room) = &room {
                if !store.is_member(room, &me.id)? {
                    return Err(coded(error_code::NOT_MEMBER, format!("not a member of '{room}'")));
                }
            }
            // A `key` is a deterministic keyed fetch (#91) — exact fact under that key, no BM25.
            let hits = match key.as_deref().filter(|k| !k.is_empty()) {
                Some(key) => store.recall_by_key(&me.id, key, room.as_deref(), limit)?,
                None => store.recall(&me.id, &query, room.as_deref(), limit, embedding.as_deref())?,
            };
            Ok(ServerFrame::Recalled { hits })
        }

        ClientFrame::Rooms => Ok(ServerFrame::Rooms { rooms: store.rooms_of(&me.id)? }),

        ClientFrame::DeleteRoom { room } => {
            store.delete_room(&room, &me.id)?;
            state.notify_room(&room);
            Ok(ServerFrame::RoomDeleted { room })
        }

        ClientFrame::Roster { room } => {
            if !store.is_member(&room, &me.id)? {
                return Err(coded(error_code::NOT_MEMBER, format!("not a member of '{room}'")));
            }
            Ok(ServerFrame::Roster { room: room.clone(), entries: store.roster(&room, now_ms())? })
        }

        ClientFrame::Presence { status, activity, attention } => {
            store.touch_presence(&me.id, &status, activity.as_deref(), attention, now_ms())?;
            Ok(ServerFrame::PresenceOk)
        }

        ClientFrame::SetAttention { attention } => {
            store.set_attention(&me.id, attention, now_ms())?;
            Ok(ServerFrame::AttentionOk)
        }

        ClientFrame::Ping => {
            // A protocol heartbeat proves this identity's connection is still live. Refresh only
            // the timestamp: overwriting `working`/`waiting`, activity, or attention with `idle`
            // would make liveness itself corrupt the status it is meant to preserve.
            store.refresh_presence(&me.id, now_ms())?;
            Ok(ServerFrame::Pong)
        }

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
                return Err(coded(error_code::NOT_MEMBER, format!("not a member of '{room}'")));
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
                return Err(coded(
                    error_code::UNKNOWN_SERVICE,
                    format!("no such service '{service}' — a worker must `serve` it first"),
                ));
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

/// A challenge is valid for 60s — plenty for a round-trip, short enough to bound replay.
const CHALLENGE_TTL_MS: i64 = 60_000;

/// A short, colon-free token identifying this hub (first 12 hex of `sha256(public_url)`), so a
/// challenge minted by one hub can't validate at another.
fn hub_token(public_url: &str) -> String {
    parler_auth::content_id(public_url.as_bytes())[..12].to_string()
}

/// Build the challenge string the client signs verbatim: `parler-auth:v1:<hub>:<exp-ms>:<rand>`.
/// Signing a structured, self-describing token (rather than a bare UUID) **domain-separates** the
/// signature so it can't be repurposed, and the embedded expiry bounds replay. Zero client change —
/// the client just signs whatever opaque nonce it is handed.
fn issue_challenge(public_url: &str, now: i64) -> String {
    format!(
        "parler-auth:v1:{}:{}:{}",
        hub_token(public_url),
        now + CHALLENGE_TTL_MS,
        uuid::Uuid::new_v4()
    )
}

/// Validate a challenge we previously issued: correct shape/version, our hub token, not yet expired.
fn challenge_valid(nonce: &str, public_url: &str, now: i64) -> bool {
    let mut it = nonce.split(':');
    if it.next() != Some("parler-auth") || it.next() != Some("v1") {
        return false;
    }
    let (Some(hub), Some(exp)) = (it.next(), it.next().and_then(|s| s.parse::<i64>().ok())) else {
        return false;
    };
    hub == hub_token(public_url) && now <= exp
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
    fn retention_defaults_are_enabled() {
        // A deployed hub must bound its growth out of the box; regressing any of these back to
        // keep-everything is the ceiling we just closed. (Opt out per-knob via a 0/negative flag.)
        let r = Retention::default();
        assert!(r.message_max_age.is_some(), "messages must be age-bounded by default");
        assert!(r.keep_unkeyed_facts.is_some(), "unkeyed facts must be bounded by default");
        assert!(r.blob_max_idle.is_some(), "idle blobs must be GC'd by default");
        assert!(r.keep_messages_per_room > 0, "a positive per-room floor protects recent history");
    }

    #[test]
    fn prune_rate_windows_drops_only_idle_agents() {
        let store = Store::open(None).unwrap();
        let state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        let now = 10_000_000i64;

        // `active` just sent; `idle` last sent over an hour ago.
        state.rate_allows("active", RateKind::Send, now);
        state.rate_allows("idle", RateKind::Send, now - 3_600_001);

        assert_eq!(state.rate.lock().len(), 2);
        prune_rate_windows(&state, now);
        let map = state.rate.lock();
        assert!(map.contains_key("active"), "a recently active agent's counter is kept");
        assert!(!map.contains_key("idle"), "an agent idle past the longest window is dropped");
    }

    #[test]
    fn room_rate_allows_enforces_per_room_budget_independent_of_agents_and_rolls() {
        let store = Store::open(None).unwrap();
        let mut state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        state.limits.max_room_sends_per_min = 2;
        let now = 10_000_000i64;

        // The room's first two sends pass, the third in the same window is throttled — regardless of
        // which agent sends them, because the budget is the room's aggregate.
        assert!(state.room_rate_allows("roomA", RateKind::Send, now));
        assert!(state.room_rate_allows("roomA", RateKind::Send, now));
        assert!(!state.room_rate_allows("roomA", RateKind::Send, now), "over-budget room send is refused");
        // A different room has its own independent budget.
        assert!(state.room_rate_allows("roomB", RateKind::Send, now), "the limit is per-room, not global");
        // A new 60s window resets the room's counter.
        assert!(state.room_rate_allows("roomA", RateKind::Send, now + 60_000), "the window rolls after 60s");
    }

    #[test]
    fn room_rate_limit_of_zero_is_disabled() {
        let store = Store::open(None).unwrap();
        let mut state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        state.limits.max_room_sends_per_min = 0;
        state.limits.max_room_blobs_per_hour = 0;
        for _ in 0..1000 {
            assert!(state.room_rate_allows("busy", RateKind::Send, 1), "a 0 send budget never throttles");
            assert!(state.room_rate_allows("busy", RateKind::Blob, 1), "a 0 blob budget never throttles");
        }
    }

    #[test]
    fn room_limits_are_on_by_default() {
        // A deployed hub bounds per-room traffic out of the box; regressing to keep-everything reopens
        // the noisy-neighbor DoS. Blob and send windows are independent (different lengths).
        let l = RateLimits::default();
        assert!(l.max_room_sends_per_min > 0, "per-room send ceiling must be on by default");
        assert!(l.max_room_blobs_per_hour > 0, "per-room blob ceiling must be on by default");
        assert!(l.max_ops_per_min > 0, "post-upgrade WebSocket operations must be bounded");
    }

    #[test]
    fn operation_rate_limit_bounds_post_upgrade_frames() {
        let store = Store::open(None).unwrap();
        let mut state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        state.limits.max_ops_per_min = 2;
        assert!(state.rate_allows("U", RateKind::Operation, 1));
        assert!(state.rate_allows("U", RateKind::Operation, 1));
        assert!(!state.rate_allows("U", RateKind::Operation, 1));
        assert!(state.rate_allows("U", RateKind::Operation, 60_001));
    }

    #[test]
    fn public_mode_does_not_authorize_private_directory_scope() {
        let store = Store::open(None).unwrap();
        let state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        assert!(!hub_scope_authorized(&state, &HeaderMap::new()));

        let now = now_ms();
        state.store.mint_directory_token("secret", "hub", now + 10_000, "U", now).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer secret".parse().unwrap(),
        );
        assert!(hub_scope_authorized(&state, &headers));
    }

    #[test]
    fn concurrent_blob_reservations_share_one_byte_budget() {
        let store = Store::open(None).unwrap();
        store.ensure_room("room", RoomKind::Channel, None, 1).unwrap();
        store.add_member("room", "U", 1).unwrap();
        let mut state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        state.max_blob_bytes = 6;
        state.set_max_inflight_blob_bytes(10);
        let me = Authed { id: "U".into(), name: "worker".into(), role: None };
        let mut first = ConnState::default();
        let mut second = ConnState::default();
        let reserve = |state: &HubState, conn: &mut ConnState| {
            handle_put_blob(
                state,
                conn,
                &me,
                Target::Room { room: "room".into() },
                "id".into(),
                6,
                None,
            )
        };
        assert!(reserve(&state, &mut first).is_ok());
        let error = match reserve(&state, &mut second) {
            Err(error) => error,
            Ok(_) => panic!("a second six-byte upload exceeded the ten-byte aggregate budget"),
        };
        assert_eq!(
            error.downcast_ref::<CodedError>().and_then(|error| error.code.as_deref()),
            Some(error_code::AT_CAPACITY)
        );
        first.pending.take();
        assert!(reserve(&state, &mut second).is_ok());
    }

    #[test]
    fn durable_quotas_reject_without_mutating_state() {
        let store = Store::open(None).unwrap();
        let mut state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        state.max_active_tokens = 0;
        state.max_owned_rooms = 0;
        state.max_keyed_facts = 0;
        let me = Authed { id: "U".into(), name: "worker".into(), role: None };

        let capacity = |result: anyhow::Result<ServerFrame>| {
            let error = result.expect_err("a zero quota must reject the operation");
            assert_eq!(
                error.downcast_ref::<CodedError>().and_then(|error| error.code.as_deref()),
                Some(error_code::AT_CAPACITY)
            );
        };

        capacity(handle_authed(
            &state,
            &me,
            ClientFrame::MintDirectoryToken { ttl_secs: Some(60) },
        ));
        assert_eq!(state.store.active_token_count(&me.id, now_ms()).unwrap(), 0);

        capacity(handle_authed(
            &state,
            &me,
            ClientFrame::Invite {
                kind: RoomKind::Channel,
                room: Some("blocked".into()),
                ttl_secs: Some(60),
                max_uses: Some(1),
                require_approval: false,
            },
        ));
        assert_eq!(state.store.room_kind("blocked").unwrap(), None);

        capacity(handle_authed(
            &state,
            &me,
            ClientFrame::Remember {
                fact: parler_protocol::Fact {
                    key: Some("blocked".into()),
                    text: "must not persist".into(),
                    room: None,
                },
                embedding: None,
                embedding_model: None,
            },
        ));
        assert!(state.store.recall_by_key(&me.id, "blocked", None, Some(1)).unwrap().is_empty());
    }

    #[test]
    fn durable_quota_check_and_write_is_atomic_across_connections() {
        let store = Store::open(None).unwrap();
        let mut state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        state.max_active_tokens = 1;
        let state = Arc::new(state);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let state = state.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                let me = Authed { id: "U".into(), name: "worker".into(), role: None };
                barrier.wait();
                handle_authed(
                    &state,
                    &me,
                    ClientFrame::MintDirectoryToken { ttl_secs: Some(60) },
                )
            }));
        }
        barrier.wait();
        let successes = threads
            .into_iter()
            .map(|thread| matches!(thread.join().unwrap(), Ok(ServerFrame::DirectoryToken { .. })))
            .filter(|success| *success)
            .count();
        assert_eq!(successes, 1);
        assert_eq!(state.store.active_token_count("U", now_ms()).unwrap(), 1);
    }

    #[test]
    fn prune_rate_windows_drops_only_idle_rooms() {
        let store = Store::open(None).unwrap();
        let state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        let now = 10_000_000i64;

        // `active` just sent; `idle` last sent over an hour ago (past the longest window).
        state.room_rate_allows("active", RateKind::Send, now);
        state.room_rate_allows("idle", RateKind::Send, now - 3_600_001);

        assert_eq!(state.room_rate.lock().len(), 2);
        prune_rate_windows(&state, now);
        let map = state.room_rate.lock();
        assert!(map.contains_key("active"), "a recently active room's counter is kept");
        assert!(!map.contains_key("idle"), "a room idle past the longest window is dropped");
    }

    #[test]
    fn http_rate_allows_enforces_per_ip_budget_and_rolls_the_window() {
        let store = Store::open(None).unwrap();
        let mut state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        state.max_http_per_min = 2;
        let a: IpAddr = "1.1.1.1".parse().unwrap();
        let b: IpAddr = "2.2.2.2".parse().unwrap();
        let now = 10_000_000i64;

        // A's first two requests pass, the third in the same window is throttled.
        assert!(state.http_rate_allows(a, now));
        assert!(state.http_rate_allows(a, now));
        assert!(!state.http_rate_allows(a, now), "over-budget request is refused");
        // A different IP has its own independent budget.
        assert!(state.http_rate_allows(b, now), "the limit is per-IP, not global");
        // A new 60s window resets A's counter.
        assert!(state.http_rate_allows(a, now + 60_000), "the window rolls after 60s");
    }

    #[test]
    fn http_rate_limit_of_zero_is_disabled() {
        let store = Store::open(None).unwrap();
        let mut state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        state.max_http_per_min = 0;
        let ip: IpAddr = "9.9.9.9".parse().unwrap();
        for _ in 0..1000 {
            assert!(state.http_rate_allows(ip, 1), "a 0 budget never throttles");
        }
    }

    #[test]
    fn waitlist_rate_allows_enforces_the_tight_per_ip_budget() {
        let store = Store::open(None).unwrap();
        let state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        let ip: IpAddr = "1.1.1.1".parse().unwrap();
        let now = 10_000_000i64;
        // The first WAITLIST_MAX_PER_MIN signups pass; the next in the same window is throttled.
        for _ in 0..WAITLIST_MAX_PER_MIN {
            assert!(state.waitlist_rate_allows(ip, now));
        }
        assert!(!state.waitlist_rate_allows(ip, now), "over-budget signup is refused");
        // A new 60s window resets the counter.
        assert!(state.waitlist_rate_allows(ip, now + 60_000), "the window rolls after 60s");
    }

    #[test]
    fn email_validation_normalizes_and_rejects_garbage() {
        // Valid, after trim + lowercase normalization.
        assert!(valid_email(&normalize_email("  Alice@Example.COM ")));
        assert_eq!(normalize_email("  Alice@Example.COM "), "alice@example.com");
        assert!(valid_email("a@b.co"));
        assert!(valid_email("user.name+tag@sub.example.org"));
        // Invalid: no @, no dot in domain, empty local/domain, too short, whitespace/control, multi-@.
        assert!(!valid_email("no-at-sign.com"));
        assert!(!valid_email("nodot@localhost"));
        assert!(!valid_email("@example.com"));
        assert!(!valid_email("user@"));
        assert!(!valid_email("a@b"));
        assert!(!valid_email("a b@example.com"));
        assert!(!valid_email("a@b.com\n"));
        assert!(!valid_email("two@@example.com"));
        // Leading/trailing dot in the domain is rejected.
        assert!(!valid_email("user@.example.com"));
        assert!(!valid_email("user@example.com."));
        // 254-char boundary: at the limit is fine, one over is not.
        let local = "a".repeat(240);
        assert!(valid_email(&format!("{local}@example.com"))); // 252 chars
        assert!(!valid_email(&format!("{}@example.com", "a".repeat(250)))); // 262 chars
    }

    #[test]
    fn prune_rate_windows_drops_stale_http_ips() {
        let store = Store::open(None).unwrap();
        let mut state = HubState::new(store, "parler://x".into(), "T".into(), HubMode::Public);
        state.max_http_per_min = 10;
        let now = 10_000_000i64;
        let fresh: IpAddr = "1.1.1.1".parse().unwrap();
        let stale: IpAddr = "2.2.2.2".parse().unwrap();

        state.http_rate_allows(fresh, now);
        state.http_rate_allows(stale, now - 60_001); // window already fully elapsed
        prune_rate_windows(&state, now);
        let map = state.http_rate.lock();
        assert!(map.contains_key(&fresh), "an IP inside its 60s window is kept");
        assert!(!map.contains_key(&stale), "an IP past its window is dropped");
    }

    #[test]
    fn client_ip_uses_forwarded_headers_only_when_proxy_trust_is_enabled() {
        let peer: IpAddr = "10.0.0.9".parse().unwrap();
        let expect = |ip: &str| -> IpAddr { ip.parse().unwrap() };

        // Fly's edge header wins over everything.
        let mut h = HeaderMap::new();
        h.insert("fly-client-ip", "203.0.113.7".parse().unwrap());
        h.insert("x-forwarded-for", "198.51.100.4, 10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(&h, Some(peer), true), Some(expect("203.0.113.7")));
        assert_eq!(client_ip(&h, Some(peer), false), Some(peer));

        // No Fly header: the leftmost X-Forwarded-For hop (the real client) is used.
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "198.51.100.4, 10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(&h, Some(peer), true), Some(expect("198.51.100.4")));

        // No forwarded headers: fall back to the socket peer (direct/local connection).
        assert_eq!(client_ip(&HeaderMap::new(), Some(peer), false), Some(peer));
        // Nothing to key on at all: fail-open (None), so the caller lets the request through.
        assert_eq!(client_ip(&HeaderMap::new(), None, false), None);
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
        // `parler://` is the invite-link scheme; the connect snippet needs a `ws(s)://` URL.
        assert_eq!(display_hub_url("parler://127.0.0.1:7070"), "ws://127.0.0.1:7070");
        assert_eq!(display_hub_url("wss://hub.example"), "wss://hub.example");
        assert_eq!(display_hub_url("ws://127.0.0.1:7070"), "ws://127.0.0.1:7070");
        // A wildcard bind isn't dialable — show `localhost` so the snippet is copy-pasteable.
        assert_eq!(display_hub_url("parler://0.0.0.0:7070"), "ws://localhost:7070");
        assert_eq!(display_hub_url("parler://[::]:7070"), "ws://localhost:7070");
    }

    #[test]
    fn landing_page_includes_publish_snippet_and_escapes_name() {
        let html =
            landing_html("A & <b>", HubMode::Public, 3, 2, "wss://hub.example", Some("https://site.example"), false);
        assert!(html.contains("parler register"));
        assert!(html.contains("--public")); // a public hub publishes a world-readable card
        assert!(html.contains("wss://hub.example"));
        assert!(html.contains("A &amp; &lt;b&gt;")); // name is HTML-escaped
        assert!(html.contains("https://site.example")); // the web CTA is rendered when set
        assert!(!html.contains("PARLER_JOIN_SECRET")); // no secret prompt on a public hub
        // The MCP line must use `-e PARLER_HUB=…` so it persists into the stored config; a shell-env
        // prefix (`PARLER_HUB=… claude mcp add`) silently drops before `parler mcp` runs (issue #100).
        assert!(html.contains("claude mcp add parler -e"), "hub must be an -e flag:\n{html}");
        assert!(!html.contains("</span> claude mcp add"), "no shell-env-prefix form:\n{html}");
    }

    #[test]
    fn private_landing_page_prompts_for_secret_without_leaking_it() {
        // landing_html takes no secret, so the world-reachable page *cannot* print one — it only ever
        // shows the placeholder. Assert the private copy + the placeholder, and that `--public` is gone.
        let html = landing_html("Team", HubMode::Private, 2, 0, "ws://localhost:7070", None, true);
        assert!(html.contains("private Parler Protocol hub"));
        assert!(html.contains("PARLER_JOIN_SECRET=&lt;your-join-secret&gt;"));
        assert!(html.contains("claude mcp add parler"));
        assert!(html.contains("startup log")); // tells the operator where to find the real secret
        assert!(!html.contains("--public")); // private cards by default
        // Both the hub and the secret must ride as `-e` flags on `claude mcp add`, never as a
        // shell-env prefix that the launched `parler mcp` never sees (issue #100).
        assert!(html.contains("-e <span class=\"k\">PARLER_JOIN_SECRET"), "secret is an -e flag:\n{html}");
        assert!(!html.contains("&gt;</span> claude mcp add"), "no shell-env-prefix form:\n{html}");
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
            ServerFrame::Challenge { nonce, .. } => nonce,
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
            ServerFrame::Challenge { nonce, .. } => nonce,
            other => panic!("expected challenge, got {other:?}"),
        };
        let sig = parler_auth::sign(&id.seed, nonce.as_bytes()).unwrap();
        let ok = handle_hello(&state, &mut conn, id.id, "a".into(), None, Some(sig), None);
        assert!(matches!(ok, ServerFrame::Welcome { .. }));
    }

    #[test]
    fn challenge_nonce_is_domain_separated_hub_bound_and_expiring() {
        let url = "parler://hub-a";
        let n = issue_challenge(url, 1_000);
        assert!(n.starts_with("parler-auth:v1:"), "carries a domain-separating prefix");
        assert!(challenge_valid(&n, url, 1_000)); // fresh
        assert!(challenge_valid(&n, url, 1_000 + CHALLENGE_TTL_MS)); // at expiry: still valid
        assert!(!challenge_valid(&n, url, 1_001 + CHALLENGE_TTL_MS)); // past expiry
        assert!(!challenge_valid(&n, "parler://hub-b", 1_000)); // minted for a different hub
        assert!(!challenge_valid("garbage", url, 1_000)); // malformed
        assert!(!challenge_valid("parler-auth:v2:x:9999999999999:r", url, 1_000)); // wrong version
    }

    fn sample_entry(
        role: Option<&str>,
        skills: Option<Vec<parler_protocol::AgentSkill>>,
        tags: Option<Vec<String>>,
        sig: Option<&str>,
        verified: bool,
    ) -> DirectoryEntry {
        use parler_protocol::{AgentCard, EndpointKind, Visibility};
        DirectoryEntry {
            card: AgentCard {
                id: "UABC".into(),
                name: "planner".into(),
                kind: EndpointKind::Agent,
                role: role.map(str::to_string),
                description: Some("Decomposes goals into ordered plans.".into()),
                tags,
                skills,
                meta: None,
                protocol_version: Some("0.2".into()),
            },
            visibility: Visibility::Public,
            status: "idle".into(),
            activity: None,
            attention: None,
            hub: "Test Hub".into(),
            verified,
            sig: sig.map(str::to_string),
            first_seen: 1,
            last_seen: 2,
        }
    }

    #[test]
    fn a2a_card_projects_core_and_parler_fields() {
        use parler_protocol::AgentSkill;
        let entry = sample_entry(
            Some("planner"),
            Some(vec![AgentSkill { id: "decompose".into(), name: "Decompose".into(), description: None }]),
            Some(vec!["planning".into()]),
            Some("BASE64SIG"),
            true,
        );
        let card = a2a_card(&entry, "https://hub.example", "Test Hub");
        // Core A2A fields an ecosystem crawler reads.
        assert_eq!(card["protocolVersion"], A2A_PROTOCOL_VERSION);
        assert_eq!(card["name"], "planner");
        assert_eq!(card["url"], "https://hub.example/a2a/agents/UABC");
        assert_eq!(card["capabilities"]["streaming"], true);
        assert_eq!(card["skills"][0]["id"], "decompose");
        assert_eq!(card["skills"][0]["tags"][0], "planning"); // card tags ride on the skill
        // Parler Protocol-native, offline-verifiable identity in the extension object.
        assert_eq!(card["parler"]["id"], "UABC");
        assert_eq!(card["parler"]["signature"], "BASE64SIG");
        assert_eq!(card["parler"]["verified"], true);
        assert_eq!(card["parler"]["visibility"], "public");
        // We must NOT fake an A2A JWS `signatures` field (that would need the agent's seed).
        assert!(card.get("signatures").is_none());
    }

    #[test]
    fn a2a_card_synthesizes_a_skill_from_tags_when_none_given() {
        let entry = sample_entry(
            Some("reviewer"),
            None,
            Some(vec!["security".into(), "rust".into()]),
            None,
            false,
        );
        let card = a2a_card(&entry, "http://localhost:7070", "Test Hub");
        // No explicit skills, but tags/role must still surface as a synthesized skill.
        assert_eq!(card["skills"].as_array().unwrap().len(), 1);
        assert_eq!(card["skills"][0]["id"], "general");
        assert_eq!(card["skills"][0]["name"], "reviewer");
        assert_eq!(card["skills"][0]["tags"][1], "rust");
        // A missing native signature serializes as JSON null, not an empty string.
        assert!(card["parler"]["signature"].is_null());
    }

    #[test]
    fn request_base_url_is_proxy_aware_and_falls_back() {
        use axum::http::HeaderValue;
        // X-Forwarded-Proto wins (a deployed hub sits behind TLS-terminating Caddy/Fly).
        let mut h = HeaderMap::new();
        h.insert(axum::http::header::HOST, HeaderValue::from_static("hub.example"));
        h.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert_eq!(request_base_url(&h, "parler://ignored"), "https://hub.example");
        // A bare loopback Host defaults to http (local dev).
        let mut h = HeaderMap::new();
        h.insert(axum::http::header::HOST, HeaderValue::from_static("127.0.0.1:7070"));
        assert_eq!(request_base_url(&h, "parler://ignored"), "http://127.0.0.1:7070");
        // A bare public Host defaults to https.
        let mut h = HeaderMap::new();
        h.insert(axum::http::header::HOST, HeaderValue::from_static("hub.example"));
        assert_eq!(request_base_url(&h, "parler://ignored"), "https://hub.example");
        // No Host header → fall back to the configured public_url, as http(s) rather than ws(s).
        let empty = HeaderMap::new();
        assert_eq!(request_base_url(&empty, "parler://127.0.0.1:7070"), "http://127.0.0.1:7070");
    }

    // ---- server-side Pull wait (issue #90) -------------------------------------------------------

    /// A member of a fresh room `r` in a new hub state, ready to drive `store_waited_pull` directly.
    fn wait_test_state(room: &str, agent: &str) -> HubState {
        let store = Store::open(None).unwrap();
        store.ensure_room(room, RoomKind::Channel, None, 0).unwrap();
        store.add_member(room, agent, 0).unwrap();
        HubState::new(store, "parler://x".into(), "T".into(), HubMode::Private)
    }

    /// Append + notify, mirroring what the real `Send` handler does (`store.append_message` then
    /// `fanout`, which calls `notify_room`). A parked `store_waited_pull` wakes on the notify.
    fn append(state: &HubState, room: &str, author: &str, text: &str) {
        let from = EndpointRef { id: author.into(), name: author.into(), role: None };
        state.store.append_message(room, &from, &[Part::text(text)], None, None, None, now_ms()).unwrap();
        state.notify_room(room);
    }

    #[tokio::test]
    async fn waited_pull_returns_immediately_when_backlog_present() {
        // A wait with messages already waiting returns them at once (no parking) — the wait only kicks
        // in on an *empty* backlog.
        let me = Authed { id: "UME".into(), name: "me".into(), role: None };
        let state = wait_test_state("r", &me.id);
        append(&state, "r", "UPEER", "already here");
        let started = tokio::time::Instant::now();
        let (msgs, _cursor) = store_waited_pull(&state, &me, "r", None, None, None, 30).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(started.elapsed() < Duration::from_millis(500), "no parking when backlog is non-empty");
    }

    #[tokio::test]
    async fn waited_pull_wakes_on_a_message_landing_mid_wait() {
        // The park completes the instant a peer's message lands (via the room notify), not at timeout.
        let me = Authed { id: "UME".into(), name: "me".into(), role: None };
        let state = std::sync::Arc::new(wait_test_state("r", &me.id));

        let writer = {
            let state = state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(150)).await;
                append(&state, "r", "UPEER", "woke you");
            })
        };
        let started = tokio::time::Instant::now();
        let (msgs, _c) = store_waited_pull(&state, &me, "r", None, None, None, 30).await.unwrap();
        writer.await.unwrap();
        assert_eq!(msgs.len(), 1, "the parked pull returned the just-landed message");
        assert!(started.elapsed() < Duration::from_secs(5), "woke on the message, not the timeout");
    }

    #[tokio::test]
    async fn waited_pull_times_out_empty_without_advancing_the_cursor() {
        // On an empty room the wait returns an empty batch at the deadline, and — crucially — the
        // cursor is untouched, so a message sent afterward is still delivered by the next pull.
        let me = Authed { id: "UME".into(), name: "me".into(), role: None };
        let state = wait_test_state("r", &me.id);
        let started = tokio::time::Instant::now();
        let (msgs, _c) = store_waited_pull(&state, &me, "r", None, None, None, 1).await.unwrap();
        assert!(msgs.is_empty());
        assert!(started.elapsed() >= Duration::from_secs(1), "waited out the window");
        // Cursor untouched: a plain pull now sees a subsequently-sent message.
        append(&state, "r", "UPEER", "after");
        let (after, _c) = state.store.pull("r", &me.id, None, None, None).unwrap();
        assert_eq!(after.len(), 1, "the empty wait left the cursor in place");
    }

    #[tokio::test]
    async fn waited_pull_refuses_a_non_member() {
        // Authorization is unchanged: a non-member's waited pull errors immediately (no parking).
        let me = Authed { id: "UME".into(), name: "me".into(), role: None };
        let state = wait_test_state("r", "USOMEONE_ELSE"); // `me` is not added
        assert!(store_waited_pull(&state, &me, "r", None, None, None, 30).await.is_err());
    }

    #[tokio::test]
    async fn waited_pull_ignores_wait_for_an_explicit_since_reread() {
        // A `since` re-read is a full-detail history read, never a live tail: it must return
        // immediately (empty if the range is empty), not park.
        let me = Authed { id: "UME".into(), name: "me".into(), role: None };
        let state = wait_test_state("r", &me.id);
        let started = tokio::time::Instant::now();
        let (msgs, _c) = store_waited_pull(&state, &me, "r", Some(0), None, None, 30).await.unwrap();
        assert!(msgs.is_empty());
        assert!(started.elapsed() < Duration::from_millis(500), "a since re-read never waits");
    }
}
