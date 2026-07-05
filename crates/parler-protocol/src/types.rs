//! Parler Protocol wire types (v0.2) — the shapes that travel on the mesh.
//!
//! Port of Cotal `packages/core/src/types.ts`. A2A-inspired (AgentCard / Message / Part) but
//! transport-agnostic. This file IS part of the wire contract — treat changes as protocol changes.
//! Wire field names are camelCase to match the JSON envelope exactly.

use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

/// 'agent' (participates in coordination) or a plain 'endpoint' (logger, dashboard…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EndpointKind {
    Agent,
    Endpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A2A-inspired identity record for an endpoint or agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentCard {
    /// Unique, stable for the lifetime of this connection (the agent's nkey public key).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    pub kind: EndpointKind,
    /// The role this participant plays (planner, reviewer, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Free-form "what it can do" tags — discovery only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<AgentSkill>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<BTreeMap<String, serde_json::Value>>,
    /// Wire-contract version this participant speaks. Omitted ⇒ assume the v0.x line.
    #[serde(
        default,
        rename = "protocolVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub protocol_version: Option<String>,
}

/// Lifecycle status of a participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresenceStatus {
    /// connected, no active task
    Idle,
    /// blocked — awaiting input, approval, or a peer
    Waiting,
    /// actively executing a task / in a turn
    Working,
    /// disconnected or heartbeat lapsed (derived by observers, not self-set while live)
    Offline,
}

/// How aggressively peer traffic interrupts an agent — chosen by the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttentionMode {
    Open,
    Dnd,
    Focus,
}

/// Per-channel attention override (more specific than the global [`AttentionMode`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelMode {
    /// still delivered + buffered, but never wakes; an `@`-mention still wakes (per-channel dnd).
    Quiet,
    /// channel messages dropped on receive, incl. `@`-mentions ("don't receive this channel").
    Muted,
}

/// Live presence record. Stored in the space's KV bucket under key = card.id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    pub card: AgentCard,
    pub status: PresenceStatus,
    /// Freeform "what I'm doing right now".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// This instance's current global attention mode (advisory; `open`/absent ⇒ receives all).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<AttentionMode>,
    /// Per-channel attention overrides this instance currently has (runtime, reset on restart).
    #[serde(
        default,
        rename = "channelModes",
        skip_serializing_if = "Option::is_none"
    )]
    pub channel_modes: Option<BTreeMap<String, ChannelMode>>,
    /// Epoch ms of the last heartbeat.
    pub ts: i64,
}

/// A channel's delivery class (SPEC §4). Fixed per channel, wire-observable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryClass {
    /// native broker-subscription delivery; at-most-once.
    Live,
    /// `live` plus a per-subscriber durable backstop; at-least-once for current members.
    Durable,
}

/// Channel registry entry — channel-global config, stored in the per-space channels KV.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<bool>,
    #[serde(
        default,
        rename = "replayWindow",
        skip_serializing_if = "Option::is_none"
    )]
    pub replay_window: Option<String>,
    #[serde(
        default,
        rename = "deliveryClass",
        skip_serializing_if = "Option::is_none"
    )]
    pub delivery_class: Option<DeliveryClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Space-wide channel defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<bool>,
    #[serde(
        default,
        rename = "replayWindow",
        skip_serializing_if = "Option::is_none"
    )]
    pub replay_window: Option<String>,
    #[serde(
        default,
        rename = "deliveryClass",
        skip_serializing_if = "Option::is_none"
    )]
    pub delivery_class: Option<DeliveryClass>,
}

/// Durable-membership state (Plane-3, SPEC §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MembershipState {
    LiveConfirmed,
    DurableActive,
}

/// A durable-membership record (privileged write only). Eligibility is by CHAT stream sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipRecord {
    pub channel: String,
    /// Owner agent id (nkey).
    pub owner: String,
    pub state: MembershipState,
    /// CHAT stream seq captured at join — durable eligibility is `seq > joinCursor`.
    #[serde(rename = "joinCursor")]
    pub join_cursor: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activated: Option<bool>,
    /// CHAT stream seq captured at leave; present ⇒ tombstone.
    #[serde(default, rename = "leaveCursor", skip_serializing_if = "Option::is_none")]
    pub leave_cursor: Option<u64>,
    /// Bumped each (re)join. Stale-write guard + idempotency-key component.
    pub generation: u64,
    /// The privileged writer's id (audit; never an agent).
    #[serde(rename = "writerIdentity")]
    pub writer_identity: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

/// A durable read-ACL record (privileged write only). One per OWNER in the `parler_acl_<space>` KV.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AclRecord {
    /// The owner's current read ACL — the channels/patterns it may read (its `allowSubscribe`).
    #[serde(rename = "allowSubscribe")]
    pub allow_subscribe: Vec<String>,
    pub revision: u64,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

/// A fan-out entry in an owner's mixed pre-auth inbox (`dinbox.<owner>`, Plane-3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plane3Entry {
    pub msg: Message,
    pub channel: String,
    pub seq: u64,
    pub reason: Plane3Reason,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Plane3Reason {
    DurableChannel,
    LiveMention,
}

/// One agent's derived channel-membership record (the broker-authoritative graph feed). Display-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMembership {
    /// Channel-subscription patterns from the broker's live connection view (CONNZ), wildcards kept.
    pub live: Vec<String>,
    /// Concrete durable channels from the privileged members registry.
    pub durable: Vec<String>,
    #[serde(rename = "observedAt")]
    pub observed_at: i64,
}

/// One agent's membership record keyed by its id — for the dashboard snapshot/SSE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipEntry {
    pub id: String,
    #[serde(flatten)]
    pub membership: ChannelMembership,
}

/// The broker-sourced membership feed as the dashboard consumes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipSnapshot {
    #[serde(default, rename = "asOf", skip_serializing_if = "Option::is_none")]
    pub as_of: Option<i64>,
    pub members: Vec<MembershipEntry>,
}

/// A message Part: `text`, `data`, or a reverse-DNS extension kind.
///
/// Serialized as `{ "kind": …, … }`. Hand-written codec: the extension variant carries a dynamic
/// `kind` value with arbitrary sibling fields, which serde's derive can't express as a catch-all.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    Data(serde_json::Value),
    /// Extension: `kind` is reverse-DNS (e.g. `com.acme.snapshot`); `fields` are the siblings.
    Extension {
        kind: String,
        fields: serde_json::Map<String, serde_json::Value>,
    },
}

impl Part {
    pub fn text(s: impl Into<String>) -> Self {
        Part::Text(s.into())
    }
    /// The wire `kind` discriminator of this part.
    pub fn kind(&self) -> &str {
        match self {
            Part::Text(_) => "text",
            Part::Data(_) => "data",
            Part::Extension { kind, .. } => kind,
        }
    }
}

/// A reverse-DNS extension kind: `^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$`.
pub fn is_extension_kind(kind: &str) -> bool {
    let segs: Vec<&str> = kind.split('.').collect();
    segs.len() >= 2
        && segs
            .iter()
            .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
}

impl Serialize for Part {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        match self {
            Part::Text(text) => {
                let mut m = ser.serialize_map(Some(2))?;
                m.serialize_entry("kind", "text")?;
                m.serialize_entry("text", text)?;
                m.end()
            }
            Part::Data(data) => {
                let mut m = ser.serialize_map(Some(2))?;
                m.serialize_entry("kind", "data")?;
                m.serialize_entry("data", data)?;
                m.end()
            }
            Part::Extension { kind, fields } => {
                let mut m = ser.serialize_map(Some(fields.len() + 1))?;
                m.serialize_entry("kind", kind)?;
                for (k, v) in fields {
                    m.serialize_entry(k, v)?;
                }
                m.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Part {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct PartVisitor;
        impl<'de> Visitor<'de> for PartVisitor {
            type Value = Part;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a Part object with a `kind` field")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Part, A::Error> {
                let mut obj = serde_json::Map::new();
                while let Some((k, v)) = map.next_entry::<String, serde_json::Value>()? {
                    obj.insert(k, v);
                }
                let kind = obj
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| de::Error::custom("Part: missing string `kind`"))?
                    .to_string();
                match kind.as_str() {
                    "text" => {
                        let text = obj
                            .get("text")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| de::Error::custom("Part text: missing `text`"))?
                            .to_string();
                        Ok(Part::Text(text))
                    }
                    "data" => {
                        let data = obj
                            .get("data")
                            .cloned()
                            .ok_or_else(|| de::Error::custom("Part data: missing `data`"))?;
                        Ok(Part::Data(data))
                    }
                    other => {
                        if !is_extension_kind(other) {
                            return Err(de::Error::custom(format!(
                                "Part: unrecognized core kind `{other}` (extensions must be reverse-DNS)"
                            )));
                        }
                        obj.remove("kind");
                        Ok(Part::Extension {
                            kind,
                            fields: obj,
                        })
                    }
                }
            }
        }
        de.deserialize_map(PartVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointRef {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// Exactly one routing target: multicast (`channel`), unicast (`to`), or anycast (`toService`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Route {
    Multicast {
        channel: String,
    },
    Unicast {
        to: String,
    },
    Anycast {
        #[serde(rename = "toService")]
        to_service: String,
    },
}

/// A message on the mesh (`CotalMessage` in the SPEC; one of the three routing variants).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Unique message id; the NATS binding also uses it as `Nats-Msg-Id`.
    pub id: String,
    /// Epoch ms.
    pub ts: i64,
    pub space: String,
    pub from: EndpointRef,
    #[serde(flatten)]
    pub route: Route,
    /// Lowercased peer names called out within a `channel` message — a wake hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<String>>,
    pub parts: Vec<Part>,
    /// Id of the message being replied to.
    #[serde(default, rename = "replyTo", skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// Conversation / thread correlation id.
    #[serde(default, rename = "contextId", skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
}

impl Message {
    /// The advisory delivery mode implied by the payload routing field. NOTE: a receiver MUST derive
    /// the *authenticated* kind from the delivering subject ([`crate::delivery_of`]), not this.
    pub fn route_mode(&self) -> DeliveryMode {
        match self.route {
            Route::Multicast { .. } => DeliveryMode::Chat,
            Route::Unicast { .. } => DeliveryMode::Unicast,
            Route::Anycast { .. } => DeliveryMode::Anycast,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PresenceEvent {
    Join { presence: Presence },
    Update { presence: Presence },
    Offline { presence: Presence },
}

/// Authenticated message class, derived from the **delivering subject** (not the payload routing
/// fields). The only trustworthy "how was this addressed to me" signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    Channel,
    Dm,
    Anycast,
}

/// Context delivered alongside a received [`Message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageMeta {
    /// `true` ⇒ replayed from a channel's backlog on join (a "catching up" block) vs live.
    pub historical: bool,
    pub kind: MessageKind,
}

/// Control-plane request (e.g. CLI → manager).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlRequest {
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<BTreeMap<String, serde_json::Value>>,
    pub from: EndpointRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlReply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

use crate::subjects::DeliveryMode;

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> EndpointRef {
        EndpointRef {
            id: "UAQGWOEVJKMIO4WXSYOTLARXYOZTCXFK67JASEH6AFFFYK6FOPSKQCAD".into(),
            name: "alice".into(),
            role: Some("planner".into()),
        }
    }

    #[test]
    fn multicast_message_round_trips_with_camelcase_wire() {
        let m = Message {
            id: "018f1d0a-0000-7000-9000-000000000001".into(),
            ts: 1710000000000,
            space: "main".into(),
            from: alice(),
            route: Route::Multicast {
                channel: "team.backend".into(),
            },
            mentions: Some(vec!["bob".into()]),
            parts: vec![Part::text("Can you review this?")],
            reply_to: None,
            context_id: Some("ctx-1".into()),
        };
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["channel"], "team.backend");
        assert_eq!(json["contextId"], "ctx-1");
        assert_eq!(json["parts"][0]["kind"], "text");
        assert!(json.get("to").is_none() && json.get("toService").is_none());
        let back: Message = serde_json::from_value(json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn unicast_and_anycast_set_exactly_one_routing_field() {
        let uni = Message {
            id: "2".into(),
            ts: 1,
            space: "main".into(),
            from: alice(),
            route: Route::Unicast { to: "UDI".into() },
            mentions: None,
            parts: vec![Part::text("Direct note.")],
            reply_to: None,
            context_id: None,
        };
        let j = serde_json::to_value(&uni).unwrap();
        assert_eq!(j["to"], "UDI");
        assert!(j.get("channel").is_none() && j.get("toService").is_none());

        let any = Message {
            route: Route::Anycast {
                to_service: "reviewer".into(),
            },
            ..uni.clone()
        };
        let j = serde_json::to_value(&any).unwrap();
        assert_eq!(j["toService"], "reviewer");
        assert!(j.get("channel").is_none() && j.get("to").is_none());
        assert_eq!(any.route_mode(), DeliveryMode::Anycast);
    }

    #[test]
    fn part_extension_round_trips_and_rejects_bare_unknown_kind() {
        let mut fields = serde_json::Map::new();
        fields.insert("ref".into(), serde_json::json!({"bucket": "b"}));
        let p = Part::Extension {
            kind: "com.acme.snapshot".into(),
            fields,
        };
        let j = serde_json::to_value(&p).unwrap();
        assert_eq!(j["kind"], "com.acme.snapshot");
        assert_eq!(j["ref"]["bucket"], "b");
        let back: Part = serde_json::from_value(j).unwrap();
        assert_eq!(back, p);

        // Bare unrecognized core kind is not conformant.
        let bad = serde_json::json!({"kind": "image", "blob": "x"});
        assert!(serde_json::from_value::<Part>(bad).is_err());
    }

    #[test]
    fn presence_omits_none_and_keeps_camelcase() {
        let p = Presence {
            card: AgentCard {
                id: "U1".into(),
                name: "bob".into(),
                kind: EndpointKind::Agent,
                role: None,
                description: None,
                tags: None,
                skills: None,
                meta: None,
                protocol_version: None,
            },
            status: PresenceStatus::Working,
            activity: Some("reviewing".into()),
            attention: Some(AttentionMode::Focus),
            channel_modes: None,
            ts: 42,
        };
        let j = serde_json::to_value(&p).unwrap();
        assert_eq!(j["status"], "working");
        assert_eq!(j["attention"], "focus");
        assert_eq!(j["card"]["kind"], "agent");
        assert!(j.get("channelModes").is_none());
        assert!(j["card"].get("role").is_none());
        let back: Presence = serde_json::from_value(j).unwrap();
        assert_eq!(back, p);
    }
}
