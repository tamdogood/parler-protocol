//! parler-protocol::hub — wire frames for the Parler **Hub**, the lightweight "Slack for agents"
//! transport (a WebSocket bus + an embedded durable store).
//!
//! These are the frames an agent's client (`parler-connector`) exchanges with the hub
//! (`parler-hub`). Like the rest of this crate they are pure serde types that perform no IO, so the
//! client and the server share one definition of the protocol. JSON field names that are multi-word
//! are camelCase to match the rest of the Parler envelope.

use crate::types::{EndpointRef, Part};
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
    },
    /// Redeem a pasted code — joins the room it grants.
    Redeem { code: String },
    /// Join/create a service room as a worker, so `Send`/`Pull` on it are authorized.
    Serve { service: String },
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
    Remember { fact: Fact },
    /// Full-text recall from the memory store (scoped to `room` if given, else the agent's reachable
    /// memory: its private facts plus the rooms it belongs to).
    Recall {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        room: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
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
    Ping,
}

/// Frames the hub sends back. Every non-error frame is the direct reply to the client's last op.
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
    Pong,
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn unit_variants_serialize_as_tag_only() {
        assert_eq!(serde_json::to_value(ClientFrame::Ping).unwrap()["op"], "ping");
        assert_eq!(serde_json::to_value(ClientFrame::Rooms).unwrap()["op"], "rooms");
        assert_eq!(
            serde_json::to_value(ServerFrame::PresenceOk).unwrap()["type"],
            "presence_ok"
        );
    }
}
