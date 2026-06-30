//! [`MeshAgent`] — the high-level API the CLI, MCP server, and Hermes plugin all call.
//!
//! Every method is one request/reply round-trip against the [`MeshTransport`]. The three send
//! patterns are just three [`Target`]s: a channel room (one-to-many), a peer DM (one-to-one), or a
//! service room (many-to-one).

use crate::{Config, HubClient, MeshTransport};
use anyhow::{bail, Result};
use parler_auth::Identity;
use parler_protocol::{
    canonical_card_bytes, canonical_message_bytes, is_message_sig_part, AgentCard, AgentSkill,
    BundleRef, ClientFrame, DirectoryEntry, DiscoverScope, EndpointKind, Fact, JoinRequest,
    MessageSig, Part, RecallHit, RoomInfo, RoomKind, RosterEntry, ServerFrame, StoredMessage, Target,
    Visibility,
};
use std::time::Duration;

/// A freshly minted invite — the code/link the human pastes to another agent.
pub struct Invite {
    pub code: String,
    pub url: String,
    pub room: String,
    pub kind: RoomKind,
    pub expires_at: i64,
}

/// The result of redeeming an invite code via [`MeshAgent::redeem`].
pub enum JoinOutcome {
    /// Admitted into the room (an ordinary invite, or an approval-gated one the owner has approved).
    Joined { room: String, kind: RoomKind },
    /// The invite is approval-gated: the redeem is recorded as a pending request and the caller is
    /// **not** in the room yet. Re-redeem the same code to poll once the owner has decided.
    Pending { room: String },
}

/// What to advertise about a pushed artifact (everything but the bytes and their content id).
#[derive(Debug, Clone, Default)]
pub struct BundleMeta {
    /// Artifact kind: `"git"` (a git bundle), `"patch"`, `"tar"`, …
    pub vcs: String,
    pub tip: Option<String>,
    pub base: Option<String>,
    pub summary: Option<String>,
    pub media_type: Option<String>,
}

/// The outcome of [`MeshAgent::push`]: the posted message + the stored blob's content id.
pub struct PushReceipt {
    pub msg_id: String,
    pub blob_id: String,
    pub room: String,
    pub seq: i64,
}

/// A connected, authenticated agent on the mesh.
pub struct MeshAgent {
    transport: Box<dyn MeshTransport>,
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub hub_url: String,
    /// The local identity (nkey seed) — present when connected from a [`Config`]; required to sign a
    /// discovery card in [`MeshAgent::register`].
    identity: Option<Identity>,
}

impl MeshAgent {
    /// Connect + authenticate using a loaded [`Config`].
    pub async fn connect(cfg: &Config) -> Result<MeshAgent> {
        let client =
            HubClient::connect(&cfg.hub_url, &cfg.identity, &cfg.name, cfg.role.as_deref()).await?;
        Ok(MeshAgent {
            transport: Box::new(client),
            id: cfg.identity.id.clone(),
            name: cfg.name.clone(),
            role: cfg.role.clone(),
            hub_url: cfg.hub_url.clone(),
            identity: Some(cfg.identity.clone()),
        })
    }

    /// Build an agent over any transport (used by tests with an in-process transport). Without an
    /// identity, [`MeshAgent::register`] is unavailable (nothing to sign the card with).
    pub fn with_transport(
        transport: Box<dyn MeshTransport>,
        id: String,
        name: String,
        role: Option<String>,
        hub_url: String,
    ) -> MeshAgent {
        MeshAgent { transport, id, name, role, hub_url, identity: None }
    }

    /// Mint an invite. `kind` is `Dm` for a 1:1 hand-off, `Channel` for a group room, `Service` for
    /// a worker queue. The returned code/link is what the human pastes to the other agent.
    pub async fn invite(
        &mut self,
        kind: RoomKind,
        room: Option<String>,
        ttl_secs: Option<u64>,
        max_uses: Option<u32>,
    ) -> Result<Invite> {
        self.invite_with_approval(kind, room, ttl_secs, max_uses, false).await
    }

    /// Mint an invite, optionally **approval-gated**: with `require_approval`, redeeming the code does
    /// not join immediately — it records a pending request the room owner must approve (see
    /// [`MeshAgent::join_requests`] / [`MeshAgent::resolve_join`]). This is how a live session vets who
    /// is let into the shared conversation. (The hub honors approval for `Channel` rooms only.)
    pub async fn invite_with_approval(
        &mut self,
        kind: RoomKind,
        room: Option<String>,
        ttl_secs: Option<u64>,
        max_uses: Option<u32>,
        require_approval: bool,
    ) -> Result<Invite> {
        match self
            .transport
            .request(ClientFrame::Invite { kind, room, ttl_secs, max_uses, require_approval })
            .await?
        {
            ServerFrame::Invited { code, url, room, kind, expires_at } => {
                Ok(Invite { code, url, room, kind, expires_at })
            }
            other => bail!("unexpected reply to invite: {other:?}"),
        }
    }

    /// Redeem a pasted code/link, distinguishing an immediate join from an approval-gated
    /// [`JoinOutcome::Pending`] (the owner must approve before the caller is admitted).
    pub async fn redeem(&mut self, code: &str) -> Result<JoinOutcome> {
        match self.transport.request(ClientFrame::Redeem { code: code.to_string() }).await? {
            ServerFrame::Joined { room, kind } => Ok(JoinOutcome::Joined { room, kind }),
            ServerFrame::JoinPending { room } => Ok(JoinOutcome::Pending { room }),
            other => bail!("unexpected reply to redeem: {other:?}"),
        }
    }

    /// Redeem a pasted code/link — joins the room it grants. If the invite is approval-gated and the
    /// owner hasn't approved yet, this errors (the join is pending); use [`MeshAgent::redeem`] to
    /// handle the pending case explicitly.
    pub async fn join(&mut self, code: &str) -> Result<(String, RoomKind)> {
        match self.redeem(code).await? {
            JoinOutcome::Joined { room, kind } => Ok((room, kind)),
            JoinOutcome::Pending { room } => {
                bail!("join request for '{room}' is pending the host's approval")
            }
        }
    }

    /// List the pending join requests for a session/room you **own** (created via an approval-gated
    /// invite). The hub authorizes this to the owner only.
    pub async fn join_requests(&mut self, room: &str) -> Result<Vec<JoinRequest>> {
        match self.transport.request(ClientFrame::JoinRequests { room: room.to_string() }).await? {
            ServerFrame::JoinRequests { requests, .. } => Ok(requests),
            other => bail!("unexpected reply to join_requests: {other:?}"),
        }
    }

    /// Approve (`approve = true`) or deny a pending join request for a room you own. On approval the
    /// requester is admitted; on denial it is rejected and cannot re-request. Returns whether the
    /// requester was admitted.
    pub async fn resolve_join(&mut self, room: &str, agent: &str, approve: bool) -> Result<bool> {
        match self
            .transport
            .request(ClientFrame::ResolveJoin {
                room: room.to_string(),
                agent: agent.to_string(),
                approve,
            })
            .await?
        {
            ServerFrame::JoinResolved { approved, .. } => Ok(approved),
            other => bail!("unexpected reply to resolve_join: {other:?}"),
        }
    }

    /// Join/create a service room as a worker, so it can receive (`pull`) tasks sent to the service.
    pub async fn serve(&mut self, service: &str) -> Result<String> {
        match self.transport.request(ClientFrame::Serve { service: service.to_string() }).await? {
            ServerFrame::Joined { room, .. } => Ok(room),
            other => bail!("unexpected reply to serve: {other:?}"),
        }
    }

    /// Publish `parts` to a target. When this agent holds its signing seed (the normal CLI/MCP path),
    /// the message is **authenticated**: we sign the author-controlled content and ride the signature
    /// inside `parts` as a [`MESSAGE_SIG_KIND`](parler_protocol::MESSAGE_SIG_KIND) extension, so a
    /// puller can verify (via [`verify_message`]) that even a compromised hub didn't forge or alter
    /// what we said. An agent built without an identity (e.g. an in-process test transport) sends
    /// unsigned, exactly as before — the signature is purely additive.
    pub async fn send(
        &mut self,
        target: Target,
        mut parts: Vec<Part>,
        mentions: Option<Vec<String>>,
        reply_to: Option<String>,
    ) -> Result<(String, i64, String)> {
        if let Some(identity) = &self.identity {
            // Defensive: never double-sign (e.g. a caller re-sending parts it received).
            parts.retain(|p| !is_message_sig_part(p));
            let ts = now_ms();
            let uid = uuid::Uuid::new_v4().to_string();
            let bytes =
                canonical_message_bytes(&self.id, &target, &parts, reply_to.as_deref(), ts, &uid);
            let sig = parler_auth::sign(&identity.seed, &bytes)?;
            parts.push(MessageSig { sig, ts, uid, target: target.clone() }.to_part());
        }
        match self
            .transport
            .request(ClientFrame::Send { target, parts, mentions, reply_to })
            .await?
        {
            ServerFrame::Sent { id, seq, room } => Ok((id, seq, room)),
            other => bail!("unexpected reply to send: {other:?}"),
        }
    }

    /// Convenience: send a single text part.
    pub async fn send_text(&mut self, target: Target, text: &str) -> Result<(String, i64, String)> {
        self.send(target, vec![Part::text(text)], None, None).await
    }

    /// Hand off an artifact (a git bundle by default): upload the bytes to the hub's content-addressed
    /// blob store (bound to the room `target` resolves to), then post a room message carrying a
    /// `com.parler.bundle` reference so peers see it through the ordinary `recv`. `note` is an
    /// optional text part shown alongside the reference.
    pub async fn push(
        &mut self,
        target: Target,
        bundle: &[u8],
        meta: BundleMeta,
        note: Option<String>,
    ) -> Result<PushReceipt> {
        let blob_id = parler_auth::content_id(bundle);
        let put = ClientFrame::PutBlob {
            target: target.clone(),
            sha256: blob_id.clone(),
            size: bundle.len() as u64,
            media_type: meta.media_type.clone(),
        };
        match self.transport.upload_blob(put, bundle).await? {
            ServerFrame::BlobStored { id, .. } if id == blob_id => {}
            ServerFrame::BlobStored { id, .. } => bail!("hub stored a different blob id: {id}"),
            other => bail!("unexpected reply to put_blob: {other:?}"),
        }
        let bref = BundleRef {
            blob: blob_id.clone(),
            vcs: meta.vcs,
            tip: meta.tip,
            base: meta.base,
            summary: meta.summary,
            size: bundle.len() as u64,
            media_type: meta.media_type,
        };
        let mut parts = Vec::new();
        if let Some(n) = note {
            if !n.is_empty() {
                parts.push(Part::text(n));
            }
        }
        parts.push(bref.to_part());
        let (msg_id, seq, room) = self.send(target, parts, None, None).await?;
        Ok(PushReceipt { msg_id, blob_id, room, seq })
    }

    /// Download a blob's bytes by its content id (as carried in a `com.parler.bundle` part).
    pub async fn fetch_blob(&mut self, id: &str) -> Result<Vec<u8>> {
        self.transport
            .download_blob(ClientFrame::GetBlob { id: id.to_string() })
            .await
    }

    /// Pull new messages for `room` (past the agent's cursor, which this advances), or past `since`
    /// (which does not). Returns the messages and the resulting cursor.
    pub async fn pull(
        &mut self,
        room: &str,
        since: Option<i64>,
        limit: Option<u32>,
    ) -> Result<(Vec<StoredMessage>, i64)> {
        match self
            .transport
            .request(ClientFrame::Pull { room: room.to_string(), since, limit })
            .await?
        {
            ServerFrame::Pulled { messages, cursor, .. } => Ok((messages, cursor)),
            other => bail!("unexpected reply to pull: {other:?}"),
        }
    }

    /// Ask the hub to **push** new room messages to this connection (sub-second delivery), instead of
    /// waiting for the next [`MeshAgent::pull`]. Returns `true` if the hub supports push, `false`
    /// otherwise (an older hub) — in which case keep using `pull`. The subscription covers every room
    /// the agent belongs to now or joins later, and ends when the connection drops.
    pub async fn subscribe(&mut self) -> Result<bool> {
        self.transport.subscribe().await
    }

    /// Block up to `max_wait` for the next pushed message (a peer's, never your own); `None` on
    /// timeout. Only meaningful after [`MeshAgent::subscribe`] returned `true`. A push is best-effort
    /// and does **not** advance the durable cursor, so the idiomatic use is "block here to wake
    /// promptly, then [`MeshAgent::pull`] to read + advance authoritatively" (which also dedups).
    pub async fn next_delivery(&mut self, max_wait: Duration) -> Result<Option<StoredMessage>> {
        self.transport.next_delivery(max_wait).await
    }

    /// Write a fact to the memory store (idempotent when `key` is set).
    pub async fn remember(
        &mut self,
        text: &str,
        key: Option<String>,
        room: Option<String>,
        embedding: Option<Vec<f32>>,
        embedding_model: Option<String>,
    ) -> Result<()> {
        match self
            .transport
            .request(ClientFrame::Remember {
                fact: Fact { key, text: text.to_string(), room },
                embedding,
                embedding_model,
            })
            .await?
        {
            ServerFrame::Remembered { .. } => Ok(()),
            other => bail!("unexpected reply to remember: {other:?}"),
        }
    }

    /// Recall from the memory store. Pure text runs BM25; with an embedding, runs hybrid
    /// BM25 + vector KNN fused via Reciprocal Rank Fusion.
    pub async fn recall(
        &mut self,
        query: &str,
        room: Option<String>,
        limit: Option<u32>,
        embedding: Option<Vec<f32>>,
    ) -> Result<Vec<RecallHit>> {
        match self
            .transport
            .request(ClientFrame::Recall { query: query.to_string(), room, limit, embedding })
            .await?
        {
            ServerFrame::Recalled { hits } => Ok(hits),
            other => bail!("unexpected reply to recall: {other:?}"),
        }
    }

    /// List the rooms the agent belongs to (with unread counts).
    pub async fn rooms(&mut self) -> Result<Vec<RoomInfo>> {
        match self.transport.request(ClientFrame::Rooms).await? {
            ServerFrame::Rooms { rooms } => Ok(rooms),
            other => bail!("unexpected reply to rooms: {other:?}"),
        }
    }

    /// The members + presence of a room.
    pub async fn roster(&mut self, room: &str) -> Result<Vec<RosterEntry>> {
        match self.transport.request(ClientFrame::Roster { room: room.to_string() }).await? {
            ServerFrame::Roster { entries, .. } => Ok(entries),
            other => bail!("unexpected reply to roster: {other:?}"),
        }
    }

    /// Advertise presence (status + optional activity line).
    pub async fn presence(&mut self, status: &str, activity: Option<String>) -> Result<()> {
        match self
            .transport
            .request(ClientFrame::Presence { status: status.to_string(), activity })
            .await?
        {
            ServerFrame::PresenceOk => Ok(()),
            other => bail!("unexpected reply to presence: {other:?}"),
        }
    }

    // ---- discovery ----

    /// Publish (or refresh) this agent's directory card. Builds an [`AgentCard`] from the agent's
    /// identity + the supplied `tags`/`skills`/`description`, **signs** its canonical bytes with the
    /// local nkey seed, and registers it. Returns the stored `(visibility, verified)`.
    pub async fn register(
        &mut self,
        visibility: Visibility,
        tags: Vec<String>,
        skills: Vec<AgentSkill>,
        description: Option<String>,
    ) -> Result<(Visibility, bool)> {
        let identity = self.identity.as_ref().ok_or_else(|| {
            anyhow::anyhow!("no local identity to sign the card — connect from a Config")
        })?;
        let card = AgentCard {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: EndpointKind::Agent,
            role: self.role.clone(),
            description,
            tags: (!tags.is_empty()).then_some(tags),
            skills: (!skills.is_empty()).then_some(skills),
            meta: None,
            protocol_version: Some(parler_protocol::PROTOCOL_VERSION.to_string()),
        };
        let sig = parler_auth::sign(&identity.seed, &canonical_card_bytes(&card))?;
        match self
            .transport
            .request(ClientFrame::Register { card, visibility, sig: Some(sig) })
            .await?
        {
            ServerFrame::Registered { visibility, verified, .. } => Ok((visibility, verified)),
            other => bail!("unexpected reply to register: {other:?}"),
        }
    }

    /// Search the hub's directory. [`DiscoverScope::Public`] returns only public agents;
    /// [`DiscoverScope::Hub`] returns every agent in the hub.
    pub async fn discover(
        &mut self,
        scope: DiscoverScope,
        query: Option<String>,
        tag: Option<String>,
        skill: Option<String>,
        status: Option<String>,
        limit: Option<u32>,
    ) -> Result<Vec<DirectoryEntry>> {
        match self
            .transport
            .request(ClientFrame::Discover { scope, query, tag, skill, status, limit })
            .await?
        {
            ServerFrame::Directory { agents } => Ok(agents),
            other => bail!("unexpected reply to discover: {other:?}"),
        }
    }

    /// Fetch a single agent's directory card by id.
    pub async fn lookup(&mut self, id: &str) -> Result<Option<DirectoryEntry>> {
        match self.transport.request(ClientFrame::Lookup { id: id.to_string() }).await? {
            ServerFrame::Card { entry } => Ok(entry),
            other => bail!("unexpected reply to lookup: {other:?}"),
        }
    }

    /// Mint a read-scoped, expiring directory token (paste into the website to view a private hub).
    pub async fn mint_directory_token(&mut self, ttl_secs: Option<u64>) -> Result<(String, i64)> {
        match self.transport.request(ClientFrame::MintDirectoryToken { ttl_secs }).await? {
            ServerFrame::DirectoryToken { token, expires_at } => Ok((token, expires_at)),
            other => bail!("unexpected reply to mint token: {other:?}"),
        }
    }

    /// Mint a read-only **watch** token for a session/room you **own** — the bearer you paste into the
    /// website's session viewer to watch the conversation and how many agents are in it, without joining.
    /// The hub authorizes this to the room owner only. Returns `(token, expires_at)`.
    pub async fn mint_watch_token(&mut self, room: &str, ttl_secs: Option<u64>) -> Result<(String, i64)> {
        match self
            .transport
            .request(ClientFrame::MintWatch { room: room.to_string(), ttl_secs })
            .await?
        {
            ServerFrame::Watch { token, expires_at, .. } => Ok((token, expires_at)),
            other => bail!("unexpected reply to mint watch token: {other:?}"),
        }
    }
}

/// The outcome of verifying a received message's author signature ([`verify_message`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigStatus {
    /// No `com.parler.sig` part — a legacy/unauthenticated message (you have only the hub's word for
    /// who sent it). Not a failure: signatures are additive, so older peers send these.
    Unsigned,
    /// A signature is present and verifies against the author's id — the content is authentic and
    /// unaltered, regardless of whether the relaying hub is trustworthy.
    Valid,
    /// A signature is present but does **not** verify: the author id, the parts, the target, the
    /// `replyTo`, or the author timestamp was forged or altered after signing. Do not trust it.
    Invalid,
}

impl SigStatus {
    /// A short marker for CLI/MCP rendering: `✓` valid, `⚠` unsigned, `✗` tampered.
    pub fn marker(self) -> &'static str {
        match self {
            SigStatus::Valid => "✓",
            SigStatus::Unsigned => "⚠",
            SigStatus::Invalid => "✗",
        }
    }
}

/// Verify a received message's author signature **offline** — the point is not to trust the hub.
///
/// Recomputes [`canonical_message_bytes`] over the non-signature `parts` and checks it against
/// `from_id` (the stored author, i.e. its public key). A forged `from` therefore fails too: the
/// signature can't verify under an id the forger doesn't hold the seed for. Returns
/// [`SigStatus::Unsigned`] when no signature part is present (legacy message).
pub fn verify_message(from_id: &str, parts: &[Part], reply_to: Option<&str>) -> SigStatus {
    let Some(ms) = MessageSig::from_parts(parts) else {
        return SigStatus::Unsigned;
    };
    let bytes = canonical_message_bytes(from_id, &ms.target, parts, reply_to, ms.ts, &ms.uid);
    if parler_auth::verify(from_id, &bytes, &ms.sig) {
        SigStatus::Valid
    } else {
        SigStatus::Invalid
    }
}

/// Current epoch time in milliseconds (the author-stamped `ts` carried in the signature).
fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-roll a signed message exactly as `MeshAgent::send` would, so we can test `verify_message`
    /// (the receive side) in isolation, without a hub or a transport.
    fn signed(id: &parler_auth::Identity, target: Target, parts: &[Part], reply_to: Option<&str>) -> Vec<Part> {
        let ts = 1_710_000_000_000;
        let uid = "018f-test-uid";
        let bytes = canonical_message_bytes(&id.id, &target, parts, reply_to, ts, uid);
        let sig = parler_auth::sign(&id.seed, &bytes).unwrap();
        let mut out = parts.to_vec();
        out.push(MessageSig { sig, ts, uid: uid.into(), target }.to_part());
        out
    }

    #[test]
    fn valid_signature_verifies() {
        let alice = parler_auth::new_identity().unwrap();
        let parts = signed(&alice, Target::Room { room: "team".into() }, &[Part::text("ship it")], None);
        assert_eq!(verify_message(&alice.id, &parts, None), SigStatus::Valid);
    }

    #[test]
    fn altered_content_is_invalid() {
        let alice = parler_auth::new_identity().unwrap();
        let mut parts = signed(&alice, Target::Room { room: "team".into() }, &[Part::text("deploy v1")], None);
        // A malicious hub rewrites the authored text but keeps the signature part.
        parts[0] = Part::text("deploy v1 to prod");
        assert_eq!(verify_message(&alice.id, &parts, None), SigStatus::Invalid);
    }

    #[test]
    fn forged_author_is_invalid() {
        let alice = parler_auth::new_identity().unwrap();
        let mallory = parler_auth::new_identity().unwrap();
        let parts = signed(&alice, Target::Room { room: "team".into() }, &[Part::text("hi")], None);
        // The hub re-attributes alice's signed parts to mallory; it can't verify under mallory's key.
        assert_eq!(verify_message(&mallory.id, &parts, None), SigStatus::Invalid);
    }

    #[test]
    fn reply_to_is_covered_by_the_signature() {
        let alice = parler_auth::new_identity().unwrap();
        let parts = signed(&alice, Target::Room { room: "team".into() }, &[Part::text("yes")], Some("q1"));
        assert_eq!(verify_message(&alice.id, &parts, Some("q1")), SigStatus::Valid);
        // The hub re-threads the reply under a different parent → detected.
        assert_eq!(verify_message(&alice.id, &parts, Some("q2")), SigStatus::Invalid);
        assert_eq!(verify_message(&alice.id, &parts, None), SigStatus::Invalid);
    }

    #[test]
    fn target_is_covered_by_the_signature() {
        let alice = parler_auth::new_identity().unwrap();
        // Sign for a DM, then swap in a different sig part claiming a channel target with the same
        // sig bytes — verification recomputes over the *claimed* target and fails.
        let parts = signed(&alice, Target::Dm { agent: "UBOB".into() }, &[Part::text("psst")], None);
        let ms = MessageSig::from_parts(&parts).unwrap();
        let tampered_sig = MessageSig {
            target: Target::Room { room: "public".into() },
            ..ms
        };
        let mut tampered: Vec<Part> = parts.into_iter().filter(|p| !is_message_sig_part(p)).collect();
        tampered.push(tampered_sig.to_part());
        assert_eq!(verify_message(&alice.id, &tampered, None), SigStatus::Invalid);
    }

    #[test]
    fn no_signature_part_is_unsigned() {
        let alice = parler_auth::new_identity().unwrap();
        assert_eq!(verify_message(&alice.id, &[Part::text("legacy")], None), SigStatus::Unsigned);
    }
}
