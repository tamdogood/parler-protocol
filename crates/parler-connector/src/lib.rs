//! parler-connector — the **client core** every agent surface shares.
//!
//! This is the `MeshAgent`: a small, transport-agnostic API for talking to a Parler Protocol hub —
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

pub use agent::{verify_message, BundleMeta, Invite, JoinOutcome, MeshAgent, PushReceipt, SigStatus};
pub use client::HubClient;
pub use config::{home_dir, Config};

use anyhow::Result;
use async_trait::async_trait;
use parler_protocol::{ClientFrame, ServerFrame, StoredMessage};
use std::time::Duration;

/// Turn an unexpected reply frame into a user/LLM-facing error — never a raw `{:?}` Debug dump of
/// the frame. If the hub sent its own error, surface that message (the most specific part) prefixed
/// with the action that failed; otherwise name it a likely version mismatch and point at the
/// recovery command. The error style is documented in `CONTRIBUTING.md`; see issue #111.
pub(crate) fn unexpected_reply(action: &str, frame: &ServerFrame) -> anyhow::Error {
    match frame {
        ServerFrame::Error { message, .. } => anyhow::anyhow!("couldn't {action}: {message}"),
        _ => anyhow::anyhow!(
            "couldn't {action}: the hub sent an unexpected reply — the hub and client may be \
             running different versions. Run `parler doctor`."
        ),
    }
}

/// The stable [`parler_protocol::error_code`] classifier of a hub error, if it carried one.
///
/// Any error surfaced by [`MeshAgent`] that originated as a hub [`ServerFrame::Error`] with a `code`
/// arrives as a downcastable [`parler_protocol::CodedError`]; this reads the classifier back out so a
/// caller can branch on *why* an op failed (e.g. `Some("rate_limited")` ⇒ back off and retry) without
/// matching on the human message. Returns `None` for an uncoded error (an old hub, or a local error).
pub fn hub_error_code(err: &anyhow::Error) -> Option<&str> {
    err.downcast_ref::<parler_protocol::CodedError>()?.code.as_deref()
}

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

#[cfg(test)]
mod error_style_tests {
    use super::unexpected_reply;
    use parler_protocol::ServerFrame;

    #[test]
    fn surfaces_the_hub_message_with_operation_context() {
        // When the hub sent its own error, that message is the most specific part — keep it, but
        // prefix it with the action that failed. Never leak the raw frame Debug.
        let e = unexpected_reply(
            "send the message",
            &ServerFrame::error("room 'x' does not exist"),
        );
        let s = e.to_string();
        assert!(s.contains("send the message"), "missing operation context: {s}");
        assert!(s.contains("room 'x' does not exist"), "dropped the hub message: {s}");
        assert!(!s.contains("Error {"), "Debug-dumped the frame: {s}");
    }

    #[test]
    fn names_a_remedy_for_an_unexpected_frame() {
        // A non-Error unexpected reply is a likely version mismatch → point at the recovery command.
        let e = unexpected_reply("pull messages", &ServerFrame::JoinPending { room: "r".into() });
        let s = e.to_string();
        assert!(s.contains("pull messages"), "missing operation context: {s}");
        assert!(s.contains("parler doctor"), "missing the remedy: {s}");
        assert!(!s.contains("JoinPending"), "Debug-dumped the frame: {s}");
    }

    /// Regression guard for issue #111: no user/LLM-facing error constructor in the transport may
    /// Debug-format a value (`{:?}`) — that dumps raw Rust internals to a model or a person. Route
    /// unexpected reply frames through [`unexpected_reply`] instead. (Test-only `{:?}` in `assert!`
    /// messages is fine — this scans only `bail!`/`anyhow!` lines.)
    #[test]
    fn no_error_message_debug_dumps_a_value() {
        for (file, src) in
            [("agent.rs", include_str!("agent.rs")), ("client.rs", include_str!("client.rs"))]
        {
            for (i, line) in src.lines().enumerate() {
                if line.contains("bail!(") || line.contains("anyhow!(") {
                    assert!(
                        !line.contains(":?}"),
                        "{file}:{}: error message Debug-dumps a value with {{:?}} — give it a named \
                         message + remedy (issue #111): {}",
                        i + 1,
                        line.trim()
                    );
                }
            }
        }
    }
}
