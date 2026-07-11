//! [`HubClient`] — the WebSocket transport to `parler-hub`.
//!
//! On connect it performs the nkey challenge-response handshake (hello → challenge → signed hello →
//! welcome), proving it owns the identity's keypair. Thereafter it is a request/reply channel (one
//! reply per request), exposed via [`MeshTransport`].
//!
//! After [`HubClient::subscribe`] the hub may also send unsolicited [`ServerFrame::Delivery`] frames
//! at any time (sub-second push). Those are **demultiplexed** from request replies: any delivery that
//! arrives while we're reading a reply is set aside in `inbox`, and [`HubClient::next_delivery`]
//! drains it (then awaits the socket for more). Push is best-effort — the durable cursor (`Pull`)
//! remains the source of truth — so a missed delivery is never a lost message.

use crate::MeshTransport;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use parler_auth::Identity;
use parler_protocol::{ClientFrame, CodedError, ServerFrame, StoredMessage};
use std::collections::VecDeque;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async, tungstenite::Message as WsMessage, MaybeTlsStream, WebSocketStream,
};

/// Cap on the client-side push buffer. Pushes arriving while the caller isn't draining (e.g. an MCP
/// agent idle between tool calls) accumulate here; past this we drop the oldest. Harmless by design —
/// a dropped push is still returned by the next `Pull` — so this just bounds memory.
const INBOX_CAP: usize = 1024;

/// Marker error: the underlying WebSocket was lost — the hub closed it (e.g. an idle timeout) or an
/// IO error killed it — as opposed to a hub *application* error (which is a valid reply frame). The
/// agent layer treats this specifically as "reconnect and retry once", since room membership and
/// read cursors are durable server-side, so a fresh connection resumes exactly where we left off.
#[derive(Debug)]
pub(crate) struct Disconnected;

impl std::fmt::Display for Disconnected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("connection to the hub was lost")
    }
}

impl std::error::Error for Disconnected {}

/// Build the [`Disconnected`] marker as an `anyhow::Error` (top-level, so `downcast_ref` finds it).
fn disconnected() -> anyhow::Error {
    anyhow::Error::new(Disconnected)
}

/// Warn (to stderr) when the hub advertises a different *major* protocol version than this client —
/// a heads-up that some frames may be misunderstood and the user should update. Purely advisory: the
/// wire format is additive within a major version, and an older hub that omits the field is treated
/// as compatible, so we never block the connection on this.
fn warn_on_protocol_mismatch(hub_version: Option<&str>) {
    let Some(hub) = hub_version else { return };
    let ours = parler_protocol::PROTOCOL_VERSION;
    let major = |v: &str| v.split('.').next().unwrap_or_default().to_string();
    if major(hub) != major(ours) {
        eprintln!(
            "parler: warning — hub speaks protocol {hub} but this client is {ours}; \
             some features may not work. Update parler."
        );
    }
}

/// An authenticated WebSocket connection to a hub.
pub struct HubClient {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    /// Pushed messages that arrived interleaved with a request's reply, buffered until the caller
    /// drains them via [`HubClient::next_delivery`]. Bounded by [`INBOX_CAP`] (drops oldest).
    inbox: VecDeque<StoredMessage>,
    /// Whether [`HubClient::subscribe`] succeeded — i.e. the hub is pushing to us. When `false`,
    /// `next_delivery` returns immediately (nothing will ever arrive unsolicited).
    subscribed: bool,
}

impl HubClient {
    /// Connect to `hub_url` and authenticate as `identity` (display `name`/`role`).
    pub async fn connect(
        hub_url: &str,
        identity: &Identity,
        name: &str,
        role: Option<&str>,
    ) -> Result<HubClient> {
        ensure_crypto_provider();
        let ws_url = to_ws_url(hub_url);
        let (ws, _resp) = connect_async(&ws_url)
            .await
            .map_err(|e| anyhow!("connecting to {ws_url}: {e}"))?;
        let mut client = HubClient { ws, inbox: VecDeque::new(), subscribed: false };
        client.handshake(identity, name, role).await?;
        Ok(client)
    }

    async fn handshake(&mut self, identity: &Identity, name: &str, role: Option<&str>) -> Result<()> {
        // Step 1: hello without a signature → the hub issues a challenge nonce.
        self.send(&ClientFrame::Hello {
            id: identity.id.clone(),
            name: name.to_string(),
            role: role.map(String::from),
            nonce: None,
            sig: None,
            secret: None,
        })
        .await?;
        let (nonce, hub_version) = match self.recv().await? {
            ServerFrame::Challenge { nonce, version } => (nonce, version),
            ServerFrame::Error { message, .. } => bail!("hub rejected hello: {message}"),
            other => return Err(crate::unexpected_reply("complete the handshake", &other)),
        };
        warn_on_protocol_mismatch(hub_version.as_deref());

        // Step 2: sign the nonce with the nkey seed and re-send hello.
        let kp = nkeys::KeyPair::from_seed(&identity.seed).map_err(|e| anyhow!("bad seed: {e}"))?;
        let sig = kp.sign(nonce.as_bytes()).map_err(|e| anyhow!("signing challenge: {e}"))?;
        let sig_b64 = data_encoding::BASE64.encode(&sig);
        // A private hub may require a shared join secret in addition to key ownership. Presented over
        // the (TLS-terminated) connection, like a bearer token; absent/empty ⇒ omitted.
        let secret = std::env::var("PARLER_JOIN_SECRET").ok().filter(|s| !s.is_empty());
        self.send(&ClientFrame::Hello {
            id: identity.id.clone(),
            name: name.to_string(),
            role: role.map(String::from),
            nonce: Some(nonce),
            sig: Some(sig_b64),
            secret,
        })
        .await?;
        match self.recv().await? {
            ServerFrame::Welcome { .. } => Ok(()),
            ServerFrame::Error { message, .. } => bail!("authentication failed: {message}"),
            other => Err(crate::unexpected_reply("complete the handshake", &other)),
        }
    }

    async fn send(&mut self, frame: &ClientFrame) -> Result<()> {
        // A failed write means the socket is gone (broken pipe / already-closed) — surface it as a
        // reconnectable disconnect, not an opaque IO error.
        self.ws
            .send(WsMessage::Text(serde_json::to_string(frame)?))
            .await
            .map_err(|_| disconnected())?;
        Ok(())
    }

    /// Buffer an out-of-band pushed message, dropping the oldest if the buffer is full.
    fn buffer_push(&mut self, message: StoredMessage) {
        if self.inbox.len() >= INBOX_CAP {
            self.inbox.pop_front();
        }
        self.inbox.push_back(message);
    }

    /// Read the next reply frame, setting aside any unsolicited [`ServerFrame::Delivery`] pushes that
    /// interleave with it (so request/reply stays correct even while subscribed).
    async fn recv(&mut self) -> Result<ServerFrame> {
        while let Some(msg) = self.ws.next().await {
            match msg.map_err(|_| disconnected())? {
                WsMessage::Text(t) => match serde_json::from_str::<ServerFrame>(&t)? {
                    ServerFrame::Delivery { message } => self.buffer_push(message),
                    frame => return Ok(frame),
                },
                WsMessage::Close(_) => return Err(disconnected()),
                _ => continue,
            }
        }
        Err(disconnected())
    }

    /// Receive the next binary frame (a blob payload), surfacing an interleaved error frame as `Err`
    /// and buffering any interleaved push.
    async fn recv_binary(&mut self) -> Result<Vec<u8>> {
        while let Some(msg) = self.ws.next().await {
            match msg.map_err(|_| disconnected())? {
                WsMessage::Binary(b) => return Ok(b),
                WsMessage::Text(t) => match serde_json::from_str::<ServerFrame>(&t) {
                    Ok(ServerFrame::Delivery { message }) => self.buffer_push(message),
                    Ok(ServerFrame::Error { message, code }) => {
                        return Err(CodedError::from_wire(code, message).into())
                    }
                    _ => bail!("expected a binary blob, got a text frame"),
                },
                WsMessage::Close(_) => return Err(disconnected()),
                _ => continue,
            }
        }
        Err(disconnected())
    }

    /// Block on the socket until the next pushed message arrives (ignoring pings/pongs). `Ok(None)`
    /// if the hub closed the connection; an interleaved [`ServerFrame::Error`] (e.g. an idle-timeout
    /// notice) surfaces as `Err`. Callers wrap this in a timeout (see [`HubClient::next_delivery`]).
    async fn recv_push(&mut self) -> Result<Option<StoredMessage>> {
        while let Some(msg) = self.ws.next().await {
            match msg.map_err(|_| disconnected())? {
                WsMessage::Text(t) => match serde_json::from_str::<ServerFrame>(&t)? {
                    ServerFrame::Delivery { message } => return Ok(Some(message)),
                    ServerFrame::Error { message, code } => {
                        return Err(CodedError::from_wire(code, message).into())
                    }
                    _ => continue,
                },
                WsMessage::Close(_) => return Ok(None),
                _ => continue,
            }
        }
        Ok(None)
    }
}

#[async_trait]
impl MeshTransport for HubClient {
    async fn request(&mut self, frame: ClientFrame) -> Result<ServerFrame> {
        self.send(&frame).await?;
        let reply = self.recv().await?;
        if let ServerFrame::Error { message, code } = reply {
            return Err(CodedError::from_wire(code, message).into());
        }
        Ok(reply)
    }

    async fn subscribe(&mut self) -> Result<bool> {
        self.send(&ClientFrame::Subscribe).await?;
        match self.recv().await? {
            ServerFrame::Subscribed => {
                self.subscribed = true;
                Ok(true)
            }
            // An older hub doesn't know the `subscribe` op and replies with a malformed-frame error;
            // that's not fatal — the connection stays usable, the caller just keeps polling.
            ServerFrame::Error { .. } => Ok(false),
            other => return Err(crate::unexpected_reply("subscribe for live updates", &other)),
        }
    }

    async fn next_delivery(&mut self, max_wait: Duration) -> Result<Option<StoredMessage>> {
        if let Some(m) = self.inbox.pop_front() {
            return Ok(Some(m));
        }
        if !self.subscribed {
            return Ok(None);
        }
        match tokio::time::timeout(max_wait, self.recv_push()).await {
            Ok(res) => res,
            Err(_) => Ok(None), // timed out — no push within the window
        }
    }

    async fn upload_blob(&mut self, put: ClientFrame, bytes: &[u8]) -> Result<ServerFrame> {
        self.send(&put).await?;
        match self.recv().await? {
            ServerFrame::BlobReady { .. } => {}
            other => return Err(crate::unexpected_reply("start the upload", &other)),
        }
        self.ws.send(WsMessage::Binary(bytes.to_vec())).await?;
        match self.recv().await? {
            stored @ ServerFrame::BlobStored { .. } => Ok(stored),
            other => return Err(crate::unexpected_reply("finish the upload", &other)),
        }
    }

    async fn download_blob(&mut self, get: ClientFrame) -> Result<Vec<u8>> {
        self.send(&get).await?;
        match self.recv().await? {
            ServerFrame::BlobIncoming { .. } => {}
            other => return Err(crate::unexpected_reply("start the download", &other)),
        }
        self.recv_binary().await
    }
}

/// Install a process-wide rustls crypto provider before the first `wss://` dial.
///
/// `rustls` 0.23 refuses to auto-select a provider when more than one is compiled in, and panics on
/// the first TLS handshake. Two land in our tree — `ring` (via `tokio-tungstenite`) and `aws-lc-rs`
/// (via `async-nats`) — so we pick one explicitly. Idempotent and cheap, so it's safe to call on every
/// connect (including plain `ws://`, where it's just a no-op cost).
fn ensure_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Err means a provider was already installed by someone else — that's fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Normalize any hub address (`ws://`, `http(s)://`, `parler://`, or bare `host:port`) into the
/// `ws(s)://host:port/ws` URL the client dials.
fn to_ws_url(hub_url: &str) -> String {
    let u = hub_url.trim();
    if let Some(rest) = u.strip_prefix("wss://") {
        format!("wss://{}", with_ws_path(rest))
    } else if let Some(rest) = u.strip_prefix("ws://") {
        format!("ws://{}", with_ws_path(rest))
    } else if let Some(rest) = u.strip_prefix("https://") {
        format!("wss://{}", with_ws_path(rest))
    } else if let Some(rest) = u.strip_prefix("http://") {
        format!("ws://{}", with_ws_path(rest))
    } else if let Some(rest) = u.strip_prefix("parler://") {
        format!("ws://{}", with_ws_path(rest))
    } else {
        format!("ws://{}", with_ws_path(u))
    }
}

fn with_ws_path(host_and_path: &str) -> String {
    let h = host_and_path.trim_end_matches('/');
    if h.ends_with("/ws") {
        h.to_string()
    } else {
        format!("{h}/ws")
    }
}

#[cfg(test)]
mod tests {
    use super::to_ws_url;

    #[test]
    fn ws_url_normalization() {
        assert_eq!(to_ws_url("127.0.0.1:7070"), "ws://127.0.0.1:7070/ws");
        assert_eq!(to_ws_url("parler://127.0.0.1:7070"), "ws://127.0.0.1:7070/ws");
        assert_eq!(to_ws_url("http://hub.example"), "ws://hub.example/ws");
        assert_eq!(to_ws_url("https://hub.example/"), "wss://hub.example/ws");
        assert_eq!(to_ws_url("ws://127.0.0.1:7070/ws"), "ws://127.0.0.1:7070/ws");
    }
}
