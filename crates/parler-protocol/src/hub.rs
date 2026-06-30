//! parler-protocol::hub — wire frames for the Parler **Hub**, the lightweight transport behind the
//! chat protocol for AI agents (a WebSocket bus + an embedded durable store).
//!
//! These are the frames an agent's client (`parler-connector`) exchanges with the hub
//! (`parler-hub`). Like the rest of this crate they are pure serde types that perform no IO, so the
//! client and the server share one definition of the protocol. JSON field names that are multi-word
//! are camelCase to match the rest of the Parler envelope.

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
    Send {
        target: Target,
        parts: Vec<Part>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mentions: Option<Vec<String>>,
        #[serde(default, rename = "replyTo", skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
    },
    /// Pull messages for a room newer than the agent's stored cursor (which this advances), or newer
    /// than `since` (which does not advance the cursor — for re-reads).
    Pull {
        room: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
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
    Recall {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        room: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        embedding: Option<Vec<f32>>,
    },
    /// List the rooms the agent belongs to.
    Rooms,
    /// The members + presence of a room.
    Roster { room: String },
    /// Advertise presence.
    Presence {
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activity: Option<String>,
    },
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
    /// Step 1 of the handshake: sign this `nonce` and re-send `Hello`.
    Challenge { nonce: String },
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
    Roster {
        room: String,
        entries: Vec<RosterEntry>,
    },
    PresenceOk,
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
    },
}

/// `skip_serializing_if` helper: omit a `bool` field from the wire when it is `false`, so an
/// approval flag defaults cleanly to absent (and old peers that never set it stay byte-compatible).
fn is_false(b: &bool) -> bool {
    !*b
}

/// The reverse-DNS [`Part`] kind that references a code/artifact bundle handed off through a room.
pub const BUNDLE_KIND: &str = "com.parler.bundle";

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
/// (`seq`, the resolved room name, the hub's own `ts`): those are the relay's to set, and binding the
/// delivered room (anti-misrouting) and ordering (anti-reorder) ride the per-room hash chain layered
/// on top of this. `mentions` are excluded because the hub normalizes them in flight.
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
            hub: "Parler Public".into(),
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
        };
        let j = serde_json::to_value(&f).unwrap();
        assert_eq!(j["op"], "send");
        assert_eq!(j["target"]["kind"], "dm");
        assert_eq!(j["target"]["agent"], "UABC");
        assert_eq!(j["parts"][0]["kind"], "text");
        let back: ClientFrame = serde_json::from_value(j).unwrap();
        assert_eq!(back, f);
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
    fn unit_variants_serialize_as_tag_only() {
        assert_eq!(serde_json::to_value(ClientFrame::Ping).unwrap()["op"], "ping");
        assert_eq!(serde_json::to_value(ClientFrame::Rooms).unwrap()["op"], "rooms");
        assert_eq!(serde_json::to_value(ClientFrame::Subscribe).unwrap()["op"], "subscribe");
        assert_eq!(
            serde_json::to_value(ServerFrame::PresenceOk).unwrap()["type"],
            "presence_ok"
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
}
