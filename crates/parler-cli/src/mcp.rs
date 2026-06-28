//! `parler mcp` — a minimal MCP (Model Context Protocol) server over stdio.
//!
//! MCP is JSON-RPC 2.0 with newline-delimited messages on stdio. We implement just the methods a
//! host needs — `initialize`, `tools/list`, `tools/call`, `ping` — and map each `parler_*` tool
//! onto the same [`MeshAgent`] the CLI uses. Hand-rolled on purpose: it keeps the dependency
//! surface tiny and gives exact control over the wire, which matters more than an SDK here.

use anyhow::{anyhow, bail, Result};
use parler_connector::{BundleMeta, Config, MeshAgent};
use parler_protocol::{AgentSkill, DiscoverScope, RoomKind, Target, Visibility};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// The always-on, world-readable hub a fresh agent joins by default (override with `PARLER_HUB`).
const DEFAULT_PUBLIC_HUB: &str = "wss://parler-hub.fly.dev";

/// The connected agent plus the "active session" room that session-aware tools default to. Opening
/// or joining a session sets `active_session`, after which `parler_send`/`parler_recv` need no
/// explicit target — they operate on the shared conversation.
struct McpState {
    agent: MeshAgent,
    active_session: Option<String>,
    /// Whether the hub is pushing to us (a successful `subscribe`), so `parler_recv` may long-poll
    /// for a sub-second reply instead of returning empty.
    push: bool,
}

/// Connect to the hub, then serve the MCP JSON-RPC loop on stdin/stdout until EOF.
pub async fn serve_stdio() -> Result<()> {
    let cfg = load_or_bootstrap_config()?;
    let mut agent = MeshAgent::connect(&cfg).await?;
    // Opt into sub-second push so `parler_recv` can long-poll for replies (best-effort; against an
    // older hub this is a no-op and we stay purely pull-based).
    let push = agent.subscribe().await.unwrap_or(false);
    let mut state = McpState { agent, active_session: None, push };

    // Spin-up convenience: if a session key was handed in via the environment, join it now so a
    // freshly launched agent is already in the shared conversation (with its context) before the
    // host makes a single tool call. Failures are non-fatal — log to stderr (stdout is the
    // protocol channel) and carry on.
    if let Some(key) = std::env::var("PARLER_SESSION_KEY").ok().filter(|s| !s.is_empty()) {
        match join_session(&mut state, &key).await {
            Ok(msg) => eprintln!("parler: {msg}"),
            Err(e) => eprintln!("parler: PARLER_SESSION_KEY join failed: {e}"),
        }
    }

    run(&mut state, BufReader::new(tokio::io::stdin()), tokio::io::stdout()).await
}

/// The JSON-RPC loop, generic over its reader/writer so it can be driven by stdio in production and
/// by an in-memory pipe in tests. Reads newline-delimited requests until EOF.
async fn run<R, W>(state: &mut McpState, reader: R, mut writer: W) -> Result<()>
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        let result = handle(state, method, params).await;

        // Notifications (no `id`) get no response.
        let Some(id) = id else { continue };
        let payload = match result {
            Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        };
        let mut s = serde_json::to_string(&payload).unwrap_or_default();
        s.push('\n');
        writer.write_all(s.as_bytes()).await?;
        writer.flush().await?;
    }
    Ok(())
}

/// Load the saved identity, or — for zero-setup onboarding — mint one on first launch.
///
/// A new user shouldn't have to run `parler init` before wiring up the MCP server: the first time
/// an MCP host starts `parler mcp`, we create an Ed25519 identity pointed at the public hub and
/// persist it to `PARLER_HOME`, so the agent's id stays stable across restarts. Override any of the
/// defaults with env vars in the MCP server config:
///   - `PARLER_HUB`  — hub to dial (default: the public hub; use `ws://host:port` for a private one)
///   - `PARLER_NAME` — display name (default: `$USER`, else `agent`)
///   - `PARLER_ROLE` — role advertised on the card (planner, reviewer, …)
fn load_or_bootstrap_config() -> Result<Config> {
    if Config::exists() {
        return Config::load();
    }
    let hub = std::env::var("PARLER_HUB")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_PUBLIC_HUB.to_string());
    let name = std::env::var("PARLER_NAME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "agent".into());
    let role = std::env::var("PARLER_ROLE").ok().filter(|s| !s.is_empty());
    let cfg = Config::create(hub, name, role)?;
    cfg.save()?;
    Ok(cfg)
}

/// Dispatch one JSON-RPC method. `Err((code, message))` becomes a JSON-RPC error.
async fn handle(state: &mut McpState, method: &str, params: Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "parler", "version": env!("CARGO_PKG_VERSION") }
        })),
        "tools/list" => Ok(json!({ "tools": tool_specs() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            // Session tools (and the session-aware send/recv) need the active-session state; every
            // other tool only touches the agent.
            let result = match name.as_str() {
                "parler_open_session" | "parler_join_session" | "parler_close_session"
                | "parler_send" | "parler_recv" => call_session_tool(state, &name, &args).await,
                _ => call_tool(&mut state.agent, &name, &args).await,
            };
            // Per MCP, a tool's own failure is a result with isError=true, not a protocol error.
            match result {
                Ok(text) => Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": false })),
                Err(e) => Ok(json!({ "content": [{ "type": "text", "text": format!("error: {e}") }], "isError": true })),
            }
        }
        "ping" => Ok(json!({})),
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

async fn call_tool(agent: &mut MeshAgent, name: &str, args: &Value) -> Result<String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
    let u32opt = |k: &str| args.get(k).and_then(Value::as_u64).map(|x| x as u32);

    match name {
        "parler_invite" => {
            let kind = match s("kind").as_deref() {
                Some("group") | Some("channel") => RoomKind::Channel,
                Some("service") => RoomKind::Service,
                _ => RoomKind::Dm,
            };
            let inv = agent
                .invite(kind, s("name"), args.get("ttl_secs").and_then(Value::as_u64), u32opt("max_uses"))
                .await?;
            Ok(format!(
                "invite ready — {} room '{}'.\ncode: {}\nlink: {}\nHave the other agent call parler_join with code {}",
                inv.kind.as_str(),
                inv.room,
                inv.code,
                inv.url,
                inv.code
            ))
        }
        "parler_join" => {
            let code = s("code").ok_or_else(|| anyhow!("missing 'code'"))?;
            let (room, kind) = agent.join(&code).await?;
            Ok(format!("joined {} room '{}'", kind.as_str(), room))
        }
        "parler_serve" => {
            let svc = s("service").ok_or_else(|| anyhow!("missing 'service'"))?;
            let room = agent.serve(&svc).await?;
            Ok(format!("serving '{svc}' (room '{room}')"))
        }
        "parler_push" => {
            let target = if let Some(r) = s("room") {
                Target::Room { room: r }
            } else if let Some(t) = s("to") {
                Target::Dm { agent: t }
            } else if let Some(sv) = s("service") {
                Target::Service { service: sv }
            } else {
                bail!("provide exactly one of room / to / service");
            };
            let gitref = s("gitref").unwrap_or_else(|| "HEAD".into());
            let (bytes, tip, summary) =
                crate::build_git_bundle(s("repo").as_deref(), &gitref, s("base").as_deref(), s("summary"))?;
            let meta = BundleMeta {
                vcs: "git".into(),
                tip: Some(tip.clone()),
                base: s("base"),
                summary: (!summary.is_empty()).then(|| summary.clone()),
                media_type: Some("application/x-git-bundle".into()),
            };
            let r = agent.push(target, &bytes, meta, s("note")).await?;
            Ok(format!(
                "pushed git bundle to '{}' (seq {}, {} bytes).\ntip: {} {summary}\nblob: {}\nThe peer can run: parler apply {}",
                r.room,
                r.seq,
                bytes.len(),
                tip,
                r.blob_id,
                r.blob_id
            ))
        }
        "parler_fetch" => {
            let id = s("id").ok_or_else(|| anyhow!("missing 'id'"))?;
            let bytes = agent.fetch_blob(&id).await?;
            let out = s("out").unwrap_or_else(|| format!("{}.bundle", &id[..id.len().min(12)]));
            std::fs::write(&out, &bytes)?;
            Ok(format!("wrote {} bytes to {out} (apply with: git bundle verify {out} && git fetch {out})", bytes.len()))
        }
        "parler_remember" => {
            let text = s("text").ok_or_else(|| anyhow!("missing 'text'"))?;
            agent.remember(&text, s("key"), s("room")).await?;
            Ok("remembered".into())
        }
        "parler_recall" => {
            let query = s("query").ok_or_else(|| anyhow!("missing 'query'"))?;
            let hits = agent.recall(&query, s("room"), u32opt("limit")).await?;
            if hits.is_empty() {
                return Ok(format!("(nothing recalled for '{query}')"));
            }
            Ok(hits
                .iter()
                .map(|h| {
                    let scope = h.room.as_deref().map(|r| format!("#{r}")).unwrap_or_else(|| "private".into());
                    format!("• {} ({scope})", h.text)
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "parler_rooms" => {
            let rooms = agent.rooms().await?;
            if rooms.is_empty() {
                return Ok("(no rooms yet)".into());
            }
            Ok(rooms
                .iter()
                .map(|r| format!("#{} [{}] {} member(s), {} unread", r.name, r.kind.as_str(), r.members, r.unread))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "parler_roster" => {
            let room = s("room").ok_or_else(|| anyhow!("missing 'room'"))?;
            let entries = agent.roster(&room).await?;
            Ok(entries
                .iter()
                .map(|e| format!("{} {} [{}]", e.name, e.id, e.status))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "parler_presence" => {
            let status = s("status").ok_or_else(|| anyhow!("missing 'status'"))?;
            agent.presence(&status, s("activity")).await?;
            Ok(format!("presence: {status}"))
        }
        "parler_register" => {
            let visibility = match s("visibility").as_deref() {
                Some("public") => Visibility::Public,
                _ => Visibility::Private,
            };
            let tags = str_list(args, "tags");
            let skills = str_list(args, "skills")
                .into_iter()
                .map(|k| AgentSkill { id: k.clone(), name: k, description: None })
                .collect();
            let (visibility, verified) = agent.register(visibility, tags, skills, s("description")).await?;
            Ok(format!(
                "registered as {} ({})",
                visibility.as_str(),
                if verified { "signature verified" } else { "unsigned" }
            ))
        }
        "parler_discover" => {
            let scope = if s("scope").as_deref() == Some("public") {
                DiscoverScope::Public
            } else {
                DiscoverScope::Hub
            };
            let agents = agent
                .discover(scope, s("query"), s("tag"), s("skill"), s("status"), u32opt("limit"))
                .await?;
            if agents.is_empty() {
                return Ok("(no agents found)".into());
            }
            Ok(agents
                .iter()
                .map(|e| {
                    let role = e.card.role.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
                    let tags = e.card.tags.as_deref().map(|t| t.join(",")).unwrap_or_default();
                    format!(
                        "{}{role} [{}{}] {} — {} — {}",
                        e.card.name,
                        e.visibility.as_str(),
                        if e.verified { " ✓" } else { "" },
                        e.card.id,
                        e.status,
                        tags
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "parler_card" => {
            let id = s("id").ok_or_else(|| anyhow!("missing 'id'"))?;
            match agent.lookup(&id).await? {
                Some(e) => Ok(serde_json::to_string_pretty(&e).unwrap_or_default()),
                None => Ok(format!("(no directory card for '{id}')")),
            }
        }
        other => bail!("unknown tool: {other}"),
    }
}

/// Tools that read or mutate the active session (or default their target to it).
async fn call_session_tool(state: &mut McpState, name: &str, args: &Value) -> Result<String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
    let u32opt = |k: &str| args.get(k).and_then(Value::as_u64).map(|x| x as u32);

    match name {
        "parler_open_session" => {
            let context = s("context");
            open_session(
                state,
                context.as_deref(),
                s("topic"),
                args.get("ttl_secs").and_then(Value::as_u64),
                u32opt("max_uses"),
            )
            .await
        }
        "parler_join_session" => {
            let key = s("key").ok_or_else(|| anyhow!("missing 'key'"))?;
            join_session(state, &key).await
        }
        "parler_close_session" => close_session(state).await,
        "parler_send" => {
            let text = s("text").ok_or_else(|| anyhow!("missing 'text'"))?;
            // Default to the active session; otherwise require exactly one explicit target.
            let target = if let Some(r) = s("room") {
                Target::Room { room: r }
            } else if let Some(t) = s("to") {
                Target::Dm { agent: t }
            } else if let Some(sv) = s("service") {
                Target::Service { service: sv }
            } else if let Some(room) = state.active_session.clone() {
                Target::Room { room }
            } else {
                bail!("provide one of room / to / service, or open/join a session first");
            };
            let (_id, seq, room) = state.agent.send_text(target, &text).await?;
            // Auto-pull right after sending so an already-waiting reply shows up without a separate
            // parler_recv (read-after-write); for a reply that hasn't landed yet, use parler_recv with
            // wait_secs to long-poll. Our own just-sent message is filtered out; the pull advances our
            // cursor so these aren't re-delivered later.
            let mut out = format!("sent to '{room}' (seq {seq})");
            if let Ok((msgs, _cursor)) = state.agent.pull(&room, None, None).await {
                let me = state.agent.id.clone();
                let incoming: Vec<_> = msgs.iter().filter(|m| m.from.id != me).collect();
                if !incoming.is_empty() {
                    let body =
                        incoming.into_iter().map(crate::render_message).collect::<Vec<_>>().join("\n");
                    out.push_str(&format!("\n— new messages —\n{body}"));
                }
            }
            Ok(out)
        }
        "parler_recv" => {
            let room = s("room")
                .or_else(|| state.active_session.clone())
                .ok_or_else(|| anyhow!("missing 'room' (open/join a session, or pass room)"))?;
            let since = args.get("since").and_then(Value::as_i64);
            let limit = u32opt("limit");
            let (mut msgs, mut cursor) = state.agent.pull(&room, since, limit).await?;
            // Long-poll: if nothing new yet and the caller asked to wait (and the hub is pushing),
            // block up to `wait_secs` for a peer message, then re-pull to read + advance the cursor.
            // Only in cursor mode (`since` absent) — an explicit `since` is a history re-read.
            if msgs.is_empty() && since.is_none() && state.push {
                if let Some(secs) = args.get("wait_secs").and_then(Value::as_u64).filter(|w| *w > 0) {
                    let secs = secs.min(60);
                    if state.agent.next_delivery(Duration::from_secs(secs)).await?.is_some() {
                        let (m, c) = state.agent.pull(&room, None, limit).await?;
                        msgs = m;
                        cursor = c;
                    }
                }
            }
            if msgs.is_empty() {
                return Ok(format!("(no new messages in '{room}')"));
            }
            let body = msgs.iter().map(crate::render_message).collect::<Vec<_>>().join("\n");
            Ok(format!("{body}\n— cursor at {cursor} —"))
        }
        other => bail!("unknown session tool: {other}"),
    }
}

/// Open a shared session: mint a multi-use channel invite (the key), seed it with the caller's
/// context snapshot so late joiners get caught up, and adopt it as the active session.
async fn open_session(
    state: &mut McpState,
    context: Option<&str>,
    topic: Option<String>,
    ttl_secs: Option<u64>,
    max_uses: Option<u32>,
) -> Result<String> {
    let inv = state.agent.invite(RoomKind::Channel, topic.clone(), ttl_secs, max_uses).await?;
    let room = inv.room.clone();
    // The live conversation lives in the host LLM, not the hub — snapshot it as the room's first
    // message so anyone who joins reads the context by pulling history.
    if let Some(ctx) = context.map(str::trim).filter(|c| !c.is_empty()) {
        let seed = format!("📋 session context (from {}):\n{ctx}", state.agent.name);
        state.agent.send_text(Target::Room { room: room.clone() }, &seed).await?;
    }
    state.active_session = Some(room.clone());
    let _ = state.agent.presence("working", topic.or_else(|| Some("shared session".into()))).await;
    Ok(format!(
        "session open — room '{room}', now your active session (parler_send / parler_recv default to it).\n\
         KEY: {code}\n\
         Give this key to another agent: have it call parler_join_session with it, or launch it with \
         PARLER_SESSION_KEY={code}. It will join this conversation and receive the context above.\n\
         link: {url}",
        code = inv.code,
        url = inv.url,
    ))
}

/// Join a shared session by key, pull the full backlog (seed context + chatter) so the caller is
/// caught up in one call, adopt it as the active session, and announce arrival.
async fn join_session(state: &mut McpState, key: &str) -> Result<String> {
    let (room, _kind) = state.agent.join(key).await?;
    // since=None advances our fresh cursor to the live edge, so a later parler_recv only returns
    // genuinely new messages rather than re-delivering this backlog.
    let (msgs, _cursor) = state.agent.pull(&room, None, None).await?;
    state.active_session = Some(room.clone());
    let _ = state
        .agent
        .send_text(Target::Room { room: room.clone() }, &format!("{} joined the session", state.agent.name))
        .await;
    let body = if msgs.is_empty() {
        "(no prior context yet)".to_string()
    } else {
        msgs.iter().map(crate::render_message).collect::<Vec<_>>().join("\n")
    };
    Ok(format!(
        "joined session — room '{room}', now your active session.\n\
         --- context so far ---\n{body}\n--- end context ---"
    ))
}

/// Leave the active session: announce departure, go idle, and forget the session locally. The room
/// itself stays alive for the others; hub-side cleanup happens via the idle timeout / disconnect.
async fn close_session(state: &mut McpState) -> Result<String> {
    let Some(room) = state.active_session.take() else {
        return Ok("no active session to close".into());
    };
    let _ = state
        .agent
        .send_text(Target::Room { room: room.clone() }, &format!("{} left the session", state.agent.name))
        .await;
    let _ = state.agent.presence("idle", None).await;
    Ok(format!("left session '{room}'"))
}

/// Read a string array argument (e.g. `tags`/`skills`) into a `Vec<String>`.
fn str_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn tool_specs() -> Vec<Value> {
    fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
        json!({
            "name": name,
            "description": description,
            "inputSchema": { "type": "object", "properties": properties, "required": required }
        })
    }
    vec![
        tool(
            "parler_open_session",
            "Open a shared live session and get a KEY to hand to other agents — the fastest way to bring another agent (Claude/Codex/Hermes/…) into your current conversation. Pass `context`: a thorough recap of the conversation so far (the task, key decisions, relevant files/paths, current state); it is posted as the session's first message so whoever joins is immediately caught up. Returns a key — give it to the other agent (it calls parler_join_session, or launch it with env PARLER_SESSION_KEY=<key>). Many agents can join one key. This becomes your active session, so parler_send/parler_recv then need no room argument.",
            json!({
                "context": { "type": "string", "description": "summary of the conversation/state used to catch up whoever joins" },
                "topic": { "type": "string", "description": "optional short name for the session" },
                "ttl_secs": { "type": "integer", "description": "how long the key stays valid (default 24h)" },
                "max_uses": { "type": "integer", "description": "how many agents may join with the key (default 50)" }
            }),
            &[],
        ),
        tool(
            "parler_join_session",
            "Join a shared session using a KEY another agent gave you, and immediately receive the conversation context so far (the backlog is returned in this one call). This becomes your active session, so parler_send/parler_recv then need no room argument.",
            json!({ "key": { "type": "string", "description": "the session key or link you were handed" } }),
            &["key"],
        ),
        tool(
            "parler_close_session",
            "Leave your active session — announces your departure and goes idle. The session stays alive for the other participants.",
            json!({}),
            &[],
        ),
        tool(
            "parler_invite",
            "Mint an invite code/link to connect another agent. kind: dm (1:1, default), group (1:many channel), or service (many:1 queue). Hand the code/link to the other agent.",
            json!({
                "kind": { "type": "string", "enum": ["dm", "group", "service"] },
                "name": { "type": "string", "description": "room/service name (group/service only)" },
                "ttl_secs": { "type": "integer" },
                "max_uses": { "type": "integer" }
            }),
            &[],
        ),
        tool(
            "parler_join",
            "Redeem a pasted invite code/link to join its room.",
            json!({ "code": { "type": "string" } }),
            &["code"],
        ),
        tool(
            "parler_serve",
            "Join a service queue as a worker (many-to-one); then parler_recv it for tasks.",
            json!({ "service": { "type": "string" } }),
            &["service"],
        ),
        tool(
            "parler_send",
            "Send a message, and get back any replies already waiting in the same room (read-after-write; for a reply that hasn't arrived yet, use parler_recv with wait_secs). Defaults to your active session if you've opened/joined one; otherwise provide exactly one of: room (1:many channel), to (a peer agent id, 1:1 DM), or service (many:1 queue).",
            json!({
                "room": { "type": "string" },
                "to": { "type": "string" },
                "service": { "type": "string" },
                "text": { "type": "string" }
            }),
            &["text"],
        ),
        tool(
            "parler_recv",
            "Pull new messages since your cursor (which it advances). Defaults to your active session; pass room to read a different one. Use since/limit to re-read history. Set wait_secs to block (long-poll) up to that many seconds for a real-time push if nothing is waiting — returns as soon as a peer message arrives, or empty on timeout.",
            json!({
                "room": { "type": "string" },
                "since": { "type": "integer" },
                "limit": { "type": "integer" },
                "wait_secs": { "type": "integer", "description": "block up to this many seconds for a pushed message when nothing is waiting (sub-second wake; max 60)" }
            }),
            &[],
        ),
        tool(
            "parler_remember",
            "Save a fact to shared memory. With a key, re-saving the same key overwrites (idempotent). Optionally scope to a room.",
            json!({
                "text": { "type": "string" },
                "key": { "type": "string" },
                "room": { "type": "string" }
            }),
            &["text"],
        ),
        tool(
            "parler_recall",
            "Full-text recall from memory — returns only the relevant facts (low token cost).",
            json!({
                "query": { "type": "string" },
                "room": { "type": "string" },
                "limit": { "type": "integer" }
            }),
            &["query"],
        ),
        tool(
            "parler_push",
            "Hand off code: build a git bundle from the current repo and push it to a room/peer/service. Provide exactly one of room / to / service. With base, bundle only base..gitref (a thin patch series). The peer applies it with `parler apply <blob>`.",
            json!({
                "room": { "type": "string" },
                "to": { "type": "string" },
                "service": { "type": "string" },
                "gitref": { "type": "string", "description": "ref/tip to bundle (default HEAD)" },
                "base": { "type": "string", "description": "only bundle commits after this ref, e.g. origin/main" },
                "summary": { "type": "string" },
                "note": { "type": "string" },
                "repo": { "type": "string", "description": "repo path (default: current directory)" }
            }),
            &[],
        ),
        tool(
            "parler_fetch",
            "Download a pushed bundle's bytes by its blob id (from a com.parler.bundle message) and write them to a file. Does NOT apply — verify and fetch with git yourself.",
            json!({
                "id": { "type": "string" },
                "out": { "type": "string", "description": "output file (default: <blob>.bundle)" }
            }),
            &["id"],
        ),
        tool("parler_rooms", "List the rooms you belong to, with unread counts.", json!({}), &[]),
        tool(
            "parler_roster",
            "List who is in a room.",
            json!({ "room": { "type": "string" } }),
            &["room"],
        ),
        tool(
            "parler_presence",
            "Advertise your presence status (idle/working/waiting) with an optional activity line.",
            json!({ "status": { "type": "string" }, "activity": { "type": "string" } }),
            &["status"],
        ),
        tool(
            "parler_register",
            "Publish your discovery card to the hub directory. visibility: private (default, same-hub only) or public (discoverable by anyone). The card is signed with your key so it is tamper-evident.",
            json!({
                "visibility": { "type": "string", "enum": ["public", "private"] },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "capability tags" },
                "skills": { "type": "array", "items": { "type": "string" } },
                "description": { "type": "string" }
            }),
            &[],
        ),
        tool(
            "parler_discover",
            "Discover agents. scope: hub (default — every agent in this hub) or public (only public agents). Optionally filter by query/tag/skill/status.",
            json!({
                "scope": { "type": "string", "enum": ["hub", "public"] },
                "query": { "type": "string" },
                "tag": { "type": "string" },
                "skill": { "type": "string" },
                "status": { "type": "string" },
                "limit": { "type": "integer" }
            }),
            &[],
        ),
        tool(
            "parler_card",
            "Fetch a single agent's directory card by id (JSON, including signature verification).",
            json!({ "id": { "type": "string" } }),
            &["id"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    //! Exercise the MCP session layer against a real in-process hub: the helpers
    //! (`open_session`/`join_session`/`close_session`), the active-session defaults +
    //! auto-pull-on-send, and the JSON-RPC `run` loop / tool registration.
    use super::*;
    use std::sync::Arc;

    /// Boot an in-memory hub on an ephemeral port; return its ws:// URL.
    async fn start_hub() -> String {
        let store = parler_hub::Store::open(None).unwrap();
        let state = Arc::new(parler_hub::HubState::new(
            store,
            "parler://test".into(),
            "Test Hub".into(),
            parler_hub::HubMode::Private,
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = parler_hub::serve(listener, state).await;
        });
        format!("ws://{addr}")
    }

    /// A fresh in-memory identity connected to `hub`, subscribed for push (never touches
    /// PARLER_HOME).
    async fn state(hub: &str, name: &str) -> McpState {
        let cfg = Config::create(hub.to_string(), name.to_string(), None).unwrap();
        let mut agent = MeshAgent::connect(&cfg).await.unwrap();
        let push = agent.subscribe().await.unwrap_or(false);
        McpState { agent, active_session: None, push }
    }

    /// Pull the `KEY: <code>` line out of an `open_session` result.
    fn key_of(open_result: &str) -> String {
        open_result
            .split("KEY: ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .expect("a KEY in the open_session output")
            .to_string()
    }

    #[tokio::test]
    async fn open_then_join_shares_context_and_sets_active_session() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let opened = open_session(&mut alice, Some("designing the auth flow; see src/auth.rs"), Some("design".into()), None, None)
            .await
            .unwrap();
        assert!(opened.contains("KEY: "));
        assert!(alice.active_session.is_some());

        let key = key_of(&opened);
        let joined = join_session(&mut bob, &key).await.unwrap();
        assert!(joined.contains("designing the auth flow"), "joiner should receive the seeded context");
        assert_eq!(bob.active_session, alice.active_session, "both share the same session room");
    }

    #[tokio::test]
    async fn open_without_context_posts_no_seed() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let opened = open_session(&mut alice, None, Some("empty".into()), None, None).await.unwrap();
        let key = key_of(&opened);
        // Bob joins; the only backlog should be his own "joined" announce — no seed context line.
        let joined = join_session(&mut bob, &key).await.unwrap();
        assert!(!joined.contains("session context"), "no seed message when context is omitted");
    }

    #[tokio::test]
    async fn send_defaults_to_active_session_and_autopull_filters_own() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let opened = open_session(&mut alice, Some("seed"), Some("design".into()), None, None).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key).await.unwrap();

        // Bob sends with no explicit target → goes to the active session.
        let bob_send = call_session_tool(&mut bob, "parler_send", &json!({ "text": "from bob" })).await.unwrap();
        assert!(bob_send.contains("sent to"));

        // Alice sends → the auto-pull surfaces bob's message but filters alice's own (seed + this one).
        let alice_send = call_session_tool(&mut alice, "parler_send", &json!({ "text": "from alice" })).await.unwrap();
        assert!(alice_send.contains("from bob"), "auto-pull should surface the peer's message");
        assert!(!alice_send.contains("from alice"), "auto-pull must filter the sender's own messages");
        assert!(!alice_send.contains("📋 session context"), "own seed is filtered too");
    }

    #[tokio::test]
    async fn recv_defaults_to_active_session() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let opened = open_session(&mut alice, Some("seed"), None, None, None).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key).await.unwrap(); // advances bob's cursor to the live edge

        // Alice posts after bob is caught up; bob's recv (no room) picks it up from the active session.
        call_session_tool(&mut alice, "parler_send", &json!({ "text": "ping bob" })).await.unwrap();
        let recv = call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();
        assert!(recv.contains("ping bob"));
    }

    #[tokio::test]
    async fn recv_wait_secs_long_polls_for_a_push() {
        // With nothing waiting, `parler_recv` + wait_secs blocks until a peer's message is pushed,
        // then returns it — sub-second, no polling. (state() subscribes for push.)
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;
        assert!(bob.push, "the hub should support push so recv can long-poll");

        let opened = open_session(&mut alice, Some("seed"), None, None, None).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key).await.unwrap(); // bob caught up to the live edge
        // join_session posts bob's own "joined" announce *after* its catch-up pull, so it now sits
        // past bob's cursor — drain it so the long-poll below starts from a genuinely empty inbox
        // (otherwise the initial pull returns non-empty and short-circuits the wait).
        call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();

        // Bob long-polls while, concurrently, alice sends after a short delay → the push wakes bob.
        let send_args = json!({ "text": "ping bob" });
        let recv_args = json!({ "wait_secs": 5 });
        let send = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            call_session_tool(&mut alice, "parler_send", &send_args).await.unwrap();
        };
        let recv = call_session_tool(&mut bob, "parler_recv", &recv_args);
        let (_sent, got) = tokio::join!(send, recv);
        assert!(got.unwrap().contains("ping bob"), "long-poll recv should wake on the pushed message");
    }

    #[tokio::test]
    async fn send_without_target_or_session_errors() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        // No active session and no room/to/service → an error.
        assert!(call_session_tool(&mut alice, "parler_send", &json!({ "text": "hi" })).await.is_err());
    }

    #[tokio::test]
    async fn close_session_clears_active_session() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;

        open_session(&mut alice, Some("seed"), None, None, None).await.unwrap();
        assert!(alice.active_session.is_some());
        let closed = close_session(&mut alice).await.unwrap();
        assert!(closed.contains("left session"));
        assert!(alice.active_session.is_none());
        // Closing again is a no-op, not an error.
        assert!(close_session(&mut alice).await.unwrap().contains("no active session"));
    }

    #[tokio::test]
    async fn run_loop_lists_session_tools_and_calls_open() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;

        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"parler_open_session\",\"arguments\":{\"context\":\"hi\"}}}\n",
        );
        let mut output: Vec<u8> = Vec::new();
        run(&mut alice, BufReader::new(input.as_bytes()), &mut output).await.unwrap();
        let out = String::from_utf8(output).unwrap();

        // initialize advertised the server; tools/list registered the new session tools.
        assert!(out.contains("\"protocolVersion\""));
        assert!(out.contains("parler_open_session"));
        assert!(out.contains("parler_join_session"));
        assert!(out.contains("parler_close_session"));
        // the open_session call ran and returned a key, and set the active session.
        assert!(out.contains("KEY: "));
        assert!(alice.active_session.is_some());
    }
}
