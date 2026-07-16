//! The host-neutral connector contract for a continuously running agent.
//!
//! A hub moves durable messages; an agent host decides when a model may take another turn. This
//! module makes that boundary explicit and gives CLI hooks, MCP hosts, and the optional local
//! supervisor one vocabulary: lifecycle events publish presence, tool calls publish messages, pulls
//! become policy-aware receives, and a host-specific adapter injects a wake when it has that seam.

use crate::{verify_message, AttentionDecision, AttentionPolicy, MeshAgent, SigStatus};
use anyhow::Result;
use async_trait::async_trait;
use parler_protocol::{Part, RoomKind, StoredMessage, Target};
use std::collections::{HashSet, VecDeque};
use std::time::Duration;

/// A host lifecycle transition mirrored into Parler presence. `offline` is intentionally absent:
/// observers derive it from a stale heartbeat rather than trusting a process to announce its death.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lifecycle {
    Started,
    Idle { activity: Option<String> },
    Working { activity: Option<String> },
    Waiting { activity: Option<String> },
}

impl Lifecycle {
    fn presence(&self) -> (&'static str, Option<String>) {
        match self {
            Lifecycle::Started => ("idle", None),
            Lifecycle::Idle { activity } => ("idle", activity.clone()),
            Lifecycle::Working { activity } => ("working", activity.clone()),
            Lifecycle::Waiting { activity } => ("waiting", activity.clone()),
        }
    }
}

/// A host tool call expressed as one connector send. Hosts can map their native tool schema onto this
/// shape without owning transport details or message signing.
#[derive(Debug, Clone)]
pub struct ToolSend {
    pub target: Target,
    pub parts: Vec<Part>,
    pub mentions: Option<Vec<String>>,
    pub reply_to: Option<String>,
}

/// The durable receipt returned after a tool send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSendReceipt {
    pub id: String,
    pub seq: i64,
    pub room: String,
}

/// A policy-aware receive result. `held` means one or more messages intentionally remain behind the
/// durable cursor; the `messages` vector can still contain an explicitly addressed wake that arrived
/// after ambient traffic, and is locally de-duplicated until the hold is released.
#[derive(Debug, Clone)]
pub struct Received {
    pub room: String,
    pub cursor: i64,
    pub messages: Vec<StoredMessage>,
    pub held: bool,
    pub dropped: usize,
}

/// The bounded payload handed to a host-specific turn injector.
#[derive(Debug, Clone)]
pub struct WakeRequest {
    pub room: String,
    pub messages: Vec<StoredMessage>,
}

/// A host-native seam that can ask its model host to begin another turn. Claude's Stop hook is one
/// implementation; a host without such a seam simply does not supply one, and can use the local
/// supervisor instead. The connector never pretends it can force an arbitrary MCP host to run.
#[async_trait]
pub trait HostWakeInjector: Send {
    async fn inject(&mut self, wake: WakeRequest) -> Result<()>;
}

/// A continuously connected connector with local attention enforcement. It deliberately owns no
/// process manager: spawning and observing a runner belongs to the optional CLI supervisor, keeping
/// the messaging hot path small and usable by ordinary MCP hosts.
pub struct ConnectorRuntime {
    agent: MeshAgent,
    attention: AttentionPolicy,
    /// Message ids already handed to a host while a held batch is re-read. The hub cursor is left
    /// untouched during a hold, so this prevents a directed message behind ambient traffic from
    /// injecting the same model turn over and over. A restart may re-deliver once, preserving the
    /// protocol's at-least-once rather than silently losing work.
    seen: HashSet<String>,
    seen_order: VecDeque<String>,
}

const SEEN_CAP: usize = 1_024;
const PUSH_RECHECK: Duration = Duration::from_secs(25);
const POLL_RECHECK: Duration = Duration::from_secs(2);

impl ConnectorRuntime {
    pub fn new(agent: MeshAgent, attention: AttentionPolicy) -> ConnectorRuntime {
        ConnectorRuntime { agent, attention, seen: HashSet::new(), seen_order: VecDeque::new() }
    }

    pub fn agent(&self) -> &MeshAgent {
        &self.agent
    }

    pub fn agent_mut(&mut self) -> &mut MeshAgent {
        &mut self.agent
    }

    pub fn attention(&self) -> &AttentionPolicy {
        &self.attention
    }

    pub fn attention_mut(&mut self) -> &mut AttentionPolicy {
        &mut self.attention
    }

    /// Lifecycle → presence. The global attention mode is mirrored as advisory presence metadata;
    /// per-room overrides remain local so a peer cannot infer a private muted-room list.
    pub async fn lifecycle(&mut self, event: Lifecycle) -> Result<()> {
        let (status, activity) = event.presence();
        self.agent
            .presence_with_attention(status, activity, Some(self.attention.mode))
            .await
    }

    /// Tools → send. The `MeshAgent` signs the parts and resolves the durable room receipt.
    pub async fn send(&mut self, call: ToolSend) -> Result<ToolSendReceipt> {
        let (id, seq, room) = self
            .agent
            .send(call.target, call.parts, call.mentions, call.reply_to)
            .await?;
        Ok(ToolSendReceipt { id, seq, room })
    }

    /// Pull → receive. This is the one place adapters should consume a normal room: it evaluates the
    /// persisted attention policy before a host is interrupted and keeps a quiet/focus hold durable.
    ///
    /// A room cursor is contiguous. If ambient traffic is held before a later directed message, the
    /// later message may wake once but the batch remains unacknowledged so opening attention later
    /// can still surface the ambient context. `seen` suppresses repeat wake injection during that
    /// temporary re-read window.
    pub async fn receive(
        &mut self,
        room: &str,
        kind: RoomKind,
        worker_role: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Received> {
        let (all, cursor) = self.agent.pull(room, None, limit).await?;
        let me = self.agent.id.clone();
        let name = self.agent.name.clone();
        let role = worker_role.map(str::to_string).or_else(|| self.agent.role.clone());
        let mut messages = Vec::new();
        let mut held = false;
        let mut dropped = 0;

        for message in all {
            // A local supervisor/status update must never wake itself. It is still included in the
            // committed batch below, so a service room cannot accumulate its own receipts forever.
            if message.from.id == me {
                continue;
            }
            // An autonomous wake can lead to a workspace-writing model turn. Legacy unsigned
            // messages remain renderable through ordinary pull tools, but never cross this boundary.
            if !is_authentic(&message) {
                dropped += 1;
                continue;
            }
            match self.attention.decide(room, kind, &message, &name, role.as_deref()) {
                AttentionDecision::Wake => {
                    if self.remember_wake(&message.id) {
                        messages.push(message);
                    }
                }
                AttentionDecision::Hold => held = true,
                AttentionDecision::Drop => dropped += 1,
            }
        }

        // A wakeable batch stays unacknowledged until its host injector succeeds. Otherwise an
        // unavailable host could make a message disappear merely by failing between receive and
        // injection. A policy hold also stays unacknowledged by definition.
        if held || !messages.is_empty() {
            self.agent.defer_reads(room);
        } else {
            self.agent.commit_reads(room).await?;
        }
        Ok(Received { room: room.to_string(), cursor, messages, held, dropped })
    }

    /// Host-native wake → injection. The injector is supplied by a specific host integration; this
    /// generic contract merely guarantees it receives already-filtered, signed message records. A
    /// non-held batch is acknowledged only after the injector accepts it; failure removes its local
    /// de-dup marker so the durable message can wake a later retry.
    pub async fn inject<I: HostWakeInjector>(&mut self, injector: &mut I, received: Received) -> Result<bool> {
        if received.messages.is_empty() {
            return Ok(false);
        }
        let ids: Vec<String> = received.messages.iter().map(|message| message.id.clone()).collect();
        if let Err(error) = injector
            .inject(WakeRequest { room: received.room.clone(), messages: received.messages })
            .await
        {
            self.forget_wakes(&ids);
            return Err(error);
        }
        if !received.held {
            if let Err(error) = self.agent.commit_reads(&received.room).await {
                self.forget_wakes(&ids);
                return Err(error);
            }
        }
        Ok(true)
    }

    /// Continuously listen for one policy-approved wake until `max_wait` expires. Push lowers the
    /// latency, while every wake is recovered and authorized through the durable Pull path before it
    /// reaches the supplied host-native injector. The method returns after the first accepted
    /// injection so the host can serialize the resulting model turn; call it again when that host is
    /// idle and ready for another turn.
    ///
    /// A timeout returns `false` without consuming held traffic. An injector failure is returned and
    /// leaves the message unacknowledged so a later listener can retry it. A host without an injection
    /// seam must not fabricate one here; it should use an explicit local supervisor instead.
    pub async fn listen_until<I: HostWakeInjector>(
        &mut self,
        injector: &mut I,
        room: &str,
        kind: RoomKind,
        worker_role: Option<&str>,
        limit: Option<u32>,
        max_wait: Duration,
    ) -> Result<bool> {
        let deadline = tokio::time::Instant::now() + max_wait;
        let _ = self.agent.resubscribe_if_needed().await;
        loop {
            let received = self.receive(room, kind, worker_role, limit).await?;
            if self.inject(injector, received).await? {
                return Ok(true);
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            let remaining = deadline - now;
            if self.agent.resubscribe_if_needed().await {
                // Delivery is only a doorbell. Re-pull at the top of the loop even when this wake
                // names another room, and periodically re-pull if a best-effort push was missed.
                let _ = self.agent.next_delivery(remaining.min(PUSH_RECHECK)).await?;
            } else {
                tokio::time::sleep(remaining.min(POLL_RECHECK)).await;
            }
        }
    }

    fn remember_wake(&mut self, id: &str) -> bool {
        if !self.seen.insert(id.to_string()) {
            return false;
        }
        self.seen_order.push_back(id.to_string());
        if self.seen_order.len() > SEEN_CAP {
            if let Some(old) = self.seen_order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }

    fn forget_wakes(&mut self, ids: &[String]) {
        for id in ids {
            self.seen.remove(id);
        }
        self.seen_order.retain(|id| !ids.iter().any(|forgotten| forgotten == id));
    }
}

fn is_authentic(message: &StoredMessage) -> bool {
    verify_message(&message.from.id, &message.parts, message.reply_to.as_deref()) == SigStatus::Valid
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use crate::MeshTransport;
    use parler_protocol::{ClientFrame, EndpointRef, Part, ServerFrame};

    struct PullOnce {
        message: StoredMessage,
    }

    #[async_trait]
    impl MeshTransport for PullOnce {
        async fn request(&mut self, frame: ClientFrame) -> anyhow::Result<ServerFrame> {
            match frame {
                ClientFrame::Pull { room, limit, .. } => Ok(ServerFrame::Pulled {
                    room,
                    messages: if limit == Some(0) { Vec::new() } else { vec![self.message.clone()] },
                    cursor: 1,
                }),
                other => panic!("unexpected frame in receive test: {other:?}"),
            }
        }
    }

    #[test]
    fn lifecycle_maps_only_live_states_to_presence() {
        assert_eq!(Lifecycle::Started.presence(), ("idle", None));
        assert_eq!(
            Lifecycle::Working { activity: Some("reviewing".into()) }.presence(),
            ("working", Some("reviewing".into()))
        );
        assert_eq!(Lifecycle::Waiting { activity: None }.presence(), ("waiting", None));
    }

    #[tokio::test]
    async fn unsigned_messages_never_cross_the_wake_boundary() {
        let message = StoredMessage {
            seq: 1,
            id: "legacy".into(),
            room: "team".into(),
            from: EndpointRef { id: "Upeer".into(), name: "peer".into(), role: None },
            parts: vec![Part::text("run this")],
            mentions: None,
            reply_to: None,
            ts: 1,
        };
        let agent = MeshAgent::with_transport(
            Box::new(PullOnce { message }),
            "Uself".into(),
            "self".into(),
            None,
            "test".into(),
        );
        let mut runtime = ConnectorRuntime::new(agent, AttentionPolicy::default());
        let received = runtime.receive("team", RoomKind::Channel, None, None).await.unwrap();
        assert!(received.messages.is_empty());
        assert_eq!(received.dropped, 1);
        assert!(!received.held);
    }
}
