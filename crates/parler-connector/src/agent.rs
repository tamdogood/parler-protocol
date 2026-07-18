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
    Attention, BundleRef, ClientFrame, DirectoryEntry, DiscoverScope, EndpointKind, Fact, FileRef,
    JoinRequest, MessageSig, Part, RecallHit, RoomInfo, RoomKind, RosterEntry, ServerFrame,
    StoredMessage, Target, Visibility,
};
use std::time::{Duration, Instant};

/// How long a long-poll chunk parks server-side before the client sends the next liveness `Ping`. A
/// half-open transport is therefore detected within one interval. Sized under typical Fly/Caddy proxy
/// idle windows (~60s) so a chunk + its ping always completes before an intermediary would cull the
/// socket, with jitter added per-agent so a fleet doesn't beat in lockstep.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(25);

/// How long to wait for the `Pong` before declaring the connection zombied and reconnecting. Short
/// relative to [`HEARTBEAT_INTERVAL`]: a live hub answers a `Ping` in well under a second.
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

/// Upper bound on the random jitter shaved off a heartbeat chunk (0–5s), so a fleet that all start
/// long-polling at once don't beat in lockstep against a shared proxy. Cheap source: a fresh v4 UUID
/// byte (no extra `rand` dependency), which is uniform enough for de-synchronizing timers.
fn heartbeat_jitter() -> Duration {
    let b = uuid::Uuid::new_v4().as_bytes()[0]; // 0..=255
    Duration::from_millis((b as u64) * 5000 / 255) // → 0..=5000 ms
}

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

/// The trailing path component of `name`, stripping any directory prefix (either separator). Keeps a
/// file's advertised name a bare basename so it can't smuggle a path into a peer's save suggestion.
fn basename(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or(name)
}

/// A connected, authenticated agent on the mesh.
pub struct MeshAgent {
    transport: Box<dyn MeshTransport>,
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub hub_url: String,
    /// The local identity (nkey seed) — present when connected from a [`Config`]; required to sign a
    /// discovery card in [`MeshAgent::register`], and to re-authenticate on a transparent reconnect.
    identity: Option<Identity>,
    /// Whether the current transport holds a live push subscription, so [`MeshAgent::reconnect`] can
    /// restore it after a dropped connection.
    subscribed: bool,
    /// Deferred-ack high-water per room (#85): the highest cursor the hub reported for a room, sent
    /// as the `ack` on this room's next pull so the hub commits the previous batch only after we've
    /// durably received it. Persists across a transparent reconnect (it's client state), so a resumed
    /// connection acks correctly; a fresh process starts empty and re-reads at most the last batch.
    pending_ack: std::collections::HashMap<String, i64>,
    /// Per-room high-water that [`MeshAgent::commit_reads`] has already flushed to the hub, so a
    /// repeat commit (or a `flush_acks` over a room a recv already committed) skips a redundant round
    /// trip. Only ever set to a `pending_ack` value we durably committed, so `committed >= pending`
    /// can only mean the hub's monotonic cursor is already at least there — never a lost commit.
    committed_ack: std::collections::HashMap<String, i64>,
    /// Rooms whose pending cursor must not ride an ordinary subsequent pull yet. Attention-aware
    /// runtimes set this while a batch is held or while a host wake is still being injected; the
    /// high-water remains available for an explicit successful [`MeshAgent::commit_reads`].
    held_reads: std::collections::HashSet<String>,
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
            subscribed: false,
            pending_ack: std::collections::HashMap::new(),
            committed_ack: std::collections::HashMap::new(),
            held_reads: std::collections::HashSet::new(),
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
        MeshAgent {
            transport,
            id,
            name,
            role,
            hub_url,
            identity: None,
            subscribed: false,
            pending_ack: std::collections::HashMap::new(),
            committed_ack: std::collections::HashMap::new(),
            held_reads: std::collections::HashSet::new(),
        }
    }

    /// Like [`MeshAgent::with_transport`], but carries a real `identity` + `hub_url`, so the
    /// transparent-reconnect path (idle-timeout drop, half-open heartbeat) is exercisable: on a lost
    /// connection this agent rebuilds a fresh [`HubClient`] against `hub_url`, exactly as
    /// [`MeshAgent::connect`] would. Used by liveness tests that wrap a real client in a fault-injecting
    /// transport and then verify the agent heals itself.
    pub fn with_transport_and_identity(
        transport: Box<dyn MeshTransport>,
        identity: Identity,
        name: String,
        role: Option<String>,
        hub_url: String,
    ) -> MeshAgent {
        MeshAgent {
            transport,
            id: identity.id.clone(),
            name,
            role,
            hub_url,
            identity: Some(identity),
            subscribed: false,
            pending_ack: std::collections::HashMap::new(),
            committed_ack: std::collections::HashMap::new(),
            held_reads: std::collections::HashSet::new(),
        }
    }

    /// Run one request/reply, transparently reconnecting and retrying **once** if the connection was
    /// lost (an idle-timeout drop, a network blip). Room membership and read cursors are durable on
    /// the hub, so the resumed connection is already in the same rooms — no re-join, no re-approval.
    /// Only attempted when we hold a local identity to re-authenticate with; an identity-less test
    /// agent surfaces the error unchanged. A retried `Send` no longer risks a double-post (#86): the
    /// frame is cloned verbatim for the retry, so it carries the *same* `client_id` idempotency key,
    /// and the hub's `(room, author, client_id)` unique index returns the original message on a
    /// replay whose first attempt had already landed — at-least-once delivery, exactly-once effect.
    async fn request(&mut self, frame: ClientFrame) -> Result<ServerFrame> {
        match self.transport.request(frame.clone()).await {
            Err(e) if self.reconnectable(&e) => {
                self.reconnect().await?;
                self.transport.request(frame).await
            }
            other => other,
        }
    }

    /// True when `e` is a lost-connection ([`crate::client::Disconnected`]) error *and* we hold an
    /// identity to re-authenticate with. A hub *application* error (e.g. "not a member") is not
    /// reconnectable — it would fail identically on a fresh connection.
    fn reconnectable(&self, e: &anyhow::Error) -> bool {
        self.identity.is_some() && e.downcast_ref::<crate::client::Disconnected>().is_some()
    }

    /// Rebuild the transport against the same identity + hub and restore the push subscription, so an
    /// idle-timeout disconnect is invisible to the caller.
    async fn reconnect(&mut self) -> Result<()> {
        let identity = self
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("cannot reconnect without a local identity"))?;
        let client =
            HubClient::connect(&self.hub_url, identity, &self.name, self.role.as_deref()).await?;
        self.transport = Box::new(client);
        // Restore the push subscription on the fresh socket, and keep `subscribed` honest: if the
        // re-subscribe fails, push is *not* live on the new connection, so record that (the MCP layer
        // queries `push_active()` and must not believe a dead subscription is up). Server-side wait
        // still works without push, so falling back to `subscribed = false` is safe, not degrading.
        if self.subscribed {
            self.subscribed = self.transport.subscribe().await.unwrap_or(false);
        }
        Ok(())
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
            .request(ClientFrame::Invite { kind, room, ttl_secs, max_uses, require_approval })
            .await?
        {
            ServerFrame::Invited { code, url, room, kind, expires_at } => {
                Ok(Invite { code, url, room, kind, expires_at })
            }
            other => Err(crate::unexpected_reply("create the invite", &other)),
        }
    }

    /// Redeem a pasted code/link, distinguishing an immediate join from an approval-gated
    /// [`JoinOutcome::Pending`] (the owner must approve before the caller is admitted).
    pub async fn redeem(&mut self, code: &str) -> Result<JoinOutcome> {
        match self.request(ClientFrame::Redeem { code: code.to_string() }).await? {
            ServerFrame::Joined { room, kind } => Ok(JoinOutcome::Joined { room, kind }),
            ServerFrame::JoinPending { room } => Ok(JoinOutcome::Pending { room }),
            other => Err(crate::unexpected_reply("redeem the code", &other)),
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
        match self.request(ClientFrame::JoinRequests { room: room.to_string() }).await? {
            ServerFrame::JoinRequests { requests, .. } => Ok(requests),
            other => Err(crate::unexpected_reply("list join requests", &other)),
        }
    }

    /// Approve (`approve = true`) or deny a pending join request for a room you own. On approval the
    /// requester is admitted; on denial it is rejected and cannot re-request. Returns whether the
    /// requester was admitted.
    pub async fn resolve_join(&mut self, room: &str, agent: &str, approve: bool) -> Result<bool> {
        match self
            .request(ClientFrame::ResolveJoin {
                room: room.to_string(),
                agent: agent.to_string(),
                approve,
            })
            .await?
        {
            ServerFrame::JoinResolved { approved, .. } => Ok(approved),
            other => Err(crate::unexpected_reply("resolve the join request", &other)),
        }
    }

    /// Join/create a service room as a worker, so it can receive (`pull`) tasks sent to the service.
    pub async fn serve(&mut self, service: &str) -> Result<String> {
        match self.request(ClientFrame::Serve { service: service.to_string() }).await? {
            ServerFrame::Joined { room, .. } => Ok(room),
            other => Err(crate::unexpected_reply("register as a worker", &other)),
        }
    }

    /// Atomically claim one role-dispatched message in a service room. `Some(lease_until)` means
    /// this worker owns (or renewed) the lease; `None` is the ordinary anycast outcome where another
    /// available worker already owns it.
    pub async fn claim(&mut self, room: &str, message: &str, lease_secs: Option<u64>) -> Result<Option<i64>> {
        match self
            .request(ClientFrame::Claim {
                room: room.to_string(),
                message: message.to_string(),
                lease_secs,
            })
            .await?
        {
            ServerFrame::Claimed { claimed, lease_until, .. } => Ok(claimed.then_some(lease_until).flatten()),
            other => Err(crate::unexpected_reply("claim the role task", &other)),
        }
    }

    /// Read work that is ready for this served role without changing the ordinary room cursor. Queue
    /// reads are separate from [`MeshAgent::pull`]: they include an expired lease a restarted worker
    /// must discover even when its broadcast-room cursor already passed the request.
    pub async fn queue(&mut self, room: &str, role: &str, limit: Option<u32>) -> Result<Vec<StoredMessage>> {
        match self
            .request(ClientFrame::Queue { room: room.to_string(), role: role.to_string(), limit })
            .await?
        {
            ServerFrame::Queued { messages, .. } => Ok(messages),
            other => Err(crate::unexpected_reply("read the role queue", &other)),
        }
    }

    /// Mark this worker's live claim terminal. Returns `false` if the lease expired and another
    /// worker already took the task, so a late child process cannot overwrite its successor's result.
    pub async fn complete_claim(
        &mut self,
        room: &str,
        message: &str,
        status: parler_protocol::TaskStatus,
    ) -> Result<bool> {
        match self
            .request(ClientFrame::Complete { room: room.to_string(), message: message.to_string(), status })
            .await?
        {
            ServerFrame::Completed { completed, .. } => Ok(completed),
            other => Err(crate::unexpected_reply("complete the role task", &other)),
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
        // Idempotency key generated once per logical send. `request` clones the frame for its
        // reconnect-and-retry-once path, so the *same* client_id rides the retry — if the first
        // attempt already reached the hub before the drop, the hub returns that original message
        // rather than posting a duplicate (#86).
        let client_id = Some(uuid::Uuid::new_v4().to_string());
        match self
            .request(ClientFrame::Send { target, parts, mentions, reply_to, client_id })
            .await?
        {
            ServerFrame::Sent { id, seq, room } => Ok((id, seq, room)),
            other => Err(crate::unexpected_reply("send the message", &other)),
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
        let blob_id = self.put_blob(&target, bundle, meta.media_type.clone()).await?;
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

    /// Transfer an arbitrary **file** to `target`: upload the bytes to the hub's content-addressed
    /// blob store (bound to the room `target` resolves to), then post a room message carrying a
    /// `com.parler.file` reference so peers see it through the ordinary `recv` and can pull the bytes
    /// with [`MeshAgent::fetch_blob`]. The download path is identical to a bundle's — it's the same
    /// content-addressed blob — so the same file sent to several agents (or re-sent) is stored once.
    /// `name` is the file's basename (what a receiver saves it back as); `note` is an optional text
    /// part shown alongside the reference.
    pub async fn send_file(
        &mut self,
        target: Target,
        name: &str,
        bytes: &[u8],
        media_type: Option<String>,
        note: Option<String>,
    ) -> Result<PushReceipt> {
        let blob_id = self.put_blob(&target, bytes, media_type.clone()).await?;
        let fref = FileRef {
            blob: blob_id.clone(),
            name: basename(name).to_string(),
            size: bytes.len() as u64,
            media_type,
            summary: None,
        };
        let mut parts = Vec::new();
        if let Some(n) = note {
            if !n.is_empty() {
                parts.push(Part::text(n));
            }
        }
        parts.push(fref.to_part());
        let (msg_id, seq, room) = self.send(target, parts, None, None).await?;
        Ok(PushReceipt { msg_id, blob_id, room, seq })
    }

    /// Upload `bytes` to the hub's content-addressed blob store (bound to the room `target` resolves
    /// to) and return their content id. The transport reconnects once on a droppable error. Shared by
    /// [`MeshAgent::push`] and [`MeshAgent::send_file`] — the two differ only in the reference part
    /// they post afterward.
    async fn put_blob(
        &mut self,
        target: &Target,
        bytes: &[u8],
        media_type: Option<String>,
    ) -> Result<String> {
        let blob_id = parler_auth::content_id(bytes);
        let put = ClientFrame::PutBlob {
            target: target.clone(),
            sha256: blob_id.clone(),
            size: bytes.len() as u64,
            media_type,
        };
        let stored = match self.transport.upload_blob(put.clone(), bytes).await {
            Err(e) if self.reconnectable(&e) => {
                self.reconnect().await?;
                self.transport.upload_blob(put, bytes).await
            }
            other => other,
        }?;
        match stored {
            ServerFrame::BlobStored { id, .. } if id == blob_id => Ok(blob_id),
            ServerFrame::BlobStored { id, .. } => bail!("hub stored a different blob id: {id}"),
            other => Err(crate::unexpected_reply("upload the data", &other)),
        }
    }

    /// Download a blob's bytes by its content id (as carried in a `com.parler.bundle` part).
    pub async fn fetch_blob(&mut self, id: &str) -> Result<Vec<u8>> {
        let get = ClientFrame::GetBlob { id: id.to_string() };
        match self.transport.download_blob(get.clone()).await {
            Err(e) if self.reconnectable(&e) => {
                self.reconnect().await?;
                self.transport.download_blob(get).await
            }
            other => other,
        }
    }

    /// Pull new messages for `room` (past the agent's cursor, which this advances), or past `since`
    /// (which does not). Returns the messages and the resulting cursor.
    pub async fn pull(
        &mut self,
        room: &str,
        since: Option<i64>,
        limit: Option<u32>,
    ) -> Result<(Vec<StoredMessage>, i64)> {
        // Cursor reads carry a deferred ack (#85); a `since` re-read is a pure read and never acks.
        let ack = if since.is_none() { self.ack_for(room) } else { None };
        match self
            .request(ClientFrame::Pull { room: room.to_string(), since, limit, wait_secs: None, ack })
            .await?
        {
            ServerFrame::Pulled { messages, cursor, .. } => {
                if since.is_none() {
                    self.record_ack(room, cursor);
                }
                Ok((messages, cursor))
            }
            other => Err(crate::unexpected_reply("pull messages", &other)),
        }
    }

    /// The deferred ack to send on the next cursor pull of `room` (#85): the highest cursor the hub
    /// has reported for it (0 until the first batch). Always `Some` — this client is ack-aware, so the
    /// hub commits a batch only once we've acked it; a bare `Some(0)` on the first pull is a no-op
    /// advance (monotonic max) that still opts into no-advance-on-read.
    fn ack_for(&self, room: &str) -> Option<i64> {
        Some(if self.held_reads.contains(room) {
            0
        } else {
            self.pending_ack.get(room).copied().unwrap_or(0)
        })
    }

    /// Record the cursor the hub reported for `room`, so the next pull acks up to it. On an empty pull
    /// the hub echoes the same cursor, so this is a no-op then.
    fn record_ack(&mut self, room: &str, cursor: i64) {
        self.pending_ack.insert(room.to_string(), cursor);
    }

    /// Commit the deferred read cursor for `room` (#85): flush the pending ack in one ack-only pull so
    /// the hub durably advances `members.cursor` past the batch we've already received.
    ///
    /// The invariant is that **a cursor may only advance past a batch the client has already
    /// received**. [`MeshAgent::pull`] defers that commit to the *next* pull's `ack` — correct for a
    /// long-lived client, but a one-shot process (a `parler` CLI invocation, an MCP cold start) does
    /// its single pull and exits with the ack stranded in memory, leaving the hub's cursor stuck and
    /// the whole history re-read next time. Call this at a **consumption boundary** — a batch rendered
    /// to a terminal, or returned to an MCP host — to commit it durably.
    ///
    /// A pure ack commit: `Pull { since: None, limit: Some(0), ack: Some(pending) }` makes the store
    /// apply the ack *before* the read, so `LIMIT 0` reads nothing and `ack.is_some()` suppresses
    /// advance-on-read. Routed through the reconnecting `request` path so a dropped connection heals
    /// and retries. A no-op `Ok(())` when nothing new has been received or this high-water is already
    /// committed.
    pub async fn commit_reads(&mut self, room: &str) -> Result<()> {
        let pending = self.pending_ack.get(room).copied().unwrap_or(0);
        self.commit_reads_through(room, pending).await
    }

    /// Commit a cursor returned by any successful [`MeshAgent::pull`], including a paginated
    /// `since` read. This is the bounded catch-up primitive: a consumer may inspect several pure
    /// read pages, hand their retained context to a host, then atomically advance through the last
    /// page only after that host accepts it.
    ///
    /// Callers must pass only a cursor they actually received for this room. The hub applies the ack
    /// monotonically, so retries and an older duplicate commit cannot move the member backward.
    pub async fn commit_reads_through(&mut self, room: &str, cursor: i64) -> Result<()> {
        if cursor <= 0 || self.committed_ack.get(room).copied().unwrap_or(0) >= cursor {
            self.held_reads.remove(room);
            return Ok(());
        }
        match self
            .request(ClientFrame::Pull {
                room: room.to_string(),
                since: None,
                limit: Some(0),
                wait_secs: None,
                ack: Some(cursor),
            })
            .await?
        {
            ServerFrame::Pulled { .. } => {
                self.pending_ack
                    .entry(room.to_string())
                    .and_modify(|pending| *pending = (*pending).max(cursor))
                    .or_insert(cursor);
                self.committed_ack.insert(room.to_string(), cursor);
                self.held_reads.remove(room);
                Ok(())
            }
            other => Err(crate::unexpected_reply("commit reads", &other)),
        }
    }

    /// Keep the last pulled batch unacknowledged. Attention adapters call this for a quiet/focus
    /// hold or while a host wake is in flight: subsequent pulls send `ack: 0`, deliberately re-read
    /// the durable batch, and retain its high-water for an explicit successful [`MeshAgent::commit_reads`].
    pub fn defer_reads(&mut self, room: &str) {
        self.held_reads.insert(room.to_string());
    }

    /// Best-effort [`MeshAgent::commit_reads`] for every room with a pending ack — call on an exit path
    /// (the MCP stdio run loop shutting down) so a cursor advanced by auto-pull-on-send isn't lost with
    /// the process. Errors are swallowed: a missed commit just re-reads the last batch on the next
    /// start, the documented at-least-once behavior.
    pub async fn flush_acks(&mut self) {
        let rooms: Vec<String> = self
            .pending_ack
            .keys()
            .filter(|room| !self.held_reads.contains(*room))
            .cloned()
            .collect();
        for room in rooms {
            let _ = self.commit_reads(&room).await;
        }
    }

    /// **Long-poll** for new messages in `room`: like [`MeshAgent::pull`], but if the backlog is empty
    /// the *hub* parks the request (server-side wait, see `Pull { wait_secs }`) and replies the moment
    /// a peer message lands or the wait window closes — so this works with **zero push machinery**
    /// (even on a connection whose `Subscribe` failed). Returns `(messages, cursor, waited)`; `waited`
    /// is `true` when a server-side wait actually occurred (the hub honored `wait_secs`), `false` when
    /// the first pull already had messages or the hub is too old to park (so the caller can honestly
    /// report a genuinely-degraded poll).
    ///
    /// Liveness during the wait: the total budget is split into heartbeat-sized chunks, and a `Ping`
    /// is sent before each chunk. A half-open/dead transport is caught when that ping times out (or the
    /// parked pull errors) and is transparently reconnected via the existing retry path — the caller
    /// sees no error, and the next chunk runs on the fresh connection. Cursor semantics are the plain
    /// [`MeshAgent::pull`]'s: the cursor advances only through the returned batch.
    pub async fn pull_wait(
        &mut self,
        room: &str,
        limit: Option<u32>,
        wait_secs: u64,
    ) -> Result<(Vec<StoredMessage>, i64, bool)> {
        let deadline = Instant::now() + Duration::from_secs(wait_secs);
        let mut waited = false;
        loop {
            let now = Instant::now();
            let remaining = deadline.saturating_duration_since(now);
            // Heartbeat: prove the socket is alive (and force a reconnect if it's half-open) *before*
            // asking the hub to hold the request open for a chunk. On the very first iteration this
            // also front-runs the pull, so a dead connection is healed before we even wait.
            self.heartbeat().await;
            let (msgs, cursor) = self.pull(room, None, limit).await?;
            if !msgs.is_empty() {
                return Ok((msgs, cursor, waited));
            }
            if remaining.is_zero() {
                return Ok((msgs, cursor, waited)); // budget exhausted — an honest empty result
            }
            // Park server-side for up to one heartbeat chunk (bounded by the remaining budget). An old
            // hub ignores `wait_secs` and replies immediately with an empty batch — detected as a pull
            // that returns instantly, so we don't spin: fall back to a client sleep for the chunk.
            // Jitter the chunk per-iteration so a fleet of agents doesn't ping the proxy in lockstep.
            let chunk = remaining.min(HEARTBEAT_INTERVAL.saturating_sub(heartbeat_jitter()));
            let before = Instant::now();
            let (m, c) = self.pull_parked(room, limit, chunk.as_secs().max(1)).await?;
            if !m.is_empty() {
                return Ok((m, c, true));
            }
            // The hub parked (it held the request roughly the whole chunk) ⇒ server-side wait works.
            // If it returned near-instantly on an empty room, it's an old hub that ignored the field —
            // sleep out the chunk client-side so we still respect the budget without a busy loop.
            if before.elapsed() >= chunk / 2 {
                waited = true;
            } else {
                tokio::time::sleep(chunk).await;
            }
        }
    }

    /// One parked `Pull { wait_secs }` round-trip (server-side wait). Split out so [`MeshAgent::pull_wait`]
    /// can time it (to tell a real park from an old hub's instant empty reply).
    async fn pull_parked(
        &mut self,
        room: &str,
        limit: Option<u32>,
        wait_secs: u64,
    ) -> Result<(Vec<StoredMessage>, i64)> {
        let ack = self.ack_for(room);
        match self
            .request(ClientFrame::Pull {
                room: room.to_string(),
                since: None,
                limit,
                wait_secs: Some(wait_secs),
                ack,
            })
            .await?
        {
            ServerFrame::Pulled { messages, cursor, .. } => {
                self.record_ack(room, cursor);
                Ok((messages, cursor))
            }
            other => Err(crate::unexpected_reply("parked pull", &other)),
        }
    }

    /// A liveness heartbeat: send a protocol `Ping` and expect a `Pong` within the heartbeat timeout.
    /// A missed pong means the transport is half-open (a proxy silently dropped it) — we mark the
    /// connection lost and reconnect, so the *next* op runs on a fresh socket. Best-effort and
    /// self-healing: the caller never sees an error (a genuinely-unreachable hub surfaces on the next
    /// real request instead). No-op success against a transport with no identity to reconnect with.
    pub async fn heartbeat(&mut self) {
        match tokio::time::timeout(HEARTBEAT_TIMEOUT, self.transport.request(ClientFrame::Ping)).await {
            // Got a reply in time (a `Pong`, or any frame) — the socket is alive.
            Ok(Ok(_)) => {}
            // The ping errored (socket already gone) — reconnect if we can; ignore the outcome, the
            // caller's next request will retry on whatever connection we end up with.
            Ok(Err(_)) => {
                let _ = self.reconnect().await;
            }
            // No pong before the deadline: the connection is zombied (half-open). Force a fresh one.
            Err(_) => {
                let _ = self.reconnect().await;
            }
        }
    }

    /// Ask the hub to **push** new room messages to this connection (sub-second delivery), instead of
    /// waiting for the next [`MeshAgent::pull`]. Returns `true` if the hub supports push, `false`
    /// otherwise (an older hub) — in which case keep using `pull`. The subscription covers every room
    /// the agent belongs to now or joins later, and ends when the connection drops.
    pub async fn subscribe(&mut self) -> Result<bool> {
        let ok = self.transport.subscribe().await?;
        self.subscribed = ok;
        Ok(ok)
    }

    /// Whether this connection currently holds a live push subscription — the authoritative,
    /// *live* answer (updated by [`MeshAgent::subscribe`] and on reconnect), so callers query
    /// this instead of caching a boolean at startup (a startup cache goes stale after a reconnect that
    /// re-subscribed, or after a retried subscribe that finally succeeded).
    pub fn push_active(&self) -> bool {
        self.subscribed
    }

    /// If we're not currently subscribed, try once to (re)subscribe — for the case where the initial
    /// `Subscribe` failed (an old hub, or a transient error) but a later attempt could succeed. Cheap
    /// and best-effort: on success `push_active()` flips to `true` and pushes start flowing; on failure
    /// the state is unchanged and the caller falls back to server-side wait / polling. Returns the
    /// resulting push state.
    pub async fn resubscribe_if_needed(&mut self) -> bool {
        if !self.subscribed {
            let _ = self.subscribe().await;
        }
        self.subscribed
    }

    /// Block up to `max_wait` for the next pushed message (a peer's, never your own); `None` on
    /// timeout. Only meaningful after [`MeshAgent::subscribe`] returned `true`. A push is best-effort
    /// and does **not** advance the durable cursor, so the idiomatic use is "block here to wake
    /// promptly, then [`MeshAgent::pull`] to read + advance authoritatively" (which also dedups).
    pub async fn next_delivery(&mut self, max_wait: Duration) -> Result<Option<StoredMessage>> {
        match self.transport.next_delivery(max_wait).await {
            // A drop during a long-poll: reconnect now and report "nothing yet" — the caller's next
            // pull runs on the fresh connection and returns anything it missed.
            Err(e) if self.reconnectable(&e) => {
                self.reconnect().await?;
                Ok(None)
            }
            other => other,
        }
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
            .request(ClientFrame::Remember {
                fact: Fact { key, text: text.to_string(), room },
                embedding,
                embedding_model,
            })
            .await?
        {
            ServerFrame::Remembered { .. } => Ok(()),
            other => Err(crate::unexpected_reply("save the fact", &other)),
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
            .request(ClientFrame::Recall { query: query.to_string(), room, limit, embedding, key: None })
            .await?
        {
            ServerFrame::Recalled { hits } => Ok(hits),
            other => Err(crate::unexpected_reply("search memory", &other)),
        }
    }

    /// Deterministic keyed fact fetch (#91): the exact fact stored under `key`, skipping BM25 on a
    /// current hub. `fallback_query` is sent as the ordinary BM25 `query` so an **older** hub — which
    /// ignores the unknown `key` field — degrades to a full-text search instead of failing; the caller
    /// verifies the returned hit is genuinely the keyed fact (guarding that fallback's false positives).
    pub async fn recall_keyed(
        &mut self,
        key: &str,
        fallback_query: &str,
        room: Option<String>,
        limit: Option<u32>,
    ) -> Result<Vec<RecallHit>> {
        match self
            .request(ClientFrame::Recall {
                query: fallback_query.to_string(),
                room,
                limit,
                embedding: None,
                key: Some(key.to_string()),
            })
            .await?
        {
            ServerFrame::Recalled { hits } => Ok(hits),
            other => Err(crate::unexpected_reply("search memory", &other)),
        }
    }

    /// List the rooms the agent belongs to (with unread counts).
    pub async fn rooms(&mut self) -> Result<Vec<RoomInfo>> {
        match self.request(ClientFrame::Rooms).await? {
            ServerFrame::Rooms { rooms } => Ok(rooms),
            other => Err(crate::unexpected_reply("list your rooms", &other)),
        }
    }

    /// Permanently delete a room this agent owns.
    pub async fn delete_room(&mut self, room: &str) -> Result<()> {
        match self.request(ClientFrame::DeleteRoom { room: room.to_string() }).await? {
            ServerFrame::RoomDeleted { .. } => Ok(()),
            other => Err(crate::unexpected_reply("delete the room", &other)),
        }
    }

    /// The members + presence of a room.
    pub async fn roster(&mut self, room: &str) -> Result<Vec<RosterEntry>> {
        match self.request(ClientFrame::Roster { room: room.to_string() }).await? {
            ServerFrame::Roster { entries, .. } => Ok(entries),
            other => Err(crate::unexpected_reply("read the roster", &other)),
        }
    }

    /// Advertise presence (status + optional activity line).
    pub async fn presence(&mut self, status: &str, activity: Option<String>) -> Result<()> {
        self.presence_with_attention(status, activity, None).await
    }

    /// Advertise lifecycle status plus the optional receiver-side attention mode peers may observe.
    /// The policy is advisory metadata: it never changes membership or hub delivery guarantees.
    pub async fn presence_with_attention(
        &mut self,
        status: &str,
        activity: Option<String>,
        attention: Option<Attention>,
    ) -> Result<()> {
        match self
            .request(ClientFrame::Presence { status: status.to_string(), activity, attention })
            .await?
        {
            ServerFrame::PresenceOk => Ok(()),
            other => Err(crate::unexpected_reply("update presence", &other)),
        }
    }

    /// Update the globally visible attention preference without replacing the host's current
    /// lifecycle status or activity line.
    pub async fn set_attention(&mut self, attention: Attention) -> Result<()> {
        match self.request(ClientFrame::SetAttention { attention }).await? {
            ServerFrame::AttentionOk => Ok(()),
            other => Err(crate::unexpected_reply("update attention", &other)),
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
            .request(ClientFrame::Register { card, visibility, sig: Some(sig) })
            .await?
        {
            ServerFrame::Registered { visibility, verified, .. } => Ok((visibility, verified)),
            other => Err(crate::unexpected_reply("publish your card", &other)),
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
            .request(ClientFrame::Discover { scope, query, tag, skill, status, limit })
            .await?
        {
            ServerFrame::Directory { agents } => Ok(agents),
            other => Err(crate::unexpected_reply("search the directory", &other)),
        }
    }

    /// Fetch a single agent's directory card by id.
    pub async fn lookup(&mut self, id: &str) -> Result<Option<DirectoryEntry>> {
        match self.request(ClientFrame::Lookup { id: id.to_string() }).await? {
            ServerFrame::Card { entry } => Ok(entry),
            other => Err(crate::unexpected_reply("look up the agent", &other)),
        }
    }

    /// Mint a read-scoped, expiring directory token (paste into the website to view a private hub).
    pub async fn mint_directory_token(&mut self, ttl_secs: Option<u64>) -> Result<(String, i64)> {
        match self.request(ClientFrame::MintDirectoryToken { ttl_secs }).await? {
            ServerFrame::DirectoryToken { token, expires_at } => Ok((token, expires_at)),
            other => Err(crate::unexpected_reply("mint the directory token", &other)),
        }
    }

    /// Mint a read-only **watch** token for a session/room you **own** — the bearer you paste into the
    /// website's session viewer to watch the conversation and how many agents are in it, without joining.
    /// The hub authorizes this to the room owner only. Returns `(token, expires_at)`.
    pub async fn mint_watch_token(&mut self, room: &str, ttl_secs: Option<u64>) -> Result<(String, i64)> {
        match self
            .request(ClientFrame::MintWatch { room: room.to_string(), ttl_secs })
            .await?
        {
            ServerFrame::Watch { token, expires_at, .. } => Ok((token, expires_at)),
            other => Err(crate::unexpected_reply("mint the watch code", &other)),
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

/// Verify a stored message and bind the author's signed routing intent to the room where the hub
/// delivered it. This is the verification boundary for any action that can wake a host or run a
/// local worker: a valid signature alone does not stop an untrusted relay from copying that valid
/// message into a different room.
///
/// Channel targets name their concrete room. Service targets resolve deterministically to
/// `svc.<token>`. DM rooms have random names, so their binding is the signed recipient plus the
/// authenticated sender; the receiver also requires the hub record to be a DM room name.
pub fn verify_message_for_room(message: &StoredMessage, recipient_id: &str) -> SigStatus {
    let status = verify_message(&message.from.id, &message.parts, message.reply_to.as_deref());
    if status != SigStatus::Valid {
        return status;
    }
    let Some(signature) = MessageSig::from_parts(&message.parts) else {
        return SigStatus::Invalid;
    };
    let routed_here = match signature.target {
        Target::Room { room } => room == message.room,
        Target::Service { service } => message.room == format!("svc.{}", parler_protocol::token(&service)),
        Target::Dm { agent } => agent == recipient_id && message.room.starts_with("dm."),
    };
    if routed_here { SigStatus::Valid } else { SigStatus::Invalid }
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
    use parler_protocol::EndpointRef;

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
    fn autonomous_verification_rejects_cross_room_replay() {
        let alice = parler_auth::new_identity().unwrap();
        let parts = signed(
            &alice,
            Target::Room { room: "room-a".into() },
            &[Part::text("run the task")],
            None,
        );
        let mut message = StoredMessage {
            seq: 1,
            id: "m1".into(),
            room: "room-a".into(),
            from: EndpointRef { id: alice.id.clone(), name: "alice".into(), role: None },
            parts,
            mentions: None,
            reply_to: None,
            ts: 1,
        };
        assert_eq!(verify_message_for_room(&message, "UBOB"), SigStatus::Valid);
        message.room = "room-b".into();
        assert_eq!(verify_message_for_room(&message, "UBOB"), SigStatus::Invalid);
    }

    #[test]
    fn autonomous_verification_binds_service_and_dm_targets() {
        let alice = parler_auth::new_identity().unwrap();
        let bob = parler_auth::new_identity().unwrap();
        let endpoint = EndpointRef { id: alice.id.clone(), name: "alice".into(), role: None };
        let service = StoredMessage {
            seq: 1,
            id: "service".into(),
            room: "svc.code_review".into(),
            from: endpoint.clone(),
            parts: signed(
                &alice,
                Target::Service { service: "code review".into() },
                &[Part::text("review")],
                None,
            ),
            mentions: None,
            reply_to: None,
            ts: 1,
        };
        assert_eq!(verify_message_for_room(&service, &bob.id), SigStatus::Valid);

        let mut dm = StoredMessage {
            seq: 2,
            id: "dm".into(),
            room: "dm.random".into(),
            from: endpoint,
            parts: signed(
                &alice,
                Target::Dm { agent: bob.id.clone() },
                &[Part::text("private")],
                None,
            ),
            mentions: None,
            reply_to: None,
            ts: 1,
        };
        assert_eq!(verify_message_for_room(&dm, &bob.id), SigStatus::Valid);
        assert_eq!(verify_message_for_room(&dm, "UMALLORY"), SigStatus::Invalid);
        dm.room = "team".into();
        assert_eq!(verify_message_for_room(&dm, &bob.id), SigStatus::Invalid);
    }

    #[test]
    fn no_signature_part_is_unsigned() {
        let alice = parler_auth::new_identity().unwrap();
        assert_eq!(verify_message(&alice.id, &[Part::text("legacy")], None), SigStatus::Unsigned);
    }

    // ---- #86: idempotent Send across a reply-lost retry --------------------------------------

    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::Arc;

    /// A transport that, once armed, **forwards** the frame to the hub (so it really lands) and then
    /// returns a `Disconnected` error — exactly the "request reached the hub, reply was lost" race.
    /// `MeshAgent::request` then reconnects (replacing this decorator with a fresh real client) and
    /// retries the *same* frame, so the retry carries the same client_id.
    struct ReplyLostOnce {
        inner: Box<dyn crate::MeshTransport>,
        armed: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl crate::MeshTransport for ReplyLostOnce {
        async fn request(&mut self, frame: ClientFrame) -> Result<ServerFrame> {
            if self.armed.swap(false, AtomicOrdering::SeqCst) {
                let _ = self.inner.request(frame).await; // it reaches the hub…
                return Err(anyhow::Error::new(crate::client::Disconnected)); // …but the reply is "lost"
            }
            self.inner.request(frame).await
        }
        async fn subscribe(&mut self) -> Result<bool> {
            self.inner.subscribe().await
        }
        async fn next_delivery(&mut self, max_wait: std::time::Duration) -> Result<Option<StoredMessage>> {
            self.inner.next_delivery(max_wait).await
        }
    }

    async fn start_hub() -> String {
        let store = parler_hub::Store::open(None).unwrap();
        let state = Arc::new(parler_hub::HubState::new(
            store,
            "parler://test".into(),
            "Test".into(),
            parler_hub::HubMode::Private,
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = parler_hub::serve(listener, state).await;
        });
        format!("ws://{addr}")
    }

    #[tokio::test]
    async fn retry_after_lost_reply_does_not_double_post() {
        let hub = start_hub().await;

        // Alice opens a channel and hands bob an invite; bob joins so both share a room.
        let mut alice = MeshAgent::connect(&Config {
            hub_url: hub.clone(),
            identity: parler_auth::new_identity().unwrap(),
            name: "alice".into(),
            role: None,
            attention: crate::AttentionPolicy::default(),
        })
        .await
        .unwrap();
        let inv = alice.invite(parler_protocol::RoomKind::Channel, Some("plan".into()), None, None).await.unwrap();
        let room = inv.room.clone();

        // Bob dials in over a real client wrapped in the (disarmed) reply-lost decorator.
        let bob_id = parler_auth::new_identity().unwrap();
        let inner = crate::client::HubClient::connect(&hub, &bob_id, "bob", None).await.unwrap();
        let armed = Arc::new(AtomicBool::new(false));
        let flaky = ReplyLostOnce { inner: Box::new(inner), armed: armed.clone() };
        let mut bob =
            MeshAgent::with_transport_and_identity(Box::new(flaky), bob_id, "bob".into(), None, hub.clone());
        bob.join(&inv.code).await.unwrap();

        // Arm the drop, then send: the first attempt lands on the hub, the reply is lost, and the
        // transparent retry re-sends the SAME frame (same client_id).
        armed.store(true, AtomicOrdering::SeqCst);
        let (id, _seq, sent_room) = bob.send_text(Target::Room { room: room.clone() }, "deploy now").await.unwrap();
        assert_eq!(sent_room, room);

        // Exactly one message exists despite the retry, and it's the id the caller got back.
        let (msgs, _cursor) = alice.pull(&room, None, None).await.unwrap();
        let deploys: Vec<_> = msgs
            .iter()
            .filter(|m| m.parts.iter().any(|p| matches!(p, Part::Text(t) if t == "deploy now")))
            .collect();
        assert_eq!(deploys.len(), 1, "retry after a lost reply must not double-post");
        assert_eq!(deploys[0].id, id, "the caller's success carries the original message id");
    }

    #[tokio::test]
    async fn pull_retry_after_lost_reply_redelivers_instead_of_skipping() {
        // #85: the reply to a Pull is lost after the hub read the batch. With advance-on-read the
        // batch would be skipped forever; with the ack model the cursor didn't commit, so the
        // transparent retry re-reads it and the caller still sees every message.
        let hub = start_hub().await;

        let mut alice = MeshAgent::connect(&Config {
            hub_url: hub.clone(),
            identity: parler_auth::new_identity().unwrap(),
            name: "alice".into(),
            role: None,
            attention: crate::AttentionPolicy::default(),
        })
        .await
        .unwrap();
        let inv = alice.invite(parler_protocol::RoomKind::Channel, Some("room".into()), None, None).await.unwrap();
        let room = inv.room.clone();

        let bob_id = parler_auth::new_identity().unwrap();
        let inner = crate::client::HubClient::connect(&hub, &bob_id, "bob", None).await.unwrap();
        let armed = Arc::new(AtomicBool::new(false));
        let flaky = ReplyLostOnce { inner: Box::new(inner), armed: armed.clone() };
        let mut bob =
            MeshAgent::with_transport_and_identity(Box::new(flaky), bob_id, "bob".into(), None, hub.clone());
        bob.join(&inv.code).await.unwrap();

        // Alice posts two messages while bob is idle.
        alice.send_text(Target::Room { room: room.clone() }, "first").await.unwrap();
        alice.send_text(Target::Room { room: room.clone() }, "second").await.unwrap();

        // Arm the drop, then pull: attempt 1 reaches the hub (reads the batch) but its reply is lost;
        // the transparent retry re-pulls and returns the batch — nothing is skipped.
        armed.store(true, AtomicOrdering::SeqCst);
        let (msgs, _cursor) = bob.pull(&room, None, None).await.unwrap();
        let texts: Vec<_> = msgs
            .iter()
            .flat_map(|m| m.parts.iter().filter_map(|p| match p {
                Part::Text(t) => Some(t.clone()),
                _ => None,
            }))
            .collect();
        assert!(texts.contains(&"first".to_string()) && texts.contains(&"second".to_string()),
            "the lost-reply pull re-delivered the batch: {texts:?}");

        // A subsequent pull acks the batch and returns nothing new (no perpetual redelivery).
        let (next, _c) = bob.pull(&room, None, None).await.unwrap();
        assert!(next.is_empty(), "the acked batch is committed on the next pull");
    }

    #[tokio::test]
    async fn pure_read_pages_can_commit_their_exact_received_cursor() {
        let hub = start_hub().await;
        let mut alice = MeshAgent::connect(&Config {
            hub_url: hub.clone(),
            identity: parler_auth::new_identity().unwrap(),
            name: "alice".into(),
            role: None,
            attention: crate::AttentionPolicy::default(),
        })
        .await
        .unwrap();
        let invitation = alice
            .invite(parler_protocol::RoomKind::Channel, Some("paged".into()), None, None)
            .await
            .unwrap();
        let mut bob = MeshAgent::connect(&Config {
            hub_url: hub,
            identity: parler_auth::new_identity().unwrap(),
            name: "bob".into(),
            role: None,
            attention: crate::AttentionPolicy::default(),
        })
        .await
        .unwrap();
        bob.join(&invitation.code).await.unwrap();
        alice
            .send_text(Target::Room { room: invitation.room.clone() }, "one")
            .await
            .unwrap();
        alice
            .send_text(Target::Room { room: invitation.room.clone() }, "two")
            .await
            .unwrap();

        let (messages, cursor) = bob.pull(&invitation.room, Some(0), Some(1)).await.unwrap();
        assert_eq!(messages.len(), 1);
        let (messages, cursor) = bob
            .pull(&invitation.room, Some(cursor), Some(1))
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        bob.commit_reads_through(&invitation.room, cursor).await.unwrap();
        let (remaining, _) = bob.pull(&invitation.room, None, None).await.unwrap();
        assert!(remaining.is_empty());
    }
}
