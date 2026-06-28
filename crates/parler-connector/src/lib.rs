//! parler-connector — the **client core** every agent surface shares.
//!
//! This is the `MeshAgent`: a small, transport-agnostic API for talking to a Parler hub —
//! invite/join (pairing), the three send patterns, pull (the durable inbox), and the memory
//! store (remember/recall). The CLI (`parler …`), the MCP server (`parler mcp`), and the Hermes
//! plugin are all thin adapters over this one type.
//!
//! The transport sits behind the [`MeshTransport`] seam. Today the only implementation is
//! [`HubClient`] (WebSocket → `parler-hub`); a `NatsTransport` reusing the full-rewrite NATS/JWT
//! stack can slot in later without touching [`MeshAgent`].

pub mod agent;
pub mod client;
pub mod config;

pub use agent::{BundleMeta, Invite, JoinOutcome, MeshAgent, PushReceipt};
pub use client::HubClient;
pub use config::{home_dir, Config};

use anyhow::Result;
use async_trait::async_trait;
use parler_protocol::{ClientFrame, ServerFrame, StoredMessage};
use std::time::Duration;

/// The seam between [`MeshAgent`] and a concrete transport: send one request frame, get one reply.
/// A [`ServerFrame::Error`] reply is surfaced as `Err`.
#[async_trait]
pub trait MeshTransport: Send {
    async fn request(&mut self, frame: ClientFrame) -> Result<ServerFrame>;

    /// Ask the hub to push new room messages to this connection (sub-second delivery). Returns
    /// `true` if the hub acknowledged (it supports push), `false` if it doesn't (e.g. an older hub),
    /// in which case the caller should keep polling with [`MeshTransport::request`] + `Pull`. The
    /// default transport doesn't support push.
    async fn subscribe(&mut self) -> Result<bool> {
        Ok(false)
    }

    /// Block up to `max_wait` for the next **pushed** message (a peer's — never your own), returning
    /// `None` on timeout. Only meaningful after a successful [`MeshTransport::subscribe`]; pushes are
    /// best-effort, so a returned message is also retrievable by `Pull` (which is what advances the
    /// durable cursor). The default transport never delivers a push.
    async fn next_delivery(&mut self, max_wait: Duration) -> Result<Option<StoredMessage>> {
        let _ = max_wait;
        Ok(None)
    }

    /// Upload a content-addressed blob: send the `put` ([`ClientFrame::PutBlob`]) frame, await
    /// [`ServerFrame::BlobReady`], stream `bytes` as one binary frame, and return the hub's
    /// [`ServerFrame::BlobStored`]. Transports without a binary side-channel leave this unimplemented.
    async fn upload_blob(&mut self, put: ClientFrame, bytes: &[u8]) -> Result<ServerFrame> {
        let _ = (put, bytes);
        anyhow::bail!("this transport does not support blob upload")
    }

    /// Download a content-addressed blob: send the `get` ([`ClientFrame::GetBlob`]) frame, await
    /// [`ServerFrame::BlobIncoming`], and return the binary payload.
    async fn download_blob(&mut self, get: ClientFrame) -> Result<Vec<u8>> {
        let _ = get;
        anyhow::bail!("this transport does not support blob download")
    }
}
