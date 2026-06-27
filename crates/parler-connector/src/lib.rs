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

pub use agent::{Invite, MeshAgent};
pub use client::HubClient;
pub use config::{home_dir, Config};

use anyhow::Result;
use async_trait::async_trait;
use parler_protocol::{ClientFrame, ServerFrame};

/// The seam between [`MeshAgent`] and a concrete transport: send one request frame, get one reply.
/// A [`ServerFrame::Error`] reply is surfaced as `Err`.
#[async_trait]
pub trait MeshTransport: Send {
    async fn request(&mut self, frame: ClientFrame) -> Result<ServerFrame>;
}
