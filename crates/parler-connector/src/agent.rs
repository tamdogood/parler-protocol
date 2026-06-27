//! [`MeshAgent`] — the high-level API the CLI, MCP server, and Hermes plugin all call.
//!
//! Every method is one request/reply round-trip against the [`MeshTransport`]. The three send
//! patterns are just three [`Target`]s: a channel room (one-to-many), a peer DM (one-to-one), or a
//! service room (many-to-one).

use crate::{Config, HubClient, MeshTransport};
use anyhow::{bail, Result};
use parler_protocol::{
    ClientFrame, Fact, Part, RecallHit, RoomInfo, RoomKind, RosterEntry, ServerFrame, StoredMessage,
    Target,
};

/// A freshly minted invite — the code/link the human pastes to another agent.
pub struct Invite {
    pub code: String,
    pub url: String,
    pub room: String,
    pub kind: RoomKind,
    pub expires_at: i64,
}

/// A connected, authenticated agent on the mesh.
pub struct MeshAgent {
    transport: Box<dyn MeshTransport>,
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub hub_url: String,
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
        })
    }

    /// Build an agent over any transport (used by tests with an in-process transport).
    pub fn with_transport(
        transport: Box<dyn MeshTransport>,
        id: String,
        name: String,
        role: Option<String>,
        hub_url: String,
    ) -> MeshAgent {
        MeshAgent { transport, id, name, role, hub_url }
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
        match self
            .transport
            .request(ClientFrame::Invite { kind, room, ttl_secs, max_uses })
            .await?
        {
            ServerFrame::Invited { code, url, room, kind, expires_at } => {
                Ok(Invite { code, url, room, kind, expires_at })
            }
            other => bail!("unexpected reply to invite: {other:?}"),
        }
    }

    /// Redeem a pasted code/link — joins the room it grants.
    pub async fn join(&mut self, code: &str) -> Result<(String, RoomKind)> {
        match self.transport.request(ClientFrame::Redeem { code: code.to_string() }).await? {
            ServerFrame::Joined { room, kind } => Ok((room, kind)),
            other => bail!("unexpected reply to join: {other:?}"),
        }
    }

    /// Join/create a service room as a worker, so it can receive (`pull`) tasks sent to the service.
    pub async fn serve(&mut self, service: &str) -> Result<String> {
        match self.transport.request(ClientFrame::Serve { service: service.to_string() }).await? {
            ServerFrame::Joined { room, .. } => Ok(room),
            other => bail!("unexpected reply to serve: {other:?}"),
        }
    }

    /// Publish `parts` to a target.
    pub async fn send(
        &mut self,
        target: Target,
        parts: Vec<Part>,
        mentions: Option<Vec<String>>,
        reply_to: Option<String>,
    ) -> Result<(String, i64, String)> {
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

    /// Write a fact to the memory store (idempotent when `key` is set).
    pub async fn remember(&mut self, text: &str, key: Option<String>, room: Option<String>) -> Result<()> {
        match self
            .transport
            .request(ClientFrame::Remember { fact: Fact { key, text: text.to_string(), room } })
            .await?
        {
            ServerFrame::Remembered { .. } => Ok(()),
            other => bail!("unexpected reply to remember: {other:?}"),
        }
    }

    /// Full-text recall from the memory store.
    pub async fn recall(&mut self, query: &str, room: Option<String>, limit: Option<u32>) -> Result<Vec<RecallHit>> {
        match self
            .transport
            .request(ClientFrame::Recall { query: query.to_string(), room, limit })
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
}
