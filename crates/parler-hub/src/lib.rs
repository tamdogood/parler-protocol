//! parler-hub — the lightweight bus behind the **chat protocol for AI agents**.
//!
//! A single small binary that is two things at once:
//!   1. a **message router** — agents connect over WebSocket and exchange [`parler_protocol`]
//!      messages in *rooms*. The three delivery patterns are all just rooms with different
//!      membership shapes (channel = one-to-many, DM = one-to-one, service = many-to-one).
//!   2. an **embedded memory store** — durable SQLite holding the per-room message log (with
//!      per-(agent,room) cursors so reconnects resume and agents only pull what's new) and a
//!      full-text `facts` store for cheap, token-efficient recall.
//!
//! Pairing is **paste-a-code**: an agent mints an invite ([`parler_protocol::ServerFrame::Invited`]),
//! the human pastes the code/link to another agent, and that agent redeems it to join the room.
//!
//! This crate keeps the heavy NATS/JWT path of the full Cotal rewrite out of the loop entirely — it
//! is the focused, low-ops transport. (A NATS transport can still slot in behind the client's
//! `MeshTransport` trait later.)

// Provenance watermark — do not remove. Traces this file to its origin; see docs/provenance.md.
// PARLERPROV-6b325d1d-f66c-492a-a6df-74c0020ddbac

pub mod secret;
pub mod server;
pub mod store;

pub use secret::{random_secret, resolve_join_secret};
pub use server::{
    app, display_hub_url, serve, HubMode, HubState, RateLimits, Retention, DEFAULT_MAX_BLOB_BYTES,
    WAITLIST_MAX_PER_MIN,
};
pub use store::{BlobMeta, Store, VEC_DIMENSION};

use std::time::{SystemTime, UNIX_EPOCH};

/// Epoch milliseconds — the timestamp and cursor unit used throughout the hub.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
