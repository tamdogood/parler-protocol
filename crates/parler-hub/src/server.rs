//! The hub's WebSocket front door + per-connection request handling.
//!
//! Each connection is a small state machine: an unauthenticated socket may only send `Hello`; the
//! hub replies with a [`ServerFrame::Challenge`] nonce, the client signs it with its nkey seed, and
//! once verified every other op is authorized against room membership. Every op gets exactly one
//! reply frame — the transport is plain request/response (a recipient *pulls* rather than being
//! pushed to), which keeps the hub stateless per message and trivially durable.

use crate::{now_ms, Store};
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use parler_protocol::{
    normalize_mentions, token, ClientFrame, EndpointRef, RoomKind, ServerFrame, Target,
};
use rand::Rng;
use std::sync::Arc;

/// Shared server state: the durable store + the base URL advertised in invite links.
pub struct HubState {
    pub store: Store,
    pub public_url: String,
}

/// Build the axum router (health check, the human-facing join page, and the agent WebSocket).
pub fn app(state: Arc<HubState>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/join/:code", get(join_page))
        .route("/ws", get(ws_handler))
        .with_state(state)
}

/// Serve the hub on an already-bound listener (so tests can bind port 0).
pub async fn serve(listener: tokio::net::TcpListener, state: Arc<HubState>) -> anyhow::Result<()> {
    axum::serve(listener, app(state).into_make_service()).await?;
    Ok(())
}

async fn join_page(Path(code): Path<String>) -> impl IntoResponse {
    format!(
        "Parler invite code: {code}\n\nHand this to another agent and have it run:\n    parler join {code}\n"
    )
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<HubState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Per-connection authentication state.
#[derive(Default)]
struct ConnState {
    nonce: Option<String>,
    authed: Option<Authed>,
}

#[derive(Clone)]
struct Authed {
    id: String,
    name: String,
    role: Option<String>,
}

async fn handle_socket(mut socket: WebSocket, state: Arc<HubState>) {
    let mut conn = ConnState::default();
    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            WsMessage::Text(txt) => {
                let reply = match serde_json::from_str::<ClientFrame>(&txt) {
                    Ok(frame) => dispatch(&state, &mut conn, frame),
                    Err(e) => ServerFrame::Error {
                        message: format!("malformed frame: {e}"),
                    },
                };
                let out = serde_json::to_string(&reply).unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"reply serialize failed\"}".into()
                });
                if socket.send(WsMessage::Text(out)).await.is_err() {
                    break;
                }
            }
            WsMessage::Ping(p) => {
                let _ = socket.send(WsMessage::Pong(p)).await;
            }
            WsMessage::Close(_) => break,
            _ => {}
        }
    }
    // Best-effort: mark the agent offline when the socket drops.
    if let Some(a) = &conn.authed {
        let _ = state.store.touch_presence(&a.id, "offline", None, now_ms());
    }
}

/// Route one client frame to its reply. Synchronous (the store never blocks across an await).
fn dispatch(state: &HubState, conn: &mut ConnState, frame: ClientFrame) -> ServerFrame {
    if let ClientFrame::Hello { id, name, role, sig, .. } = frame {
        return handle_hello(state, conn, id, name, role, sig);
    }
    let Some(authed) = conn.authed.clone() else {
        return ServerFrame::Error {
            message: "not authenticated — send `hello` first".into(),
        };
    };
    handle_authed(&state.store, &state.public_url, &authed, frame)
        .unwrap_or_else(|e| ServerFrame::Error { message: e.to_string() })
}

fn handle_hello(
    state: &HubState,
    conn: &mut ConnState,
    id: String,
    name: String,
    role: Option<String>,
    sig: Option<String>,
) -> ServerFrame {
    match sig {
        // Step 1: issue a challenge to sign.
        None => {
            let nonce = uuid::Uuid::new_v4().to_string();
            conn.nonce = Some(nonce.clone());
            ServerFrame::Challenge { nonce }
        }
        // Step 2: verify the signature over the issued nonce.
        Some(sig) => {
            let Some(nonce) = conn.nonce.clone() else {
                return ServerFrame::Error {
                    message: "no challenge issued — send `hello` without a signature first".into(),
                };
            };
            if !verify_sig(&id, &nonce, &sig) {
                return ServerFrame::Error {
                    message: "signature verification failed".into(),
                };
            }
            let now = now_ms();
            if let Err(e) = state.store.upsert_agent(&id, &name, role.as_deref(), now) {
                return ServerFrame::Error { message: e.to_string() };
            }
            let _ = state.store.touch_presence(&id, "idle", None, now);
            conn.authed = Some(Authed { id: id.clone(), name: name.clone(), role });
            ServerFrame::Welcome { id, name }
        }
    }
}

fn handle_authed(
    store: &Store,
    public_url: &str,
    me: &Authed,
    frame: ClientFrame,
) -> anyhow::Result<ServerFrame> {
    match frame {
        ClientFrame::Hello { .. } => unreachable!("handled in dispatch"),

        ClientFrame::Invite { kind, room, ttl_secs, max_uses } => {
            let now = now_ms();
            let expires = now + (ttl_secs.unwrap_or(24 * 3600) as i64) * 1000;
            let (room_name, max) = match kind {
                RoomKind::Dm => (format!("dm.{}", gen_suffix()), 1),
                RoomKind::Channel => (
                    room.map(|r| token(&r)).unwrap_or_else(|| format!("room.{}", gen_suffix())),
                    max_uses.unwrap_or(50),
                ),
                RoomKind::Service => (
                    format!("svc.{}", room.map(|r| token(&r)).unwrap_or_else(gen_suffix)),
                    max_uses.unwrap_or(50),
                ),
            };
            store.ensure_room(&room_name, kind, None, now)?;
            store.add_member(&room_name, &me.id, now)?;
            let code = gen_code();
            store.create_invite(&code, &room_name, kind, None, max, expires, &me.id, now)?;
            let url = format!("{public_url}/join/{code}");
            Ok(ServerFrame::Invited { code, url, room: room_name, kind, expires_at: expires })
        }

        ClientFrame::Redeem { code } => {
            let code = normalize_code(&code);
            let (room, kind) = store.redeem_invite(&code, &me.id, now_ms())?;
            Ok(ServerFrame::Joined { room, kind })
        }

        ClientFrame::Serve { service } => {
            let room = format!("svc.{}", token(&service));
            let now = now_ms();
            store.ensure_room(&room, RoomKind::Service, None, now)?;
            store.add_member(&room, &me.id, now)?;
            Ok(ServerFrame::Joined { room, kind: RoomKind::Service })
        }

        ClientFrame::Send { target, parts, mentions, reply_to } => {
            let room = resolve_target(store, me, &target)?;
            let mentions = mentions.as_deref().and_then(normalize_mentions);
            let from = EndpointRef { id: me.id.clone(), name: me.name.clone(), role: me.role.clone() };
            let (id, seq) = store.append_message(
                &room,
                &from,
                &parts,
                mentions.as_deref(),
                reply_to.as_deref(),
                now_ms(),
            )?;
            Ok(ServerFrame::Sent { id, seq, room })
        }

        ClientFrame::Pull { room, since, limit } => {
            if !store.is_member(&room, &me.id)? {
                anyhow::bail!("not a member of '{room}'");
            }
            let (messages, cursor) = store.pull(&room, &me.id, since, limit)?;
            Ok(ServerFrame::Pulled { room, messages, cursor })
        }

        ClientFrame::Remember { fact } => {
            if let Some(room) = &fact.room {
                if !store.is_member(room, &me.id)? {
                    anyhow::bail!("not a member of '{room}'");
                }
            }
            store.remember(&me.id, &fact, now_ms())?;
            Ok(ServerFrame::Remembered { ok: true })
        }

        ClientFrame::Recall { query, room, limit } => {
            if let Some(room) = &room {
                if !store.is_member(room, &me.id)? {
                    anyhow::bail!("not a member of '{room}'");
                }
            }
            let hits = store.recall(&me.id, &query, room.as_deref(), limit)?;
            Ok(ServerFrame::Recalled { hits })
        }

        ClientFrame::Rooms => Ok(ServerFrame::Rooms { rooms: store.rooms_of(&me.id)? }),

        ClientFrame::Roster { room } => {
            if !store.is_member(&room, &me.id)? {
                anyhow::bail!("not a member of '{room}'");
            }
            Ok(ServerFrame::Roster { room: room.clone(), entries: store.roster(&room)? })
        }

        ClientFrame::Presence { status, activity } => {
            store.touch_presence(&me.id, &status, activity.as_deref(), now_ms())?;
            Ok(ServerFrame::PresenceOk)
        }

        ClientFrame::Ping => Ok(ServerFrame::Pong),
    }
}

/// Resolve a [`Target`] to the concrete room the hub stores under, enforcing authorization.
fn resolve_target(store: &Store, me: &Authed, target: &Target) -> anyhow::Result<String> {
    match target {
        Target::Room { room } => {
            if !store.is_member(room, &me.id)? {
                anyhow::bail!("not a member of '{room}'");
            }
            Ok(room.clone())
        }
        Target::Dm { agent } => store
            .find_dm_room(&me.id, agent)?
            .ok_or_else(|| anyhow::anyhow!("no DM channel with '{agent}' — pair first (invite/join)")),
        Target::Service { service } => {
            let room = format!("svc.{}", token(service));
            if store.room_kind(&room)?.is_none() {
                anyhow::bail!("no such service '{service}' — a worker must `serve` it first");
            }
            // A requester auto-joins so it can also receive replies on the service room.
            store.add_member(&room, &me.id, now_ms())?;
            Ok(room)
        }
    }
}

fn verify_sig(id: &str, nonce: &str, sig_b64: &str) -> bool {
    let Ok(kp) = nkeys::KeyPair::from_public_key(id) else {
        return false;
    };
    let Ok(sig) = data_encoding::BASE64.decode(sig_b64.as_bytes()) else {
        return false;
    };
    kp.verify(nonce.as_bytes(), &sig).is_ok()
}

/// Accept a bare code or a pasted link (`parler://host/join/CODE`, `http://host/join/CODE`).
fn normalize_code(s: &str) -> String {
    let s = s.trim();
    if let Some(idx) = s.rfind("/join/") {
        return s[idx + 6..].trim().trim_end_matches('/').to_string();
    }
    s.to_string()
}

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const SUFFIX_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

fn gen_code() -> String {
    let mut rng = rand::thread_rng();
    (0..8).map(|_| CODE_ALPHABET[rng.gen_range(0..CODE_ALPHABET.len())] as char).collect()
}

fn gen_suffix() -> String {
    let mut rng = rand::thread_rng();
    (0..6).map(|_| SUFFIX_ALPHABET[rng.gen_range(0..SUFFIX_ALPHABET.len())] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_code_extracts_from_links() {
        assert_eq!(normalize_code("AB12CD34"), "AB12CD34");
        assert_eq!(normalize_code("  AB12CD34 "), "AB12CD34");
        assert_eq!(normalize_code("parler://127.0.0.1:7070/join/AB12CD34"), "AB12CD34");
        assert_eq!(normalize_code("http://hub.example/join/AB12CD34/"), "AB12CD34");
    }

    #[test]
    fn generated_codes_have_expected_shape() {
        let c = gen_code();
        assert_eq!(c.len(), 8);
        assert!(c.bytes().all(|b| CODE_ALPHABET.contains(&b)));
    }
}
