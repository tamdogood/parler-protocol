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
        self.send(&ClientFrame::Hello {
            id: identity.id.clone(),
            name: name.to_string(),
            role: role.map(String::from),
            nonce: Some(nonce),
            sig: Some(sig_b64),
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
