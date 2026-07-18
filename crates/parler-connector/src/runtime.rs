//! The host-neutral connector contract for a continuously running agent.
//!
//! A hub moves durable messages; an agent host decides when a model may take another turn. This
//! module makes that boundary explicit and gives CLI hooks, MCP hosts, and the optional local
//! supervisor one vocabulary: lifecycle events publish presence, tool calls publish messages, pulls
//! become policy-aware receives, and a host-specific adapter injects a wake when it has that seam.

use crate::{home_dir, verify_message_for_room, AttentionDecision, AttentionPolicy, MeshAgent, SigStatus};
use anyhow::{Context, Result};
use async_trait::async_trait;
use parler_protocol::{MessageSig, Part, RoomKind, StoredMessage, Target};
use std::collections::{HashSet, VecDeque};
use std::fs::File;
use std::path::PathBuf;
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

#[derive(Default)]
struct ReplayBucket {
    completed: HashSet<String>,
    order: VecDeque<String>,
}

/// Receiver-owned, bounded replay protection for messages that can start autonomous work.
///
/// A signed UID is stable even if an untrusted hub changes its own stored-message id. Production
/// runtimes reserve `(sender, uid)` keys per receiving identity beneath `$PARLER_HOME` before a host
/// acts. A stable lock file makes that check-and-write atomic across local processes sharing an
/// identity; an explicit pre-action failure can release its reservation for retry.
pub struct AutonomousReplayGuard {
    path: Option<PathBuf>,
    bucket: ReplayBucket,
    pending: HashSet<String>,
    loaded: bool,
}

const REPLAY_CAP: usize = 16_384;

impl AutonomousReplayGuard {
    /// An in-memory guard for tests and embedded callers that do not want local persistence.
    pub fn ephemeral() -> AutonomousReplayGuard {
        AutonomousReplayGuard {
            path: None,
            bucket: ReplayBucket::default(),
            pending: HashSet::new(),
            loaded: true,
        }
    }

    /// A durable guard scoped to one local identity.
    pub fn persistent(agent_id: &str) -> AutonomousReplayGuard {
        let identity = parler_auth::content_id(agent_id.as_bytes());
        AutonomousReplayGuard {
            path: Some(home_dir().join("autonomous-replay").join(format!("{identity}.json"))),
            bucket: ReplayBucket::default(),
            pending: HashSet::new(),
            loaded: false,
        }
    }

    /// Verify the signature's delivery context and reserve its stable signed UID for one local
    /// execution attempt. Returns `false` for invalid, misrouted, completed, or already-pending work.
    pub fn admit(&mut self, message: &StoredMessage, recipient_id: &str) -> Result<bool> {
        if verify_message_for_room(message, recipient_id) != SigStatus::Valid {
            return Ok(false);
        }
        let Some(key) = replay_key(message) else { return Ok(false) };
        let _lock = self.lock_file()?;
        self.refresh()?;
        if self.bucket.completed.contains(&key) {
            return Ok(false);
        }
        self.remember(key.clone());
        self.save()?;
        self.pending.insert(key);
        Ok(true)
    }

    /// Mark accepted host actions complete before their hub cursor is committed. Admission already
    /// persisted the signed UID, so completion only closes the local releasable state.
    pub fn complete(&mut self, messages: &[StoredMessage]) -> Result<()> {
        for message in messages {
            let Some(key) = replay_key(message) else { continue };
            self.pending.remove(&key);
        }
        Ok(())
    }

    /// Release an uncompleted reservation after a host injection fails so the durable pull can retry.
    pub fn release(&mut self, messages: &[StoredMessage]) -> Result<()> {
        let _lock = self.lock_file()?;
        self.refresh()?;
        let mut dirty = false;
        for message in messages {
            if let Some(key) = replay_key(message) {
                if self.pending.remove(&key) && self.bucket.completed.remove(&key) {
                    self.bucket.order.retain(|entry| entry != &key);
                    dirty = true;
                }
            }
        }
        if dirty {
            self.save()?;
        }
        Ok(())
    }

    fn refresh(&mut self) -> Result<()> {
        if self.path.is_none() && self.loaded {
            return Ok(());
        }
        let mut bucket = ReplayBucket::default();
        if let Some(path) = &self.path {
            match std::fs::read(path) {
                Ok(bytes) => {
                    let keys: Vec<String> = serde_json::from_slice(&bytes)
                        .with_context(|| format!("parsing autonomous replay ledger {}", path.display()))?;
                    for key in keys.into_iter().rev().take(REPLAY_CAP).collect::<Vec<_>>().into_iter().rev() {
                        if bucket.completed.insert(key.clone()) {
                            bucket.order.push_back(key);
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("reading autonomous replay ledger {}", path.display()));
                }
            }
        }
        self.bucket = bucket;
        self.loaded = true;
        Ok(())
    }

    fn remember(&mut self, key: String) {
        if self.bucket.completed.insert(key.clone()) {
            self.bucket.order.push_back(key);
        }
        while self.bucket.order.len() > REPLAY_CAP {
            if let Some(old) = self.bucket.order.pop_front() {
                self.bucket.completed.remove(&old);
            }
        }
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let body = serde_json::to_vec(&self.bucket.order)?;
        parler_auth::write_private_file(path, &body)
            .with_context(|| format!("writing autonomous replay ledger {}", path.display()))
    }

    fn lock_file(&self) -> Result<Option<File>> {
        let Some(path) = &self.path else { return Ok(None) };
        let parent = path.parent().context("autonomous replay ledger has no parent directory")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating autonomous replay directory {}", parent.display()))?;
        let lock_path = path.with_extension("lock");
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&lock_path)
            .with_context(|| format!("opening autonomous replay lock {}", lock_path.display()))?;
        file.lock()
            .with_context(|| format!("locking autonomous replay ledger {}", path.display()))?;
        Ok(Some(file))
    }
}

fn replay_key(message: &StoredMessage) -> Option<String> {
    let signature = MessageSig::from_parts(&message.parts)?;
    Some(format!("{}:{}", message.from.id, signature.uid))
}

/// A continuously connected connector with local attention enforcement. It deliberately owns no
/// process manager: spawning and observing a runner belongs to the optional CLI supervisor, keeping
/// the messaging hot path small and usable by ordinary MCP hosts.
pub struct ConnectorRuntime {
    agent: MeshAgent,
    attention: AttentionPolicy,
    replay: AutonomousReplayGuard,
}

const PUSH_RECHECK: Duration = Duration::from_secs(25);
const POLL_RECHECK: Duration = Duration::from_secs(2);

impl ConnectorRuntime {
    /// Construct an embedded/test runtime with process-local replay protection.
    pub fn new(agent: MeshAgent, attention: AttentionPolicy) -> ConnectorRuntime {
        ConnectorRuntime { agent, attention, replay: AutonomousReplayGuard::ephemeral() }
    }

    /// Construct a production runtime whose completed signed UIDs survive host restarts.
    pub fn persistent(agent: MeshAgent, attention: AttentionPolicy) -> ConnectorRuntime {
        let replay = AutonomousReplayGuard::persistent(&agent.id);
        ConnectorRuntime { agent, attention, replay }
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
    /// can still surface the ambient context. The replay guard suppresses repeat wake injection
    /// during that temporary re-read window and across later relay replays.
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
            if verify_message_for_room(&message, &me) != SigStatus::Valid {
                dropped += 1;
                continue;
            }
            match self.attention.decide(room, kind, &message, &name, role.as_deref()) {
                AttentionDecision::Wake => {
                    if self.replay.admit(&message, &me)? {
                        messages.push(message);
                    } else {
                        dropped += 1;
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
        let messages = received.messages;
        if let Err(error) = injector
            .inject(WakeRequest { room: received.room.clone(), messages: messages.clone() })
            .await
        {
            self.replay.release(&messages)?;
            return Err(error);
        }
        self.replay.complete(&messages)?;
        if !received.held {
            self.agent.commit_reads(&received.room).await?;
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

    /// Admit one queue-delivered message through the same room-binding and signed-UID replay gate
    /// used by ordinary receives. Role queues bypass `receive`, so supervisors call this explicitly.
    pub fn admit_autonomous(&mut self, message: &StoredMessage) -> Result<bool> {
        self.replay.admit(message, &self.agent.id)
    }

    /// Record that the host action for these admitted messages has occurred.
    pub fn complete_autonomous(&mut self, messages: &[StoredMessage]) -> Result<()> {
        self.replay.complete(messages)
    }

    /// Release admitted messages that did not reach a host action.
    pub fn release_autonomous(&mut self, messages: &[StoredMessage]) -> Result<()> {
        self.replay.release(messages)
    }
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

    #[test]
    fn completed_signed_uid_is_rejected_under_a_new_hub_id() {
        let sender = parler_auth::new_identity().unwrap();
        let recipient = parler_auth::new_identity().unwrap();
        let target = Target::Dm { agent: recipient.id.clone() };
        let mut parts = vec![Part::text("run this once")];
        let ts = 1_710_000_000_000;
        let uid = "stable-author-uid";
        let bytes = parler_protocol::canonical_message_bytes(
            &sender.id,
            &target,
            &parts,
            None,
            ts,
            uid,
        );
        let sig = parler_auth::sign(&sender.seed, &bytes).unwrap();
        parts.push(MessageSig { sig, ts, uid: uid.into(), target }.to_part());
        let mut message = StoredMessage {
            seq: 1,
            id: "hub-id-1".into(),
            room: "dm.first".into(),
            from: EndpointRef { id: sender.id, name: "sender".into(), role: None },
            parts,
            mentions: None,
            reply_to: None,
            ts: 1,
        };
        let mut replay = AutonomousReplayGuard::ephemeral();
        assert!(replay.admit(&message, &recipient.id).unwrap());
        replay.complete(std::slice::from_ref(&message)).unwrap();
        message.id = "hub-id-2".into();
        message.seq = 99;
        message.room = "dm.second".into();
        assert!(!replay.admit(&message, &recipient.id).unwrap());
        replay.release(std::slice::from_ref(&message)).unwrap();
        assert!(!replay.admit(&message, &recipient.id).unwrap());
    }

    #[test]
    fn durable_admission_is_visible_to_another_local_listener_and_releasable() {
        let sender = parler_auth::new_identity().unwrap();
        let recipient = parler_auth::new_identity().unwrap();
        let target = Target::Dm { agent: recipient.id.clone() };
        let mut parts = vec![Part::text("run once")];
        let bytes = parler_protocol::canonical_message_bytes(
            &sender.id,
            &target,
            &parts,
            None,
            1,
            "shared-uid",
        );
        let sig = parler_auth::sign(&sender.seed, &bytes).unwrap();
        parts.push(MessageSig { sig, ts: 1, uid: "shared-uid".into(), target }.to_part());
        let message = StoredMessage {
            seq: 1,
            id: "hub-id".into(),
            room: "dm.first".into(),
            from: EndpointRef { id: sender.id, name: "sender".into(), role: None },
            parts,
            mentions: None,
            reply_to: None,
            ts: 1,
        };

        let directory = std::env::temp_dir().join(format!("parler-replay-{}", uuid::Uuid::new_v4()));
        let path = directory.join("receiver.json");
        let guard = || AutonomousReplayGuard {
            path: Some(path.clone()),
            bucket: ReplayBucket::default(),
            pending: HashSet::new(),
            loaded: false,
        };
        let mut first = guard();
        let mut second = guard();
        assert!(first.admit(&message, &recipient.id).unwrap());
        assert!(!second.admit(&message, &recipient.id).unwrap());
        first.release(std::slice::from_ref(&message)).unwrap();
        assert!(second.admit(&message, &recipient.id).unwrap());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
