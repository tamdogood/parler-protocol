//! parler-protocol::hub — wire frames for the Parler Protocol **Hub**, the lightweight transport behind the
//! chat protocol for AI agents (a WebSocket bus + an embedded durable store).
//!
//! These are the frames an agent's client (`parler-connector`) exchanges with the hub
//! (`parler-hub`). Like the rest of this crate they are pure serde types that perform no IO, so the
//! client and the server share one definition of the protocol. JSON field names that are multi-word
//! are camelCase to match the rest of the Parler Protocol envelope.

use crate::types::{AgentCard, EndpointRef, Part};
use serde::{Deserialize, Serialize};

/// What kind of room a name refers to. The three delivery patterns are all just rooms with
/// different membership shapes: a [`RoomKind::Channel`] with N members is one-to-many, a two-member
/// [`RoomKind::Dm`] is one-to-one, and a [`RoomKind::Service`] room that publishers share with the
/// worker(s) is many-to-one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomKind {
    Channel,
    Dm,
    Service,
}

impl RoomKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RoomKind::Channel => "channel",
            RoomKind::Dm => "dm",
            RoomKind::Service => "service",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "channel" => Some(RoomKind::Channel),
            "dm" => Some(RoomKind::Dm),
            "service" => Some(RoomKind::Service),
            _ => None,
        }
    }
}

/// Who may discover an agent in the hub's directory. **Secure by default:** an agent is
/// [`Visibility::Private`] unless it explicitly opts into [`Visibility::Public`].
///
/// - [`Visibility::Public`] — listed in the hub's world-readable public directory; discoverable by
///   any agent (and by anyone hitting the public REST API).
/// - [`Visibility::Private`] — discoverable only by agents in the **same hub** (an authenticated
///   member, or a holder of a read-scoped directory token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    #[default]
    Private,
}

/// How readily an agent wants inbound mesh traffic to interrupt its current work. This is advisory
/// presence metadata: the receiver enforces the policy locally, while the hub keeps delivering
/// durably according to room membership and cursors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Attention {
    /// Ambient room traffic may wake the agent.
    #[default]
    Open,
    /// Hold ambient room traffic; direct messages, addressed handoffs, and assigned work may wake it.
    Dnd,
    /// Hold everything except explicitly addressed handoffs and assigned role work.
    Focus,
}

impl Attention {
    pub fn as_str(self) -> &'static str {
        match self {
            Attention::Open => "open",
            Attention::Dnd => "dnd",
            Attention::Focus => "focus",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "open" => Some(Attention::Open),
            "dnd" => Some(Attention::Dnd),
            "focus" => Some(Attention::Focus),
            _ => None,
        }
    }
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "public" => Some(Visibility::Public),
            "private" => Some(Visibility::Private),
            _ => None,
        }
    }
}

/// The scope of a [`ClientFrame::Discover`] query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscoverScope {
    /// Every agent in this hub (public + private) — the "same private hub" view. Available to any
    /// authenticated member.
    #[default]
    Hub,
    /// Only this hub's `public` agents — the world-readable directory.
    Public,
}

/// A directory record as the hub returns it on [`ServerFrame::Directory`] / [`ServerFrame::Card`].
///
/// The `card` is the agent's self-described [`AgentCard`]; `sig` is the agent's detached nkey
/// signature over [`canonical_card_bytes`] of that card. Because an agent's `id` *is* its public
/// key, any consumer can verify `sig` and know the hub did not forge or alter the card — `verified`
/// is the hub's own check of exactly that.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub card: AgentCard,
    pub visibility: Visibility,
    /// Last-known presence status (`idle`/`working`/`waiting`/`offline`).
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// Receiver-side interruption preference, mirrored into presence for peers to observe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<Attention>,
    /// The hub (workspace) this agent registered in.
    pub hub: String,
    /// Whether the hub verified `sig` against `card.id` at registration.
    pub verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig: Option<String>,
    #[serde(rename = "firstSeen")]
    pub first_seen: i64,
    #[serde(rename = "lastSeen")]
    pub last_seen: i64,
}

/// Where a [`ClientFrame::Send`] is addressed. The hub resolves each to the concrete room it stores
/// the message under, so the three patterns share one code path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Target {
    /// One-to-many (or many-to-one): a named channel room.
    Room { room: String },
    /// One-to-one: the DM room shared with `agent` (established by a prior `dm` invite + redeem).
    Dm { agent: String },
    /// Many-to-one: a service room (`svc.<service>`) shared by requesters and the worker(s).
    Service { service: String },
}

/// A fact in the memory store. `key` makes a write idempotent — the same key overwrites within its
/// scope rather than appending a duplicate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub text: String,
    /// Room scope; `None` = the author's private memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
}

/// A stored message as the hub returns it on [`ClientFrame::Pull`] (room-scoped, with its monotonic
/// per-hub `seq` — the cursor unit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredMessage {
    pub seq: i64,
    pub id: String,
    pub room: String,
    pub from: EndpointRef,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<String>>,
    #[serde(default, rename = "replyTo", skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub ts: i64,
}

/// A recall hit: the matched fact plus its relevance (BM25 — lower is a better match).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallHit {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    pub author: String,
    pub ts: i64,
    pub score: f64,
}

/// A pending request to join an **approval-gated** room: who is asking, and when. Surfaced to the
/// room's owner so it can vet (approve or deny) the requester *before* it is admitted — until then
/// the requester is not a member and cannot read the room's backlog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequest {
    /// The requester's agent id (its public key).
    pub agent: String,
    /// The requester's display name.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// When the request was made (epoch-ms).
    #[serde(rename = "requestedAt")]
    pub requested_at: i64,
}

/// A roster entry — a room member with their last-known presence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// Receiver-side interruption preference, mirrored into presence for peers to observe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<Attention>,
    /// The role this member is actively serving in this service room, if any.
    #[serde(default, rename = "serviceRole", skip_serializing_if = "Option::is_none")]
    pub service_role: Option<String>,
    #[serde(rename = "lastSeen")]
    pub last_seen: i64,
}

/// A room the calling agent belongs to, with its unread count (messages past the agent's cursor).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomInfo {
    pub name: String,
    pub kind: RoomKind,
    pub members: u32,
    pub unread: u32,
}

/// Frames the client sends to the hub.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ClientFrame {
    /// Identify + authenticate. Sent first with `sig = None`; the hub replies
    /// [`ServerFrame::Challenge`] and the client re-sends with `nonce` echoed and `sig` = the
    /// base64 nkey signature over that nonce (proves it owns the `id` keypair).
    Hello {
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nonce: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sig: Option<String>,
        /// A shared join secret, presented on the signed (second) hello. Required only when the hub
        /// is configured with one (private hubs); ignored otherwise. Gates *who may connect* —
        /// proving key ownership alone is not authorization on a closed hub.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret: Option<String>,
    },
    /// Mint an invite code/link. `room` is optional for `channel` (auto-named when absent) and
    /// ignored for `dm` (a fresh DM room is always created).
    Invite {
        kind: RoomKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        room: Option<String>,
        #[serde(default, rename = "ttlSecs", skip_serializing_if = "Option::is_none")]
        ttl_secs: Option<u64>,
        #[serde(default, rename = "maxUses", skip_serializing_if = "Option::is_none")]
        max_uses: Option<u32>,
        /// When set, redeeming this invite does **not** join immediately — it records a *pending
        /// request* the room owner must approve first (see [`ClientFrame::ResolveJoin`]). Default
        /// `false` preserves the historical "redeem joins on the spot" behavior. Live sessions set it
        /// so the host vets every agent that asks to enter the shared conversation.
        #[serde(default, rename = "requireApproval", skip_serializing_if = "is_false")]
        require_approval: bool,
    },
    /// Redeem a pasted code. Joins the room it grants — or, if the invite is approval-gated, records a
    /// pending request (the hub replies [`ServerFrame::JoinPending`]) until the owner approves.
    Redeem { code: String },
    /// List the pending join requests for `room` — authorized only for the room's **owner** (the
    /// agent whose invite created it). Replies [`ServerFrame::JoinRequests`].
    JoinRequests { room: String },
    /// Approve or deny a pending join request for `room`. Only the room owner may call it: on
    /// `approve` the requester is admitted as a member; on deny it is rejected (and cannot re-request).
    /// Replies [`ServerFrame::JoinResolved`].
    ResolveJoin {
        room: String,
        agent: String,
        approve: bool,
    },
    /// Join/create a service room as a worker, so `Send`/`Pull` on it are authorized.
    Serve { service: String },
    /// Atomically claim one role-dispatched message in a service room. Only a worker that has served
    /// the room and is currently `idle`/`waiting` may claim it. Repeating a successful claim by the
    /// same worker renews its lease; another worker may take it after the lease expires.
    Claim {
        room: String,
        message: String,
        #[serde(default, rename = "leaseSecs", skip_serializing_if = "Option::is_none")]
        lease_secs: Option<u64>,
    },
    /// List unclaimed (or expired) role-addressed work for a service worker. This is deliberately a
    /// queue read rather than a room cursor read: a worker that crashes after claiming work must be
    /// able to discover the expired lease after it reconnects, even though it already pulled the
    /// service room's broadcast log.
    Queue {
        room: String,
        role: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    /// Mark a claim terminal after the local worker has published its signed task receipt. Only the
    /// current claim owner may complete it; an expired claim that another worker took cannot be
    /// completed by the old worker.
    Complete {
        room: String,
        message: String,
        status: TaskStatus,
    },
    /// Publish (or refresh) this agent's directory card. `card.id` must equal the authenticated
    /// agent id; `sig` is the agent's nkey signature over [`canonical_card_bytes`] of `card`, which
    /// the hub verifies so the stored entry is tamper-evident.
    Register {
        card: AgentCard,
        #[serde(default)]
        visibility: Visibility,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sig: Option<String>,
    },
    /// Search the directory. [`DiscoverScope::Hub`] returns every agent in this hub;
    /// [`DiscoverScope::Public`] returns only its `public` agents. Optional `query`/`tag`/`skill`/
    /// `status` narrow the result.
    Discover {
        #[serde(default)]
        scope: DiscoverScope,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    /// Fetch a single agent's directory card by id.
    Lookup { id: String },
    /// Mint a read-scoped, expiring directory token (a bearer the website pastes to view this hub's
    /// private directory over the REST API).
    MintDirectoryToken {
        #[serde(default, rename = "ttlSecs", skip_serializing_if = "Option::is_none")]
        ttl_secs: Option<u64>,
    },
    /// Mint a read-only, expiring **watch** token for `room` — a bearer the session owner pastes into
    /// the website to *view* (not join) the conversation and how many agents are in it, over the REST
    /// API. Only the room's **owner** may mint one (the same authority that approves joiners), so a
    /// leaked *join* key still can't read the backlog: viewing is a capability the host grants
    /// explicitly, scoped to this one room, read-only, and time-bounded. Replies [`ServerFrame::Watch`].
    MintWatch {
        room: String,
        #[serde(default, rename = "ttlSecs", skip_serializing_if = "Option::is_none")]
        ttl_secs: Option<u64>,
    },
    /// Publish a message to a target.
    ///
    /// `client_id` is an optional idempotency key the sender generates once per logical send and
    /// reuses on a transparent retry-after-reconnect: the hub enforces `(room, author, client_id)`
    /// unique, so a retry whose first attempt already landed returns the original message's id/seq
    /// instead of double-posting (Kafka idempotent-producer / NATS `Nats-Msg-Id` pattern). Absent ⇒
    /// today's at-least-once behavior, byte-identical on the wire (older hubs ignore the field).
    Send {
        target: Target,
        parts: Vec<Part>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mentions: Option<Vec<String>>,
        #[serde(default, rename = "replyTo", skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
        #[serde(default, rename = "clientId", skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
    },
    /// Pull messages for a room newer than the agent's stored cursor (which this advances), or newer
    /// than `since` (which does not advance the cursor — for re-reads).
    ///
    /// `wait_secs` turns this into a **long-poll**: when the backlog is empty the hub parks the
    /// request (bounded ≤ 60s, counted as connection activity) and completes it the moment a message
    /// lands in `room` — or the timer fires (an empty `Pulled`). Absent ⇒ today's immediate reply, so
    /// a `Pull` with no `wait_secs` is byte-identical on the wire and behaves exactly as before. The
    /// wait resolves through normal Pull/cursor semantics: it never advances the cursor except through
    /// the returned batch. (Older hubs ignore the unknown field and reply immediately, which a client
    /// treats as "server-side wait unsupported" and falls back to short polling.)
    ///
    /// `ack` is a deferred/piggybacked acknowledgement (#85): "I have durably received everything up
    /// to seq X." The hub advances the member cursor to `ack` **first** (monotonic, never backward),
    /// then reads `seq > cursor` — but with `ack` present it does **not** advance the cursor past the
    /// returned batch (the *next* pull acks it). This closes the pull-loss window: a batch whose reply
    /// is lost on a drop is re-read on the retry instead of skipped, since its cursor never moved.
    /// Absent ⇒ today's advance-on-read (old clients unaffected; older hubs ignore the field).
    Pull {
        room: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
        #[serde(default, rename = "waitSecs", skip_serializing_if = "Option::is_none")]
        wait_secs: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ack: Option<i64>,
    },
    /// Write a fact to the memory store.
    Remember {
        fact: Fact,
        /// Client-supplied embedding vector for semantic recall. The hub stores it alongside the
        /// fact; dimension must match the hub's configured `vec_dimension` (default 768).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        embedding: Option<Vec<f32>>,
        /// Which model produced the embedding (e.g. `"text-embedding-3-small"`), stored so
        /// mixed models are detectable. Informational; the hub does not interpret it.
        #[serde(default, rename = "embeddingModel", skip_serializing_if = "Option::is_none")]
        embedding_model: Option<String>,
    },
    /// Recall from the memory store. Pure text runs FTS5/BM25; with an `embedding`, the hub runs
    /// hybrid BM25 + vector KNN fused via Reciprocal Rank Fusion (best of both). Text-only or
    /// vector-only also works — either field may be empty/absent for graceful degradation.
    ///
    /// With `key` set (#91), the hub instead returns the exact fact(s) stored under that key
    /// (`parler_remember key=…`), room-scoped, **skipping BM25 entirely** — a deterministic keyed
    /// fetch independent of full-text ranking. `query` is still sent as the BM25 fallback so an older
    /// hub that doesn't know `key` degrades to the previous heuristic instead of failing.
    Recall {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        room: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        embedding: Option<Vec<f32>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    /// List the rooms the agent belongs to.
    Rooms,
    /// Permanently delete a room and its room-scoped data. Owner-only.
    DeleteRoom { room: String },
    /// The members + presence of a room.
    Roster { room: String },
    /// Advertise presence.
    Presence {
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activity: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attention: Option<Attention>,
    },
    /// Change the advisory global interruption mode without overwriting the host's current lifecycle
    /// status/activity. Room-level quiet/muted overrides never leave the receiver.
    SetAttention { attention: Attention },
    /// Reserve storage for a content-addressed blob (e.g. a git bundle) bound to the room that
    /// `target` resolves to. The hub checks membership + the size cap, replies
    /// [`ServerFrame::BlobReady`], and then expects the bytes as a **single binary frame**; once it
    /// verifies they hash to `sha256` and match `size` it replies [`ServerFrame::BlobStored`].
    PutBlob {
        target: Target,
        /// The content id the bytes must hash to (lowercase-hex SHA-256).
        sha256: String,
        /// The exact byte length of the blob to follow.
        size: u64,
        #[serde(default, rename = "mediaType", skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
    },
    /// Fetch a stored blob by its content id (as carried in a [`BundleRef`]). The hub checks the
    /// caller is a member of a room the blob was posted to, replies [`ServerFrame::BlobIncoming`],
    /// then sends the bytes as a single binary frame.
    GetBlob { id: String },
    /// Ask the hub to **push** new room messages to this connection as [`ServerFrame::Delivery`]
    /// frames (sub-second delivery), for every room the agent belongs to now or joins later. A
    /// standing intent that ends when the connection closes. Best-effort: a push the hub can't
    /// deliver (slow/closed socket) is simply dropped — the durable per-room cursor still returns it
    /// on the next [`ClientFrame::Pull`], so push never changes the delivery guarantee, only latency.
    /// The hub acks with [`ServerFrame::Subscribed`]. (Older hubs don't know this op and answer
    /// [`ServerFrame::Error`]; a client treats that as "push unsupported" and keeps polling.)
    Subscribe,
    Ping,
}

/// Frames the hub sends back. Every non-error frame is the direct reply to the client's last op.
///
/// Variants differ a lot in size (a `Pulled` carries a `Vec`, most carry a few strings), but this is
/// a short-lived wire type built once per reply — boxing the big variants would complicate the serde
/// tagging and every match site for no real gain, so we accept the spread.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    /// Step 1 of the handshake: sign this `nonce` and re-send `Hello`. `version` is the hub's protocol
    /// version — additive and optional, so an older hub that omits it (and an older client that ignores
    /// it) both keep working; a newer client uses it to warn on a major-version mismatch.
    Challenge {
        nonce: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    /// Handshake complete — the connection is authenticated as `id`.
    Welcome { id: String, name: String },
    Invited {
        code: String,
        url: String,
        room: String,
        kind: RoomKind,
        #[serde(rename = "expiresAt")]
        expires_at: i64,
    },
    Joined {
        room: String,
        kind: RoomKind,
    },
    /// The result of an atomic service-task claim. `claimed = false` means another available worker
    /// already owns the live lease; it is a normal queue outcome, not an error.
    Claimed {
        room: String,
        message: String,
        claimed: bool,
        #[serde(default, rename = "leaseUntil", skip_serializing_if = "Option::is_none")]
        lease_until: Option<i64>,
    },
    /// Available role-addressed work returned to a registered service worker. This read never
    /// changes the room cursor; [`ClientFrame::Claim`] is the routing decision.
    Queued {
        room: String,
        messages: Vec<StoredMessage>,
    },
    /// Result of [`ClientFrame::Complete`]. `completed = false` means the caller no longer owned
    /// the claim (normally because its lease expired and another worker took it).
    Completed {
        room: String,
        message: String,
        completed: bool,
    },
    /// A redeem of an approval-gated invite was recorded as a **pending request** — the room owner
    /// must approve before the caller is admitted. The caller is *not* a member yet (it cannot read
    /// the room until approved); it may re-redeem the same code to poll for the decision.
    JoinPending {
        room: String,
    },
    /// The pending join requests for a room the caller owns (reply to [`ClientFrame::JoinRequests`]).
    JoinRequests {
        room: String,
        requests: Vec<JoinRequest>,
    },
    /// The outcome of a [`ClientFrame::ResolveJoin`]: `approved` is `true` when the requester was
    /// admitted, `false` when denied.
    JoinResolved {
        room: String,
        agent: String,
        approved: bool,
    },
    Registered {
        id: String,
        visibility: Visibility,
        verified: bool,
    },
    Directory {
        agents: Vec<DirectoryEntry>,
    },
    Card {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry: Option<DirectoryEntry>,
    },
    DirectoryToken {
        token: String,
        #[serde(rename = "expiresAt")]
        expires_at: i64,
    },
    /// A minted read-only **watch** token (reply to [`ClientFrame::MintWatch`]) — the bearer the owner
    /// pastes into the website's session viewer to read `room`'s messages + roster.
    Watch {
        token: String,
        room: String,
        #[serde(rename = "expiresAt")]
        expires_at: i64,
    },
    Sent {
        id: String,
        seq: i64,
        room: String,
    },
    Pulled {
        room: String,
        messages: Vec<StoredMessage>,
        cursor: i64,
    },
    Remembered {
        ok: bool,
    },
    Recalled {
        hits: Vec<RecallHit>,
    },
    Rooms {
        rooms: Vec<RoomInfo>,
    },
    RoomDeleted {
        room: String,
    },
    Roster {
        room: String,
        entries: Vec<RosterEntry>,
    },
    PresenceOk,
    AttentionOk,
    /// Storage reserved for a [`ClientFrame::PutBlob`] — send the bytes as one binary frame next.
    BlobReady {
        id: String,
    },
    /// A blob was received, verified (hash + size), and persisted.
    BlobStored {
        id: String,
        size: u64,
    },
    /// A [`ClientFrame::GetBlob`] is authorized — the bytes follow as one binary frame.
    BlobIncoming {
        id: String,
        size: u64,
        #[serde(default, rename = "mediaType", skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
    },
    /// Acknowledges a [`ClientFrame::Subscribe`] — this connection will now receive [`Delivery`]
    /// pushes for the agent's rooms.
    ///
    /// [`Delivery`]: ServerFrame::Delivery
    Subscribed,
    /// A **pushed** room message — sent unsolicited (not in reply to any op) to a subscribed member
    /// the instant a peer's [`ClientFrame::Send`] lands. It is never echoed to the message's own
    /// author, and it does **not** advance the recipient's durable cursor: a subscriber still
    /// [`ClientFrame::Pull`]s to advance/dedup (the push only wakes it sooner). A client that didn't
    /// subscribe never sees this frame; one that did must demultiplex it from request replies.
    Delivery {
        message: StoredMessage,
    },
    Pong,
    Error {
        message: String,
        /// A stable, machine-readable classifier from [`error_code`] — lets a client branch on *why*
        /// a request failed (retryable rate-limit vs terminal not-a-member) without matching on the
        /// human `message`. Optional and `skip`-ped when absent, so an old hub that never sets it and
        /// an old client that never reads it both stay byte-compatible.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

impl ServerFrame {
    /// An uncoded error frame (no [`error_code`] classifier) — the message is the whole story.
    pub fn error(message: impl Into<String>) -> ServerFrame {
        ServerFrame::Error { message: message.into(), code: None }
    }

    /// An error frame tagged with a stable [`error_code`] so a client can branch on the cause.
    pub fn error_coded(code: &str, message: impl Into<String>) -> ServerFrame {
        ServerFrame::Error { message: message.into(), code: Some(code.to_string()) }
    }
}

/// A stable, machine-readable error classifier carried in [`ServerFrame::Error::code`].
///
/// The **values are the wire contract** — a client branches on these exact strings, so they may be
/// added to but never renamed. The human `message` beside the code is free to change; the code is not.
pub mod error_code {
    /// A frame arrived before the `Hello` handshake authenticated the connection.
    pub const UNAUTHENTICATED: &str = "unauthenticated";
    /// The client frame did not parse (malformed JSON / unknown shape).
    pub const BAD_FRAME: &str = "bad_frame";
    /// A protocol-sequencing violation (e.g. a binary frame with no `PutBlob` in flight).
    pub const PROTOCOL: &str = "protocol";
    /// The hub is at its connection cap; the client should retry shortly.
    pub const AT_CAPACITY: &str = "at_capacity";
    /// The unauthenticated handshake did not complete before the timeout.
    pub const TIMEOUT: &str = "timeout";
    /// The caller is not a member of the target room (terminal — a retry fails identically).
    pub const NOT_MEMBER: &str = "not_member";
    /// No worker is serving the named service queue.
    pub const UNKNOWN_SERVICE: &str = "unknown_service";
    /// A pasted invite/session code is invalid, expired, or used up.
    pub const INVALID_INVITE: &str = "invalid_invite";
    /// The caller is not the owner of the room and the op is owner-only.
    pub const NOT_OWNER: &str = "not_owner";
    /// The client exceeded a rate limit (messages or blob uploads); back off and retry.
    pub const RATE_LIMITED: &str = "rate_limited";
    /// A payload exceeded a size cap (e.g. a blob larger than the hub's limit).
    pub const TOO_LARGE: &str = "too_large";
    /// The hub's blob storage budget is exhausted; try again later.
    pub const STORAGE_FULL: &str = "storage_full";
    /// The caller is not authorized for the requested resource (e.g. a blob it can't reach).
    pub const NOT_AUTHORIZED: &str = "not_authorized";
    /// The requested blob id does not exist.
    pub const UNKNOWN_BLOB: &str = "unknown_blob";
    /// A discovery card failed validation (id mismatch or bad signature).
    pub const INVALID_CARD: &str = "invalid_card";
    /// An unexpected internal hub failure (serialization, missing bytes) — not the caller's fault.
    pub const INTERNAL: &str = "internal";
}

/// An error tagged with a stable [`error_code`] classifier, shared by both ends of the wire.
///
/// The **hub** raises it (`Err(CodedError::new(error_code::NOT_MEMBER, "…").into())`) so a failure
/// carries its classifier through `?` to the reply path, where the hub projects it onto the wire as a
/// coded [`ServerFrame::Error`]. The **client** reconstructs it from a received frame so any caller
/// can `downcast_ref::<CodedError>()` and branch on `.code`. `Display` is just the message, so a
/// `CodedError` prints and `.to_string()`s exactly like the plain `anyhow!` it replaces — nothing that
/// only reads the message text changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodedError {
    /// The stable classifier from [`error_code`], if the origin set one.
    pub code: Option<String>,
    /// The human-readable message.
    pub message: String,
}

impl CodedError {
    /// A coded error with a stable [`error_code`] classifier.
    pub fn new(code: &str, message: impl Into<String>) -> CodedError {
        CodedError { code: Some(code.to_string()), message: message.into() }
    }

    /// Reconstruct from a received [`ServerFrame::Error`]'s parts (the client side of the wire).
    pub fn from_wire(code: Option<String>, message: impl Into<String>) -> CodedError {
        CodedError { code, message: message.into() }
    }
}

impl std::fmt::Display for CodedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CodedError {}

/// `skip_serializing_if` helper: omit a `bool` field from the wire when it is `false`, so an
/// approval flag defaults cleanly to absent (and old peers that never set it stay byte-compatible).
fn is_false(b: &bool) -> bool {
    !*b
}

// ---- communication-cost metrics (estimated) ------------------------------------------------------

/// A rough token estimate for a piece of text.
///
/// The hub is a **relay, not an LLM** — it never runs a model, so it cannot know a given model's exact
/// tokenization. This approximates it with the widely-used **~4 characters per token** heuristic,
/// counting Unicode scalar values (not bytes, so multi-byte text isn't overcounted). It exists for
/// directional "how much have these agents been talking / how much has this cost" insight and is always
/// surfaced as an estimate (`≈`), never an exact billing figure.
pub fn estimate_tokens(text: &str) -> u64 {
    chars_to_tokens(text.chars().count())
}

/// `ceil(chars / 4)` — the shared core, so a whole message and a bare string agree on the rule.
fn chars_to_tokens(chars: usize) -> u64 {
    (chars as u64).div_ceil(4)
}

/// Estimated tokens carried by a message's parts — the language content an agent actually exchanged.
///
/// Sums every human-readable string the message carries: [`Part::Text`] verbatim, plus the string
/// values inside [`Part::Data`] and extension parts (a tool observation's output, a handoff's
/// instruction, …). The detached author-signature part ([`MESSAGE_SIG_KIND`]) is skipped — it is
/// plumbing, not conversation. Uses the same [`estimate_tokens`] heuristic, so it is an estimate by the
/// same rule.
pub fn estimate_message_tokens(parts: &[Part]) -> u64 {
    let mut chars = 0usize;
    for p in parts {
        match p {
            Part::Text(t) => chars += t.chars().count(),
            Part::Data(v) => chars += json_string_chars(v),
            Part::Extension { fields, .. } => {
                // The message signature is a base64 blob + routing echo, not words the agents said.
                if is_message_sig_part(p) {
                    continue;
                }
                for v in fields.values() {
                    chars += json_string_chars(v);
                }
            }
        }
    }
    chars_to_tokens(chars)
}

/// Total length (in Unicode scalar values) of every string leaf in a JSON value — so a structured part
/// contributes the text it actually carries, not its JSON punctuation or numeric/boolean scaffolding.
fn json_string_chars(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::String(s) => s.chars().count(),
        serde_json::Value::Array(a) => a.iter().map(json_string_chars).sum(),
        serde_json::Value::Object(m) => m.values().map(json_string_chars).sum(),
        _ => 0,
    }
}

/// The reverse-DNS [`Part`] kind that references a code/artifact bundle handed off through a room.
pub const BUNDLE_KIND: &str = "com.parler.bundle";

/// The reverse-DNS [`Part`] kind that references an arbitrary **file** handed off through a room.
/// Files ride the exact same content-addressed blob transport as [`BUNDLE_KIND`]. See [`FileRef`].
pub const FILE_KIND: &str = "com.parler.file";

/// The reverse-DNS [`Part`] kind that carries a structured turn handoff — an explicit "you're up
/// next" between agents sharing a room. See [`HandoffRef`].
pub const HANDOFF_KIND: &str = "com.parler.handoff";

/// The reverse-DNS [`Part`] kind that carries a **task status update** — where a dispatched unit of
/// work stands in its lifecycle. See [`TaskRef`].
pub const TASK_KIND: &str = "com.parler.task";

/// The reverse-DNS [`Part`] kind that marks a service-room message as role-addressed work. Legacy
/// `--service` messages remain ordinary room broadcasts; a `DispatchRef` opts into the atomic
/// anycast claim path used by `parler work --role`.
pub const DISPATCH_KIND: &str = "com.parler.dispatch";

/// A reference to a content-addressed artifact (a git bundle by default) carried inside a room
/// message as a [`Part::Extension`] of kind [`BUNDLE_KIND`].
///
/// The bytes live in the hub's blob store under `blob` (their SHA-256); the message only points at
/// them, so a code handoff rides the ordinary room / cursor / durability machinery — `send`/`recv`
/// are unchanged, and a client that doesn't understand the kind still sees a renderable extension
/// part. Build one with [`BundleRef::to_part`]; recover it from a received part with
/// [`BundleRef::from_part`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleRef {
    /// Content id (lowercase-hex SHA-256) of the bytes — the key passed to [`ClientFrame::GetBlob`].
    pub blob: String,
    /// The artifact kind: `"git"` (a git bundle), `"patch"`, `"tar"`, …
    pub vcs: String,
    /// The bundled tip (e.g. the commit hash at HEAD), when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<String>,
    /// The base/prerequisite the bundle is thin against (a commit the receiver must already have).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// A one-line human summary (e.g. the tip's commit subject).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Byte length of the blob.
    pub size: u64,
    #[serde(default, rename = "mediaType", skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

impl BundleRef {
    /// Encode as the `com.parler.bundle` extension [`Part`].
    pub fn to_part(&self) -> Part {
        let fields = match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        Part::Extension { kind: BUNDLE_KIND.to_string(), fields }
    }

    /// Recover a [`BundleRef`] from a part — `Some` iff it is a well-formed `com.parler.bundle`
    /// extension.
    pub fn from_part(part: &Part) -> Option<BundleRef> {
        match part {
            Part::Extension { kind, fields } if kind == BUNDLE_KIND => {
                serde_json::from_value(serde_json::Value::Object(fields.clone())).ok()
            }
            _ => None,
        }
    }
}

/// A reference to a content-addressed **file** carried inside a room message as a [`Part::Extension`]
/// of kind [`FILE_KIND`] — the plain-file sibling of [`BundleRef`].
///
/// The bytes live in the hub's blob store under `blob` (their SHA-256) and are pulled with
/// [`ClientFrame::GetBlob`]; the message only points at them, so a file transfer rides the ordinary
/// `send`/`recv`/cursor/durability/reconnect machinery with no new wire frame. Unlike a code bundle
/// it carries the original `name` so a receiver can save it back to disk, and no VCS/commit fields.
/// Build one with [`FileRef::to_part`]; recover it with [`FileRef::from_part`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRef {
    /// Content id (lowercase-hex SHA-256) of the bytes — the key passed to [`ClientFrame::GetBlob`].
    pub blob: String,
    /// The original file name (basename only), so a receiver can save it back with the same name.
    pub name: String,
    /// Byte length of the file.
    pub size: u64,
    /// IANA media type (e.g. `image/png`, `application/pdf`), when known.
    #[serde(default, rename = "mediaType", skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// An optional one-line human description shown alongside the reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl FileRef {
    /// Encode as the `com.parler.file` extension [`Part`].
    pub fn to_part(&self) -> Part {
        let fields = match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        Part::Extension { kind: FILE_KIND.to_string(), fields }
    }

    /// Recover a [`FileRef`] from a part — `Some` iff it is a well-formed `com.parler.file` extension.
    pub fn from_part(part: &Part) -> Option<FileRef> {
        match part {
            Part::Extension { kind, fields } if kind == FILE_KIND => {
                serde_json::from_value(serde_json::Value::Object(fields.clone())).ok()
            }
            _ => None,
        }
    }
}

/// The reverse-DNS [`Part`] kind that carries an author's detached signature over a message.
///
/// Like [`BUNDLE_KIND`], a signature rides *inside* the message's `parts` as a [`Part::Extension`],
/// so authenticating a message needs **no new wire frame and no hub/store change** — the hub already
/// persists and returns arbitrary parts verbatim. A client that doesn't understand the kind still
/// sees a renderable extension part; one that does verifies it against the author's id (its public
/// key) and learns whether the (possibly untrusted) hub forged or altered the authored content.
pub const MESSAGE_SIG_KIND: &str = "com.parler.sig";

/// Signed-payload schema version embedded in [`canonical_message_bytes`] and the sig part.
pub const MESSAGE_SIG_V: u64 = 1;

/// `true` iff `p` is a [`MESSAGE_SIG_KIND`] extension part (the detached message signature).
pub fn is_message_sig_part(p: &Part) -> bool {
    matches!(p, Part::Extension { kind, .. } if kind == MESSAGE_SIG_KIND)
}

/// An author's detached signature over a message, carried as a [`MESSAGE_SIG_KIND`] extension part.
///
/// The signature covers [`canonical_message_bytes`] of the message's *content* — the author id, the
/// routing `target` the author chose, the non-signature `parts`, the optional `replyTo`, and the
/// author-stamped `ts`/`uid`. It deliberately does **not** cover hub-assigned routing metadata
/// (`seq`, the resolved room name, the hub's own `ts`): those are the relay's to set. Autonomous
/// receivers bind the signed target to the delivered room and durably reject a repeated signed
/// `(author, uid)` before acting. `mentions` are excluded because the hub normalizes them in flight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSig {
    /// Base64 nkey/Ed25519 signature over [`canonical_message_bytes`].
    pub sig: String,
    /// Author-stamped send time (epoch-ms) — part of the signed payload (not the hub's `ts`).
    pub ts: i64,
    /// Author-chosen unique id for this message — part of the signed payload, and the idempotency key.
    pub uid: String,
    /// The routing target the author addressed (its intent), covered by the signature.
    pub target: Target,
}

impl MessageSig {
    /// Encode as the `com.parler.sig` extension [`Part`].
    pub fn to_part(&self) -> Part {
        let mut fields = serde_json::Map::new();
        fields.insert("sig".into(), serde_json::Value::String(self.sig.clone()));
        fields.insert("alg".into(), serde_json::Value::String("ed25519".into()));
        fields.insert("v".into(), serde_json::json!(MESSAGE_SIG_V));
        fields.insert("ts".into(), serde_json::json!(self.ts));
        fields.insert("uid".into(), serde_json::Value::String(self.uid.clone()));
        fields.insert(
            "target".into(),
            serde_json::to_value(&self.target).unwrap_or(serde_json::Value::Null),
        );
        Part::Extension { kind: MESSAGE_SIG_KIND.to_string(), fields }
    }

    /// Recover the [`MessageSig`] from a message's parts — `Some` iff exactly one well-formed
    /// `com.parler.sig` part is present.
    pub fn from_parts(parts: &[Part]) -> Option<MessageSig> {
        parts.iter().find_map(|p| match p {
            Part::Extension { kind, fields } if kind == MESSAGE_SIG_KIND => Some(MessageSig {
                sig: fields.get("sig")?.as_str()?.to_string(),
                ts: fields.get("ts")?.as_i64()?,
                uid: fields.get("uid")?.as_str()?.to_string(),
                target: serde_json::from_value(fields.get("target")?.clone()).ok()?,
            }),
            _ => None,
        })
    }
}

/// The canonical bytes an author signs (and a verifier reconstructs) for a message.
///
/// Deterministic, whitespace-free, recursively key-sorted JSON (the same JCS-style form as
/// [`canonical_card_bytes`]) over `{v, from, target, parts, replyTo?, ts, uid}`. Any
/// [`MESSAGE_SIG_KIND`] part is filtered out first, so passing the full `parts` (signature included)
/// reproduces the exact bytes the author signed — a verifier and the signer can't disagree on framing.
pub fn canonical_message_bytes(
    from_id: &str,
    target: &Target,
    parts: &[Part],
    reply_to: Option<&str>,
    ts: i64,
    uid: &str,
) -> Vec<u8> {
    let signable: Vec<&Part> = parts.iter().filter(|p| !is_message_sig_part(p)).collect();
    let mut obj = serde_json::Map::new();
    obj.insert("v".into(), serde_json::json!(MESSAGE_SIG_V));
    obj.insert("from".into(), serde_json::Value::String(from_id.to_string()));
    obj.insert("target".into(), serde_json::to_value(target).unwrap_or(serde_json::Value::Null));
    obj.insert("parts".into(), serde_json::to_value(&signable).unwrap_or(serde_json::Value::Null));
    if let Some(rt) = reply_to {
        obj.insert("replyTo".into(), serde_json::Value::String(rt.to_string()));
    }
    obj.insert("ts".into(), serde_json::json!(ts));
    obj.insert("uid".into(), serde_json::Value::String(uid.to_string()));
    serde_json::to_vec(&canonicalize(&serde_json::Value::Object(obj))).unwrap_or_default()
}

/// A structured turn handoff carried inside a room message as a [`Part::Extension`] of kind
/// [`HANDOFF_KIND`] — the explicit "you're up next" signal between agents sharing a room.
///
/// Parler Protocol delivers it like any other part (room / cursor / push / durability all unchanged), but the
/// typed shape lets a worker loop or MCP host *recognise* a handoff addressed to it and continue
/// without a human re-prompting. Pair it with `recv --watch` / `parler_recv wait_secs` for the
/// wakeup. A client that doesn't understand the kind still sees a renderable extension part.
///
/// Build one with [`HandoffRef::to_part`]; recover it with [`HandoffRef::from_part`]; ask whether it
/// is meant for a given agent with [`HandoffRef::is_for`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffRef {
    /// What the next agent should do — the actual instruction to act on.
    pub next: String,
    /// A recap of what was just completed / the current state, so the next agent has context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The addressee: a target agent **name or role**. Absent means "any agent in the room".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Optional content id of an attached code bundle (a [`BundleRef::blob`]) handed off alongside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
}

impl HandoffRef {
    /// Encode as the `com.parler.handoff` extension [`Part`].
    pub fn to_part(&self) -> Part {
        let fields = match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        Part::Extension { kind: HANDOFF_KIND.to_string(), fields }
    }

    /// Recover a [`HandoffRef`] from a part — `Some` iff it is a well-formed `com.parler.handoff`
    /// extension.
    pub fn from_part(part: &Part) -> Option<HandoffRef> {
        match part {
            Part::Extension { kind, fields } if kind == HANDOFF_KIND => {
                serde_json::from_value(serde_json::Value::Object(fields.clone())).ok()
            }
            _ => None,
        }
    }

    /// Whether this handoff is for the agent with the given `name` / optional `role`.
    ///
    /// An unaddressed handoff (`to` absent) is for everyone. An addressed one matches
    /// case-insensitively against either the name or the role, so `--for webdev` reaches an agent
    /// named `webdev` *or* one whose role is `webdev`.
    pub fn is_for(&self, name: &str, role: Option<&str>) -> bool {
        match &self.to {
            None => true,
            Some(addr) => {
                let addr = addr.trim();
                addr.eq_ignore_ascii_case(name)
                    || role.is_some_and(|r| addr.eq_ignore_ascii_case(r))
            }
        }
    }
}

/// A role-addressed work request carried alongside the human-readable task parts in a service room.
///
/// The hub does not need to rewrite or interpret this extension: it persists it verbatim, while an
/// available worker uses the separate [`ClientFrame::Claim`] operation to atomically own the stored
/// message. This keeps the authored request signed end-to-end and makes the queue upgrade additive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchRef {
    /// The role/service that should execute this request (for example `reviewer`).
    pub role: String,
}

impl DispatchRef {
    /// Encode as the `com.parler.dispatch` extension [`Part`].
    pub fn to_part(&self) -> Part {
        let fields = match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        Part::Extension { kind: DISPATCH_KIND.to_string(), fields }
    }

    /// Recover a role dispatch from one extension part.
    pub fn from_part(part: &Part) -> Option<DispatchRef> {
        match part {
            Part::Extension { kind, fields } if kind == DISPATCH_KIND => {
                serde_json::from_value(serde_json::Value::Object(fields.clone())).ok()
            }
            _ => None,
        }
    }
}

/// Where a dispatched unit of work stands in its lifecycle.
///
/// Borrowed from ACP's run state machine (`created → in-progress → awaiting → completed/failed/
/// cancelled`) and collapsed onto Parler Protocol's chat model: a status update is just a message part,
/// so it rides the ordinary room / cursor / durability machinery with **no new wire frame**. The
/// values are the wire contract — add to them, never rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// The worker accepted the task and will start it.
    Accepted,
    /// The worker is actively executing the task.
    Working,
    /// Paused — the worker needs input/approval before it can continue (the `note` is the question).
    Awaiting,
    /// Finished successfully (a `result` blob id may accompany it).
    Done,
    /// Ended in failure (the `note` is why).
    Failed,
    /// Abandoned before completion (by the worker or on request).
    Cancelled,
}

impl TaskStatus {
    /// The lowercase wire/label name (matches the serialized form).
    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::Accepted => "accepted",
            TaskStatus::Working => "working",
            TaskStatus::Awaiting => "awaiting",
            TaskStatus::Done => "done",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    /// A one-glyph marker for rendering a status line.
    pub fn marker(self) -> &'static str {
        match self {
            TaskStatus::Accepted => "🟢",
            TaskStatus::Working => "🔧",
            TaskStatus::Awaiting => "⏳",
            TaskStatus::Done => "✅",
            TaskStatus::Failed => "❌",
            TaskStatus::Cancelled => "🚫",
        }
    }

    /// A terminal status expects no further updates for the task (it's a *receipt*).
    pub fn is_terminal(self) -> bool {
        matches!(self, TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled)
    }

    /// Parse a case-insensitive status name (CLI/MCP input) — `None` if unrecognized.
    pub fn parse(s: &str) -> Option<TaskStatus> {
        match s.trim().to_ascii_lowercase().as_str() {
            "accepted" => Some(TaskStatus::Accepted),
            "working" => Some(TaskStatus::Working),
            "awaiting" => Some(TaskStatus::Awaiting),
            "done" => Some(TaskStatus::Done),
            "failed" => Some(TaskStatus::Failed),
            "cancelled" | "canceled" => Some(TaskStatus::Cancelled),
            _ => None,
        }
    }

    /// Every status name, for CLI/MCP help and validation errors.
    pub const ALL: [&'static str; 6] =
        ["accepted", "working", "awaiting", "done", "failed", "cancelled"];
}

/// A task status update carried inside a room message as a [`Part::Extension`] of kind [`TASK_KIND`].
///
/// Parler Protocol delivers it like any other part, but the typed shape lets a dispatcher (or a human
/// watching a service queue) *recognise* where a unit of work stands without a human reading prose —
/// the missing observability over the fire-and-hope `serve`/`send --service` flow. A terminal update
/// ([`TaskStatus::is_terminal`]) is a **receipt**: because every message is already signed
/// ([`MESSAGE_SIG_KIND`]), a signed `done`/`failed` is a verifiable record, and its optional
/// `tokens`/`elapsed_ms` are the raw material a hub can aggregate into per-agent directory telemetry
/// (derived from real receipts, never self-reported averages). A client that doesn't understand the
/// kind still sees a renderable extension part.
///
/// Build one with [`TaskRef::to_part`]; recover it with [`TaskRef::from_part`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRef {
    /// Where the work stands.
    pub status: TaskStatus,
    /// Correlates updates to one unit of work — the originating request's message id, or a client-
    /// chosen id. Absent ⇒ a standalone status not tied to a prior request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// A human one-liner: what's happening / why it failed / the question when `awaiting`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// A content-addressed result handed back with a terminal `done` — a blob id (see [`FileRef`] /
    /// [`BundleRef`]) the requester pulls with [`ClientFrame::GetBlob`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Estimated model tokens this unit of work consumed (terminal receipts) — raw material for
    /// hub-derived directory telemetry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    /// Wall-clock milliseconds the unit of work took (terminal receipts).
    #[serde(default, rename = "elapsedMs", skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

impl TaskRef {
    /// A minimal status update (no correlation id, note, or receipt fields).
    pub fn new(status: TaskStatus) -> TaskRef {
        TaskRef { status, task: None, note: None, result: None, tokens: None, elapsed_ms: None }
    }

    /// Encode as the `com.parler.task` extension [`Part`].
    pub fn to_part(&self) -> Part {
        let fields = match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        Part::Extension { kind: TASK_KIND.to_string(), fields }
    }

    /// Recover a [`TaskRef`] from a part — `Some` iff it is a well-formed `com.parler.task` extension.
    pub fn from_part(part: &Part) -> Option<TaskRef> {
        match part {
            Part::Extension { kind, fields } if kind == TASK_KIND => {
                serde_json::from_value(serde_json::Value::Object(fields.clone())).ok()
            }
            _ => None,
        }
    }
}

/// The canonical byte encoding of an [`AgentCard`] for signing/verification.
///
/// Produces a deterministic, whitespace-free JSON with **recursively key-sorted** objects (an
/// RFC 8785 / JCS-style canonical form, robust even if `serde_json` is built with `preserve_order`).
/// Both the signer (the agent) and the verifier (the hub, or any client) feed these exact bytes to
/// the nkey sign/verify so a card cannot be silently altered after signing.
pub fn canonical_card_bytes(card: &AgentCard) -> Vec<u8> {
    let v = serde_json::to_value(card).unwrap_or(serde_json::Value::Null);
    serde_json::to_vec(&canonicalize(&v)).unwrap_or_default()
}

/// Recursively rebuild a JSON value with object keys in sorted order.
fn canonicalize(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut sorted = serde_json::Map::with_capacity(m.len());
            for k in keys {
                sorted.insert(k.clone(), canonicalize(&m[k]));
            }
            Value::Object(sorted)
        }
        Value::Array(a) => Value::Array(a.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentSkill, EndpointKind};
    use std::collections::BTreeMap;

    fn sample_card() -> AgentCard {
        let mut meta = BTreeMap::new();
        meta.insert("zone".to_string(), serde_json::json!("us-east"));
        meta.insert("region".to_string(), serde_json::json!("global"));
        AgentCard {
            id: "UABC".into(),
            name: "alice".into(),
            kind: EndpointKind::Agent,
            role: Some("planner".into()),
            description: Some("plans things".into()),
            tags: Some(vec!["planning".into(), "ops".into()]),
            skills: Some(vec![AgentSkill {
                id: "plan".into(),
                name: "Planning".into(),
                description: None,
            }]),
            meta: Some(meta),
            protocol_version: Some("0.2".into()),
        }
    }

    #[test]
    fn canonical_card_bytes_is_deterministic_and_key_sorted() {
        let card = sample_card();
        let a = canonical_card_bytes(&card);
        let b = canonical_card_bytes(&card.clone());
        assert_eq!(a, b, "canonicalization must be stable");
        let s = String::from_utf8(a).unwrap();
        // Object keys are sorted: `description` precedes `id` precedes `name`; nested meta `region`
        // precedes `zone`. And there is no insignificant whitespace.
        assert!(s.find("\"description\"").unwrap() < s.find("\"id\"").unwrap());
        assert!(s.find("\"region\"").unwrap() < s.find("\"zone\"").unwrap());
        assert!(!s.contains(": "));
    }

    #[test]
    fn directory_entry_round_trips_camelcase() {
        let entry = DirectoryEntry {
            card: sample_card(),
            visibility: Visibility::Public,
            status: "working".into(),
            activity: Some("planning the sprint".into()),
            attention: Some(Attention::Focus),
            hub: "Parler Protocol Public".into(),
            verified: true,
            sig: Some("AAAA".into()),
            first_seen: 10,
            last_seen: 20,
        };
        let j = serde_json::to_value(&entry).unwrap();
        assert_eq!(j["visibility"], "public");
        assert_eq!(j["verified"], true);
        assert_eq!(j["firstSeen"], 10);
        assert_eq!(j["lastSeen"], 20);
        assert_eq!(j["card"]["id"], "UABC");
        let back: DirectoryEntry = serde_json::from_value(j).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn discovery_frames_round_trip() {
        let reg = ClientFrame::Register {
            card: sample_card(),
            visibility: Visibility::Public,
            sig: Some("SIG".into()),
        };
        let j = serde_json::to_value(&reg).unwrap();
        assert_eq!(j["op"], "register");
        assert_eq!(j["visibility"], "public");
        assert_eq!(serde_json::from_value::<ClientFrame>(j).unwrap(), reg);

        let disc = ClientFrame::Discover {
            scope: DiscoverScope::Public,
            query: Some("plan".into()),
            tag: None,
            skill: Some("review".into()),
            status: None,
            limit: Some(20),
        };
        let j = serde_json::to_value(&disc).unwrap();
        assert_eq!(j["op"], "discover");
        assert_eq!(j["scope"], "public");
        assert_eq!(serde_json::from_value::<ClientFrame>(j).unwrap(), disc);
    }

    #[test]
    fn visibility_defaults_to_private() {
        assert_eq!(Visibility::default(), Visibility::Private);
        // An absent `visibility`/`scope` deserializes to the secure-by-default values.
        let reg: ClientFrame =
            serde_json::from_value(serde_json::json!({"op":"register","card":sample_card()}))
                .unwrap();
        match reg {
            ClientFrame::Register { visibility, .. } => assert_eq!(visibility, Visibility::Private),
            _ => panic!("expected register"),
        }
        let disc: ClientFrame =
            serde_json::from_value(serde_json::json!({"op":"discover"})).unwrap();
        match disc {
            ClientFrame::Discover { scope, .. } => assert_eq!(scope, DiscoverScope::Hub),
            _ => panic!("expected discover"),
        }
    }

    #[test]
    fn client_frame_round_trips_with_op_tag() {
        let f = ClientFrame::Send {
            target: Target::Dm {
                agent: "UABC".into(),
            },
            parts: vec![Part::text("hi")],
            mentions: None,
            reply_to: None,
            client_id: None,
        };
        let j = serde_json::to_value(&f).unwrap();
        assert_eq!(j["op"], "send");
        assert_eq!(j["target"]["kind"], "dm");
        assert_eq!(j["target"]["agent"], "UABC");
        assert_eq!(j["parts"][0]["kind"], "text");
        // Absent client_id must not appear on the wire (old-client byte-compatibility, #86).
        assert!(j.get("clientId").is_none(), "unset client_id is omitted");
        let back: ClientFrame = serde_json::from_value(j).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn recall_key_is_optional_and_omitted_when_absent() {
        // #91: a keyed Recall round-trips and carries `key`; an unset key never hits the wire, so an
        // old hub sees exactly the bytes it saw before.
        let keyed = ClientFrame::Recall {
            query: "SESSION DIGEST".into(),
            room: Some("team".into()),
            limit: Some(1),
            embedding: None,
            key: Some("session-digest".into()),
        };
        let j = serde_json::to_value(&keyed).unwrap();
        assert_eq!(j["op"], "recall");
        assert_eq!(j["key"], "session-digest");
        assert_eq!(serde_json::from_value::<ClientFrame>(j).unwrap(), keyed);

        // Unset key is omitted (old-client byte-compatibility), and an old-client payload with no
        // `key` field still deserializes (defaults to None).
        let plain = ClientFrame::Recall { query: "deploy".into(), room: None, limit: None, embedding: None, key: None };
        let jp = serde_json::to_value(&plain).unwrap();
        assert!(jp.get("key").is_none(), "unset key is omitted from the wire");
        let from_old: ClientFrame = serde_json::from_str(r#"{"op":"recall","query":"deploy"}"#).unwrap();
        assert_eq!(from_old, plain);
    }

    #[test]
    fn server_frame_round_trips_with_type_tag() {
        let f = ServerFrame::Invited {
            code: "AB12CD34".into(),
            url: "parler://127.0.0.1:7070/join/AB12CD34".into(),
            room: "dm.x".into(),
            kind: RoomKind::Dm,
            expires_at: 123,
        };
        let j = serde_json::to_value(&f).unwrap();
        assert_eq!(j["type"], "invited");
        assert_eq!(j["kind"], "dm");
        assert_eq!(j["expiresAt"], 123);
        let back: ServerFrame = serde_json::from_value(j).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn approval_invite_field_defaults_and_round_trips() {
        // Absent on the wire ⇒ `require_approval` deserializes to false (back-compat with old peers),
        // and a false value is omitted when re-serialized (clean wire).
        let open: ClientFrame =
            serde_json::from_value(serde_json::json!({"op":"invite","kind":"channel"})).unwrap();
        match &open {
            ClientFrame::Invite { require_approval, .. } => assert!(!require_approval),
            _ => panic!("expected invite"),
        }
        assert!(serde_json::to_value(&open).unwrap().get("requireApproval").is_none());

        // When set it survives the round trip under its camelCase wire name.
        let gated = ClientFrame::Invite {
            kind: RoomKind::Channel,
            room: Some("design".into()),
            ttl_secs: None,
            max_uses: None,
            require_approval: true,
        };
        let j = serde_json::to_value(&gated).unwrap();
        assert_eq!(j["requireApproval"], true);
        assert_eq!(serde_json::from_value::<ClientFrame>(j).unwrap(), gated);
    }

    #[test]
    fn join_approval_frames_round_trip() {
        let resolve = ClientFrame::ResolveJoin {
            room: "room.x".into(),
            agent: "UBOB".into(),
            approve: true,
        };
        let j = serde_json::to_value(&resolve).unwrap();
        assert_eq!(j["op"], "resolve_join");
        assert_eq!(j["approve"], true);
        assert_eq!(serde_json::from_value::<ClientFrame>(j).unwrap(), resolve);

        let pending = ServerFrame::JoinPending { room: "room.x".into() };
        assert_eq!(serde_json::to_value(&pending).unwrap()["type"], "join_pending");

        let reqs = ServerFrame::JoinRequests {
            room: "room.x".into(),
            requests: vec![JoinRequest {
                agent: "UBOB".into(),
                name: "bob".into(),
                role: Some("reviewer".into()),
                requested_at: 42,
            }],
        };
        let j = serde_json::to_value(&reqs).unwrap();
        assert_eq!(j["type"], "join_requests");
        assert_eq!(j["requests"][0]["agent"], "UBOB");
        assert_eq!(j["requests"][0]["requestedAt"], 42);
        assert_eq!(serde_json::from_value::<ServerFrame>(j).unwrap(), reqs);
    }

    #[test]
    fn watch_frames_round_trip() {
        // Mint request: camelCase ttlSecs, omitted when absent.
        let mint = ClientFrame::MintWatch { room: "room.abc".into(), ttl_secs: Some(3600) };
        let j = serde_json::to_value(&mint).unwrap();
        assert_eq!(j["op"], "mint_watch");
        assert_eq!(j["room"], "room.abc");
        assert_eq!(j["ttlSecs"], 3600);
        assert_eq!(serde_json::from_value::<ClientFrame>(j).unwrap(), mint);

        let bare: ClientFrame =
            serde_json::from_value(serde_json::json!({"op":"mint_watch","room":"r"})).unwrap();
        assert!(matches!(bare, ClientFrame::MintWatch { ttl_secs: None, .. }));

        // Reply.
        let w = ServerFrame::Watch { token: "TOK".into(), room: "room.abc".into(), expires_at: 99 };
        let j = serde_json::to_value(&w).unwrap();
        assert_eq!(j["type"], "watch");
        assert_eq!(j["token"], "TOK");
        assert_eq!(j["expiresAt"], 99);
        assert_eq!(serde_json::from_value::<ServerFrame>(j).unwrap(), w);
    }

    #[test]
    fn blob_frames_round_trip() {
        let put = ClientFrame::PutBlob {
            target: Target::Room { room: "dev".into() },
            sha256: "abc".into(),
            size: 10,
            media_type: Some("application/x-git-bundle".into()),
        };
        let j = serde_json::to_value(&put).unwrap();
        assert_eq!(j["op"], "put_blob");
        assert_eq!(j["target"]["kind"], "room");
        assert_eq!(j["mediaType"], "application/x-git-bundle");
        assert_eq!(serde_json::from_value::<ClientFrame>(j).unwrap(), put);

        let get = ClientFrame::GetBlob { id: "abc".into() };
        let j = serde_json::to_value(&get).unwrap();
        assert_eq!(j["op"], "get_blob");
        assert_eq!(serde_json::from_value::<ClientFrame>(j).unwrap(), get);

        let inc = ServerFrame::BlobIncoming { id: "abc".into(), size: 10, media_type: None };
        let j = serde_json::to_value(&inc).unwrap();
        assert_eq!(j["type"], "blob_incoming");
        assert!(j.get("mediaType").is_none());
        assert_eq!(serde_json::from_value::<ServerFrame>(j).unwrap(), inc);
    }

    #[test]
    fn bundle_ref_round_trips_through_a_part() {
        let b = BundleRef {
            blob: "abc123".into(),
            vcs: "git".into(),
            tip: Some("deadbeef".into()),
            base: Some("cafe".into()),
            summary: Some("feat: x".into()),
            size: 99,
            media_type: Some("application/x-git-bundle".into()),
        };
        let part = b.to_part();
        match &part {
            Part::Extension { kind, .. } => assert_eq!(kind, BUNDLE_KIND),
            _ => panic!("expected an extension part"),
        }
        // Survives a JSON wire round-trip as a Part, camelCase `mediaType` intact…
        let j = serde_json::to_value(&part).unwrap();
        assert_eq!(j["kind"], BUNDLE_KIND);
        assert_eq!(j["blob"], "abc123");
        assert_eq!(j["mediaType"], "application/x-git-bundle");
        let back: Part = serde_json::from_value(j).unwrap();
        assert_eq!(BundleRef::from_part(&back), Some(b));
        // …and a plain part is not a bundle ref.
        assert_eq!(BundleRef::from_part(&Part::text("hi")), None);
    }

    #[test]
    fn file_ref_round_trips_through_a_part() {
        let f = FileRef {
            blob: "abc123".into(),
            name: "report.pdf".into(),
            size: 4096,
            media_type: Some("application/pdf".into()),
            summary: Some("Q3 numbers".into()),
        };
        let part = f.to_part();
        match &part {
            Part::Extension { kind, .. } => assert_eq!(kind, FILE_KIND),
            _ => panic!("expected an extension part"),
        }
        // Survives a JSON wire round-trip as a Part, camelCase `mediaType` intact…
        let j = serde_json::to_value(&part).unwrap();
        assert_eq!(j["kind"], FILE_KIND);
        assert_eq!(j["name"], "report.pdf");
        assert_eq!(j["mediaType"], "application/pdf");
        let back: Part = serde_json::from_value(j).unwrap();
        assert_eq!(FileRef::from_part(&back), Some(f));
        // …and neither a plain part nor a sibling bundle part is a file ref.
        assert_eq!(FileRef::from_part(&Part::text("hi")), None);
        let bundle_part = BundleRef {
            blob: "x".into(),
            vcs: "git".into(),
            tip: None,
            base: None,
            summary: None,
            size: 1,
            media_type: None,
        }
        .to_part();
        assert_eq!(FileRef::from_part(&bundle_part), None);
    }

    #[test]
    fn task_ref_round_trips_through_a_part() {
        // A terminal receipt with the optional telemetry fields present.
        let t = TaskRef {
            status: TaskStatus::Done,
            task: Some("review-42".into()),
            note: Some("LGTM, shipped".into()),
            result: Some("deadbeef".into()),
            tokens: Some(1234),
            elapsed_ms: Some(5000),
        };
        let part = t.to_part();
        match &part {
            Part::Extension { kind, .. } => assert_eq!(kind, TASK_KIND),
            _ => panic!("expected an extension part"),
        }
        let j = serde_json::to_value(&part).unwrap();
        assert_eq!(j["kind"], TASK_KIND);
        assert_eq!(j["status"], "done"); // lowercase enum on the wire
        assert_eq!(j["elapsedMs"], 5000); // camelCase rename intact
        let back: Part = serde_json::from_value(j).unwrap();
        assert_eq!(TaskRef::from_part(&back), Some(t));

        // A minimal update omits every optional field on the wire (byte-lean, old-client safe).
        let minimal = TaskRef::new(TaskStatus::Working);
        let j = serde_json::to_value(minimal.to_part()).unwrap();
        assert_eq!(j["status"], "working");
        for k in ["task", "note", "result", "tokens", "elapsedMs"] {
            assert!(j.get(k).is_none(), "minimal task update must omit `{k}`, got {j}");
        }

        // Status classification + parsing (CLI/MCP input).
        assert!(TaskStatus::Done.is_terminal() && !TaskStatus::Working.is_terminal());
        assert_eq!(TaskStatus::parse("DONE"), Some(TaskStatus::Done));
        assert_eq!(TaskStatus::parse("canceled"), Some(TaskStatus::Cancelled)); // both spellings
        assert_eq!(TaskStatus::parse("nope"), None);

        // A sibling handoff part is not a task ref, and a plain part isn't either.
        let handoff = HandoffRef { next: "go".into(), summary: None, to: None, bundle: None }.to_part();
        assert_eq!(TaskRef::from_part(&handoff), None);
        assert_eq!(TaskRef::from_part(&Part::text("hi")), None);
    }

    #[test]
    fn error_frame_carries_an_optional_code_over_the_wire() {
        // A coded error serializes its stable classifier; an uncoded one omits `code` entirely so an
        // old client (and an old hub that never sets it) stay byte-compatible.
        let coded = ServerFrame::error_coded(error_code::NOT_MEMBER, "not a member of 'x'");
        let j = serde_json::to_value(&coded).unwrap();
        assert_eq!(j["type"], "error");
        assert_eq!(j["message"], "not a member of 'x'");
        assert_eq!(j["code"], "not_member");

        let plain = ServerFrame::error("something opaque broke");
        let j = serde_json::to_value(&plain).unwrap();
        assert_eq!(j["message"], "something opaque broke");
        assert!(j.get("code").is_none(), "an uncoded error must omit `code`, got {j}");

        // Both survive a full wire round-trip with the code preserved / still absent.
        match serde_json::from_value::<ServerFrame>(serde_json::to_value(&coded).unwrap()).unwrap() {
            ServerFrame::Error { message, code } => {
                assert_eq!(message, "not a member of 'x'");
                assert_eq!(code.as_deref(), Some(error_code::NOT_MEMBER));
            }
            other => panic!("expected an error frame, got {other:?}"),
        }
    }

    #[test]
    fn coded_error_displays_as_just_the_message() {
        // Display == message, so a CodedError prints and `.to_string()`s exactly like the plain
        // `anyhow!` it replaces — nothing that only reads the text is affected. Downcast still recovers
        // the classifier (the `anyhow` path is exercised end-to-end in the connector's e2e suite).
        let e = CodedError::new(error_code::RATE_LIMITED, "slow down");
        assert_eq!(e.to_string(), "slow down");
        assert_eq!(e.code.as_deref(), Some("rate_limited"));
        let boxed: Box<dyn std::error::Error> = Box::new(e);
        assert_eq!(boxed.to_string(), "slow down");
        assert_eq!(
            boxed.downcast_ref::<CodedError>().and_then(|c| c.code.as_deref()),
            Some("rate_limited")
        );
    }

    #[test]
    fn message_sig_part_round_trips_and_ignores_non_sig_parts() {
        let ms = MessageSig {
            sig: "BASE64SIG".into(),
            ts: 1710000000000,
            uid: "018f-uid".into(),
            target: Target::Room { room: "team".into() },
        };
        let part = ms.to_part();
        assert!(is_message_sig_part(&part));
        // Survives a JSON wire round-trip as a Part…
        let j = serde_json::to_value(&part).unwrap();
        assert_eq!(j["kind"], MESSAGE_SIG_KIND);
        assert_eq!(j["alg"], "ed25519");
        assert_eq!(j["target"]["kind"], "room");
        let back: Part = serde_json::from_value(j).unwrap();
        // …recovered from a parts list that also holds ordinary parts.
        let parts = vec![Part::text("hi"), back];
        assert_eq!(MessageSig::from_parts(&parts), Some(ms));
        // A list with no sig part yields None.
        assert_eq!(MessageSig::from_parts(&[Part::text("hi")]), None);
    }

    #[test]
    fn canonical_message_bytes_is_stable_and_sig_part_independent() {
        let target = Target::Dm { agent: "UBOB".into() };
        let parts = vec![Part::text("ship it")];
        let a = canonical_message_bytes("UALICE", &target, &parts, Some("m0"), 42, "uid1");
        // Recomputing with the sig part appended must yield identical bytes (it's filtered out), so a
        // verifier that passes the full received `parts` reconstructs exactly what the author signed.
        let ms = MessageSig { sig: "x".into(), ts: 42, uid: "uid1".into(), target: target.clone() };
        let mut with_sig = parts.clone();
        with_sig.push(ms.to_part());
        let b = canonical_message_bytes("UALICE", &target, &with_sig, Some("m0"), 42, "uid1");
        assert_eq!(a, b, "the sig part must not feed back into the signed bytes");
        // Any covered field changing the payload changes the bytes.
        let c = canonical_message_bytes("UALICE", &target, &[Part::text("ship them")], Some("m0"), 42, "uid1");
        assert_ne!(a, c);
        let d = canonical_message_bytes("UMALLORY", &target, &parts, Some("m0"), 42, "uid1");
        assert_ne!(a, d, "the author id is part of the signed payload");
    }

    #[test]
    fn handoff_ref_round_trips_and_addresses() {
        let h = HandoffRef {
            next: "build the page structure".into(),
            summary: Some("design direction is locked".into()),
            to: Some("webdev".into()),
            bundle: Some("blobsha".into()),
        };
        let part = h.to_part();
        match &part {
            Part::Extension { kind, .. } => assert_eq!(kind, HANDOFF_KIND),
            _ => panic!("expected an extension part"),
        }
        // Survives a JSON wire round-trip as a Part.
        let j = serde_json::to_value(&part).unwrap();
        assert_eq!(j["kind"], HANDOFF_KIND);
        assert_eq!(j["next"], "build the page structure");
        let back: Part = serde_json::from_value(j).unwrap();
        assert_eq!(HandoffRef::from_part(&back), Some(h.clone()));
        // …and a plain part is not a handoff ref.
        assert_eq!(HandoffRef::from_part(&Part::text("hi")), None);

        // Addressing: matches the agent's name or role, case-insensitively.
        assert!(h.is_for("webdev", None));
        assert!(h.is_for("bob", Some("WebDev")));
        assert!(!h.is_for("designer", Some("planner")));
        // An unaddressed handoff is for everyone.
        let any = HandoffRef { to: None, ..h };
        assert!(any.is_for("anyone", None));

        // Optional fields stay absent on the wire when None.
        let minimal = HandoffRef { next: "go".into(), summary: None, to: None, bundle: None };
        let j = serde_json::to_value(minimal.to_part()).unwrap();
        assert!(j.get("summary").is_none());
        assert!(j.get("to").is_none());
        assert!(j.get("bundle").is_none());
    }

    #[test]
    fn unit_variants_serialize_as_tag_only() {
        assert_eq!(serde_json::to_value(ClientFrame::Ping).unwrap()["op"], "ping");
        assert_eq!(serde_json::to_value(ClientFrame::Rooms).unwrap()["op"], "rooms");
        assert_eq!(
            serde_json::to_value(ClientFrame::DeleteRoom { room: "team".into() }).unwrap()["op"],
            "delete_room"
        );
        assert_eq!(serde_json::to_value(ClientFrame::Subscribe).unwrap()["op"], "subscribe");
        assert_eq!(
            serde_json::to_value(ServerFrame::PresenceOk).unwrap()["type"],
            "presence_ok"
        );
        assert_eq!(
            serde_json::to_value(ServerFrame::RoomDeleted { room: "team".into() }).unwrap()["type"],
            "room_deleted"
        );
        assert_eq!(serde_json::to_value(ServerFrame::Subscribed).unwrap()["type"], "subscribed");
    }

    #[test]
    fn push_delivery_frame_round_trips() {
        let msg = StoredMessage {
            seq: 42,
            id: "0190-msg".into(),
            room: "team".into(),
            from: EndpointRef { id: "UALICE".into(), name: "alice".into(), role: Some("planner".into()) },
            parts: vec![Part::text("live ping")],
            mentions: Some(vec!["UBOB".into()]),
            reply_to: None,
            ts: 1700,
        };
        let f = ServerFrame::Delivery { message: msg.clone() };
        let j = serde_json::to_value(&f).unwrap();
        assert_eq!(j["type"], "delivery");
        assert_eq!(j["message"]["seq"], 42);
        assert_eq!(j["message"]["room"], "team");
        assert_eq!(j["message"]["parts"][0]["kind"], "text");
        let back: ServerFrame = serde_json::from_value(j).unwrap();
        assert_eq!(back, ServerFrame::Delivery { message: msg });
    }

    #[test]
    fn pull_without_wait_secs_is_byte_identical_on_the_wire() {
        // Backward-compat guarantee: a `Pull` with no `wait_secs` serializes exactly as before the
        // field existed — the `waitSecs` key is omitted entirely, so an old hub/client sees the same
        // bytes. (The field is `skip_serializing_if = "Option::is_none"`.)
        let old_shape =
            ClientFrame::Pull { room: "r".into(), since: None, limit: Some(30), wait_secs: None, ack: None };
        let j = serde_json::to_value(&old_shape).unwrap();
        assert_eq!(j["op"], "pull");
        assert_eq!(j["room"], "r");
        assert_eq!(j["limit"], 30);
        assert!(j.get("waitSecs").is_none(), "wait_secs=None must not appear on the wire");
        assert!(j.get("since").is_none(), "since=None still omitted as before");
        assert!(j.get("ack").is_none(), "ack=None must not appear on the wire (old-hub compat, #85)");
        // A frame from before the field existed (no `waitSecs` key) deserializes with `wait_secs: None`.
        let legacy = serde_json::json!({ "op": "pull", "room": "r", "limit": 30 });
        let back: ClientFrame = serde_json::from_value(legacy).unwrap();
        assert_eq!(back, old_shape);
    }

    #[test]
    fn pull_with_wait_secs_uses_camel_case_key() {
        // The new long-poll field is present + camelCased only when set (matching the crate's wire
        // convention, e.g. `replyTo`/`ttlSecs`).
        let waited =
            ClientFrame::Pull { room: "r".into(), since: None, limit: None, wait_secs: Some(30), ack: None };
        let j = serde_json::to_value(&waited).unwrap();
        assert_eq!(j["waitSecs"], 30);
        let back: ClientFrame = serde_json::from_value(j).unwrap();
        assert_eq!(back, waited);
    }

    #[test]
    fn estimate_tokens_is_ceil_quarter_of_unicode_chars() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1); // ceil(1/4)
        assert_eq!(estimate_tokens("abcd"), 1); // 4 chars → 1
        assert_eq!(estimate_tokens("abcde"), 2); // ceil(5/4) = 2
        // Unicode scalar values, not bytes: 4 emoji are 4 "chars" → 1 token, not 16 bytes → 4.
        assert_eq!(estimate_tokens("😀😀😀😀"), 1);
    }

    #[test]
    fn estimate_message_tokens_sums_text_and_extension_strings_but_skips_the_sig() {
        // A text part (4 chars) + an observation extension whose string leaves total 8 chars.
        let mut obs = serde_json::Map::new();
        obs.insert("tool_name".into(), serde_json::json!("Bash")); // 4
        obs.insert("tool_output".into(), serde_json::json!("done")); // 4
        obs.insert("exit".into(), serde_json::json!(0)); // non-string ⇒ ignored
        let sig = MessageSig {
            sig: "AAAABBBBCCCCDDDD".into(),
            ts: 1,
            uid: "uid".into(),
            target: Target::Room { room: "team".into() },
        };
        let parts = vec![
            Part::text("abcd"), // 4
            Part::Extension { kind: "com.parler.observation".into(), fields: obs }, // 8
            sig.to_part(),      // excluded — plumbing, not conversation
        ];
        // 4 + 8 = 12 chars → ceil(12/4) = 3. The sig's base64/target must not inflate the count.
        assert_eq!(estimate_message_tokens(&parts), 3);
        // Dropping the sig part changes nothing (it never counted).
        assert_eq!(estimate_message_tokens(&parts[..2]), 3);
    }
}
