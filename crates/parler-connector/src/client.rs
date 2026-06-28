//! [`HubClient`] — the WebSocket transport to `parler-hub`.
//!
//! On connect it performs the nkey challenge-response handshake (hello → challenge → signed hello →
//! welcome), proving it owns the identity's keypair. Thereafter it is a simple request/reply
//! channel (one reply per request — the hub never pushes), which it exposes via [`MeshTransport`].

use crate::MeshTransport;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use parler_auth::Identity;
use parler_protocol::{ClientFrame, ServerFrame};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async, tungstenite::Message as WsMessage, MaybeTlsStream, WebSocketStream,
};

/// An authenticated WebSocket connection to a hub.
pub struct HubClient {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
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
        let mut client = HubClient { ws };
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
        let nonce = match self.recv().await? {
            ServerFrame::Challenge { nonce } => nonce,
            ServerFrame::Error { message } => bail!("hub rejected hello: {message}"),
            other => bail!("expected a challenge, got {other:?}"),
        };

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
            ServerFrame::Error { message } => bail!("authentication failed: {message}"),
            other => bail!("expected welcome, got {other:?}"),
        }
    }

    async fn send(&mut self, frame: &ClientFrame) -> Result<()> {
        self.ws
            .send(WsMessage::Text(serde_json::to_string(frame)?))
            .await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<ServerFrame> {
        while let Some(msg) = self.ws.next().await {
            match msg? {
                WsMessage::Text(t) => return Ok(serde_json::from_str(&t)?),
                WsMessage::Close(_) => bail!("hub closed the connection"),
                _ => continue,
            }
        }
        bail!("hub connection ended")
    }

    /// Receive the next binary frame (a blob payload), surfacing an interleaved error frame as `Err`.
    async fn recv_binary(&mut self) -> Result<Vec<u8>> {
        while let Some(msg) = self.ws.next().await {
            match msg? {
                WsMessage::Binary(b) => return Ok(b),
                WsMessage::Text(t) => {
                    if let Ok(ServerFrame::Error { message }) = serde_json::from_str::<ServerFrame>(&t) {
                        bail!("{message}");
                    }
                    bail!("expected a binary blob, got a text frame");
                }
                WsMessage::Close(_) => bail!("hub closed the connection"),
                _ => continue,
            }
        }
        bail!("hub connection ended")
    }
}

#[async_trait]
impl MeshTransport for HubClient {
    async fn request(&mut self, frame: ClientFrame) -> Result<ServerFrame> {
        self.send(&frame).await?;
        let reply = self.recv().await?;
        if let ServerFrame::Error { message } = &reply {
            bail!("{message}");
        }
        Ok(reply)
    }

    async fn upload_blob(&mut self, put: ClientFrame, bytes: &[u8]) -> Result<ServerFrame> {
        self.send(&put).await?;
        match self.recv().await? {
            ServerFrame::BlobReady { .. } => {}
            ServerFrame::Error { message } => bail!("{message}"),
            other => bail!("expected blob_ready, got {other:?}"),
        }
        self.ws.send(WsMessage::Binary(bytes.to_vec())).await?;
        match self.recv().await? {
            stored @ ServerFrame::BlobStored { .. } => Ok(stored),
            ServerFrame::Error { message } => bail!("{message}"),
            other => bail!("expected blob_stored, got {other:?}"),
        }
    }

    async fn download_blob(&mut self, get: ClientFrame) -> Result<Vec<u8>> {
        self.send(&get).await?;
        match self.recv().await? {
            ServerFrame::BlobIncoming { .. } => {}
            ServerFrame::Error { message } => bail!("{message}"),
            other => bail!("expected blob_incoming, got {other:?}"),
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
