//! `parler mcp` — a minimal MCP (Model Context Protocol) server over stdio.
//!
//! MCP is JSON-RPC 2.0 with newline-delimited messages on stdio. We implement just the methods a
//! host needs — `initialize`, `tools/list`, `tools/call`, `ping` — and map each `parler_*` tool
//! onto the same [`MeshAgent`] the CLI uses. Hand-rolled on purpose: it keeps the dependency
//! surface tiny and gives exact control over the wire, which matters more than an SDK here.

use anyhow::{anyhow, bail, Result};
use parler_connector::{BundleMeta, Config, JoinOutcome, MeshAgent};
use parler_protocol::{AgentSkill, DiscoverScope, HandoffRef, RoomKind, StoredMessage, Target, Visibility};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// The always-on, world-readable hub a fresh agent joins by default (override with `PARLER_HUB`).
/// Shared with [`crate::connect`] so both the bootstrap and the wiring command agree on one URL.
pub(crate) const DEFAULT_PUBLIC_HUB: &str = "wss://parler-hub.fly.dev";

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
    let mut agent = match MeshAgent::connect(&cfg).await {
        Ok(a) => a,
        Err(e) => {
            // Leave a breadcrumb before we exit: a GUI host swallows stderr, so `parler doctor`
            // reading this log is often the only way a user learns *why* the agent went dark.
            log_event(&format!("connect FAILED → {}: {e}", cfg.hub_url));
            return Err(e);
        }
    };
    log_event(&format!("connected as {} ({}) → {}", cfg.name, cfg.identity.id, cfg.hub_url));
    // Opt into sub-second push so `parler_recv` can long-poll for replies (best-effort; against an
    // older hub this is a no-op and we stay purely pull-based).
    let push = agent.subscribe().await.unwrap_or(false);
    // Self-list on the hub the moment we connect, so a freshly wired agent is visible to same-hub
    // peers (and shows up under the desktop app's Agents) without a human having to call
    // `parler_register` first — "connected" should mean "discoverable". Private by default
    // (same-hub only); opt into the public directory or enrich the card via env. Best-effort.
    auto_register(&mut agent).await;
    let mut state = McpState { agent, active_session: None, push };

    // Spin-up convenience: if a session key was handed in via the environment, join it now so a
    // freshly launched agent is already in the shared conversation (with its context) before the
    // host makes a single tool call. Failures are non-fatal — log to stderr (stdout is the
    // protocol channel) and carry on.
    if let Some(key) = std::env::var("PARLER_SESSION_KEY").ok().filter(|s| !s.is_empty()) {
        match join_session(&mut state, &key, Backlog::Recent).await {
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
pub(crate) fn load_or_bootstrap_config() -> Result<Config> {
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
    // First-run visibility: announce the freshly minted identity so a user (or `parler doctor`) can
    // confirm setup took, instead of an identity materializing silently.
    eprintln!("parler: initialized new agent {} ({}) → {}", cfg.name, cfg.identity.id, cfg.hub_url);
    log_event(&format!("bootstrapped identity {} → {}", cfg.identity.id, cfg.hub_url));
    Ok(cfg)
}

/// Where the MCP connection breadcrumb log lives (`~/.parler/mcp.log`).
fn mcp_log_path() -> std::path::PathBuf {
    parler_connector::home_dir().join("mcp.log")
}

const LOG_KEEP: usize = 200;

/// Append a timestamped line to the breadcrumb log (best-effort — diagnostics, never load-bearing),
/// trimmed to the most recent [`LOG_KEEP`] lines so it can't grow without bound.
fn log_event(msg: &str) {
    log_event_at(&mcp_log_path(), unix_secs(), msg);
}

fn log_event_at(path: &std::path::Path, ts_secs: u64, msg: &str) {
    let mut lines: Vec<String> = std::fs::read_to_string(path)
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default();
    lines.push(format!("{ts_secs}\t{}", msg.replace('\n', " ")));
    let start = lines.len().saturating_sub(LOG_KEEP);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, lines[start..].join("\n") + "\n");
}

/// The most recent `n` breadcrumb entries as `(relative-age, message)`, oldest-first. `None` if no
/// log exists yet. Read back by `parler doctor` so "did my MCP agent actually connect?" is answerable.
pub(crate) fn recent_log(n: usize) -> Option<Vec<(String, String)>> {
    recent_log_at(&mcp_log_path(), unix_secs(), n)
}

fn recent_log_at(path: &std::path::Path, now_secs: u64, n: usize) -> Option<Vec<(String, String)>> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut out: Vec<(String, String)> = content
        .lines()
        .rev()
        .take(n)
        .filter_map(|l| {
            let (ts, msg) = l.split_once('\t')?;
            let secs = ts.parse::<u64>().ok()?;
            Some((fmt_ago(now_secs.saturating_sub(secs)), msg.to_string()))
        })
        .collect();
    out.reverse();
    Some(out)
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn fmt_ago(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// Publish a directory card for this agent on startup so it's discoverable the instant it connects.
///
/// Private (same-hub) by default to preserve secure-by-default visibility; set `PARLER_PUBLIC=1` to
/// list in the world-readable directory (mirrors `parler register --public`). The card can be
/// enriched from the same env the MCP config already carries: `PARLER_TAGS` / `PARLER_SKILLS`
/// (comma-separated) and `PARLER_DESCRIBE`. Opt out entirely with `PARLER_NO_REGISTER=1`. Failures
/// are non-fatal — stderr only (stdout is the JSON-RPC channel) — so a card-averse hub never blocks
/// the agent from running.
async fn auto_register(agent: &mut MeshAgent) {
    if env_flag("PARLER_NO_REGISTER") {
        return;
    }
    let visibility = if env_flag("PARLER_PUBLIC") { Visibility::Public } else { Visibility::Private };
    let tags = env_list("PARLER_TAGS");
    let skills = env_list("PARLER_SKILLS")
        .into_iter()
        .map(|s| AgentSkill { id: s.clone(), name: s, description: None })
        .collect();
    let describe = std::env::var("PARLER_DESCRIBE").ok().filter(|s| !s.trim().is_empty());
    match agent.register(visibility, tags, skills, describe).await {
        Ok(_) => log_event("auto-registered card (discoverable)"),
        Err(e) => {
            eprintln!("parler: auto-register failed (agent still connected): {e}");
            log_event(&format!("auto-register failed: {e}"));
        }
    }
}

/// A truthy env flag: set and not one of `0`/`false`/`no`/`` (case-insensitive).
fn env_flag(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no"))
        .unwrap_or(false)
}

/// Split a comma/whitespace-separated env var into trimmed, non-empty tokens.
fn env_list(key: &str) -> Vec<String> {
    std::env::var(key)
        .ok()
        .map(|v| {
            v.split([',', ' '])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Dispatch one JSON-RPC method. `Err((code, message))` becomes a JSON-RPC error.
async fn handle(state: &mut McpState, method: &str, params: Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": {},
                "resources": {},
                "prompts": {}
            },
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
                | "parler_send" | "parler_recv" | "parler_handoff" | "parler_join_requests"
                | "parler_approve_join" | "parler_deny_join" | "parler_watch_session" => {
                    call_session_tool(state, &name, &args).await
                }
                _ => call_tool(&mut state.agent, &name, &args).await,
            };
            // Per MCP, a tool's own failure is a result with isError=true, not a protocol error.
            match result {
                Ok(text) => Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": false })),
                Err(e) => Ok(json!({ "content": [{ "type": "text", "text": format!("error: {e}") }], "isError": true })),
            }
        }
        "resources/list" => Ok(json!({
            "resources": [
                {
                    "uri": "parler://active-session/backlog",
                    "name": "Active Session Backlog",
                    "description": "Full chronological history of the active session — the explicit full-replay escape hatch when the join/recv digests aren't enough. The hub returns up to 200 messages per page (page older ranges with parler_recv since=<seq>).",
                    "mimeType": "text/plain"
                },
                {
                    "uri": "parler://roster",
                    "name": "Session Roster",
                    "description": "The list of agents currently in the active session room.",
                    "mimeType": "application/json"
                }
            ]
        })),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).ok_or_else(|| (-32602, "missing 'uri'".to_string()))?;
            match uri {
                "parler://active-session/backlog" => {
                    let room = state.active_session.clone().ok_or_else(|| (-32602, "no active session room".to_string()))?;
                    let (msgs, _) = state.agent.pull(&room, Some(0), None).await.map_err(|e| (-32000, format!("failed to pull messages: {e}")))?;
                    let text = msgs.iter().map(crate::render_message).collect::<Vec<_>>().join("\n");
                    Ok(json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "text/plain",
                            "text": text
                        }]
                    }))
                }
                "parler://roster" => {
                    let room = state.active_session.clone().ok_or_else(|| (-32602, "no active session room".to_string()))?;
                    let entries = state.agent.roster(&room).await.map_err(|e| (-32000, format!("failed to fetch roster: {e}")))?;
                    Ok(json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "application/json",
                            "text": serde_json::to_string(&entries).unwrap_or_default()
                        }]
                    }))
                }
                other => Err((-32602, format!("unknown resource URI: {other}")))
            }
        }
        "prompts/list" => Ok(json!({
            "prompts": [
                {
                    "name": "parler_session_handoff",
                    "description": "Instructs an agent on how to consume the active session backlog and resume work.",
                    "arguments": []
                },
                {
                    "name": "parler_consolidate_session",
                    "description": "Instructs the agent on how to consolidate the active session backlog into key facts.",
                    "arguments": []
                }
            ]
        })),
        "prompts/get" => {
            let name = params.get("name").and_then(Value::as_str).ok_or_else(|| (-32602, "missing 'name'".to_string()))?;
            match name {
                "parler_session_handoff" => {
                    let room = state.active_session.clone().unwrap_or_else(|| "none".to_string());
                    // Digest the backlog (seed + recent tail + an omission line), not a full replay —
                    // the same token-efficient render a late join gets. `since=Some(0)` reads history
                    // without touching the cursor.
                    let mut backlog = String::new();
                    let mut roster_count = 0usize;
                    if let Some(ref r) = state.active_session {
                        if let Ok((msgs, _)) = state.agent.pull(r, Some(0), None).await {
                            backlog = digest_backlog(&msgs, Backlog::Recent);
                        }
                        if let Ok(entries) = state.agent.roster(r).await {
                            roster_count = entries.len();
                        }
                    }
                    let text = format!(
                        "You are joining a Parler collaborative session.\n\
                         Active Room: {room} — {roster_count} agent(s) in the room.\n\n\
                         Context so far (digest — seed + recent messages):\n\
                         {backlog}\n\n\
                         For anything older, parler_recv since=<seq> re-reads a range in full and \
                         parler_recall surfaces saved decisions. Review this, then continue \
                         coordinating with the other agents or address the task at hand."
                    );
                    Ok(json!({
                        "description": "Handoff instructions for the active session",
                        "messages": [{
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": text
                            }
                        }]
                    }))
                }
                "parler_consolidate_session" => {
                    let room = state.active_session.clone().unwrap_or_else(|| "none".to_string());
                    // Analyze at most the last 100 messages — enough to consolidate recent work
                    // without pulling an unbounded backlog into context.
                    let mut backlog = String::new();
                    if let Some(ref r) = state.active_session {
                        if let Ok((msgs, _)) = state.agent.pull(r, Some(0), None).await {
                            let tail = &msgs[msgs.len().saturating_sub(100)..];
                            backlog = tail.iter().map(crate::render_message).collect::<Vec<_>>().join("\n");
                        }
                    }
                    let text = format!(
                        "Analyze this recent conversation from a collaborative session (Room: {room}).\n\
                         Extract 1 to 5 key decisions, architectural choices, modified file paths, or lessons learned, \
                         then write a concise rolling recap with:\n\
                         parler_remember key=\"session-digest\" room=\"{room}\" text=\"SESSION DIGEST: …\"\n\
                         Re-saving that key overwrites it, so late joiners always get the current summary cheaply.\n\n\
                         Recent messages:\n{backlog}"
                    );
                    Ok(json!({
                        "description": "Consolidate the session backlog into facts",
                        "messages": [{
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": text
                            }
                        }]
                    }))
                }
                other => Err((-32602, format!("unknown prompt: {other}")))
            }
        }
        "ping" => Ok(json!({})),
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

fn parse_embedding(v: Option<&Value>) -> Option<Vec<f32>> {
    v.and_then(Value::as_array).map(|a| {
        a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect()
    })
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
            let target = crate::resolve_target(agent, target).await?;
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
            let embedding = parse_embedding(args.get("embedding"));
            agent.remember(&text, s("key"), s("room"), embedding, s("embedding_model")).await?;
            Ok("remembered".into())
        }
        "parler_recall" => {
            let query = s("query").ok_or_else(|| anyhow!("missing 'query'"))?;
            let embedding = parse_embedding(args.get("embedding"));
            let hits = agent.recall(&query, s("room"), u32opt("limit"), embedding).await?;
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
            let detail = args.get("detail").and_then(Value::as_bool).unwrap_or(false);
            let entries = agent.roster(&room).await?;
            Ok(entries
                .iter()
                .map(|e| {
                    let role = e.role.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
                    if detail {
                        format!("{}{role} {} [{}]", e.name, e.id, e.status)
                    } else {
                        format!("{}{role} [{}]", e.name, e.status)
                    }
                })
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
            let detail = args.get("detail").and_then(Value::as_bool).unwrap_or(false);
            // Client default cap: a full directory listing renders one long id line each, which is
            // costly context. Cap unless the caller asks for more (or full detail). Compact lines
            // drop the 56-char id; `detail:true` restores it (needed to DM/card an agent by id).
            let applied = u32opt("limit").unwrap_or(DISCOVER_DEFAULT_LIMIT);
            let agents = agent
                .discover(scope, s("query"), s("tag"), s("skill"), s("status"), Some(applied))
                .await?;
            if agents.is_empty() {
                return Ok("(no agents found)".into());
            }
            let mut out = agents
                .iter()
                .map(|e| {
                    let role = e.card.role.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
                    let tags = e.card.tags.as_deref().map(|t| t.join(",")).unwrap_or_default();
                    let flag = if e.verified { " ✓" } else { "" };
                    if detail {
                        format!(
                            "{}{role} [{}{flag}] {} — {} — {}",
                            e.card.name, e.visibility.as_str(), e.card.id, e.status, tags
                        )
                    } else {
                        // Compact: name (role) [vis✓] status — tags. No id (use detail:true, or the
                        // name works directly with parler_send to / parler_card).
                        format!(
                            "{}{role} [{}{flag}] {} — {}",
                            e.card.name, e.visibility.as_str(), e.status, tags
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            // A full batch likely means there are more — nudge toward narrowing instead of a bigger dump.
            if agents.len() as u32 >= applied {
                out.push_str("\n— more agents may match; narrow with query/tag/skill or raise limit —");
            }
            Ok(out)
        }
        "parler_card" => {
            let id = s("id").ok_or_else(|| anyhow!("missing 'id'"))?;
            // Accept a directory name as well as a full id (same unique-match-or-error resolver as
            // `to`): a name is resolved to its id before the lookup, an id passes straight through.
            let id = match crate::resolve_target(agent, Target::Dm { agent: id.clone() }).await? {
                Target::Dm { agent } => agent,
                _ => id,
            };
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
            // Approval defaults ON: a session is a live conversation, so the host vets each joiner
            // before they can read it. Pass approval=false to revert to open (paste-and-join) keys.
            let approval = args.get("approval").and_then(Value::as_bool).unwrap_or(true);
            open_session(
                state,
                context.as_deref(),
                s("topic"),
                args.get("ttl_secs").and_then(Value::as_u64),
                u32opt("max_uses"),
                approval,
            )
            .await
        }
        "parler_join_session" => {
            let key = s("key").ok_or_else(|| anyhow!("missing 'key'"))?;
            join_session(state, &key, Backlog::from_arg(s("backlog").as_deref())).await
        }
        "parler_close_session" => close_session(state).await,
        "parler_join_requests" => {
            let room = s("room")
                .or_else(|| state.active_session.clone())
                .ok_or_else(|| anyhow!("missing 'room' (open a session, or pass room)"))?;
            let reqs = state.agent.join_requests(&room).await?;
            if reqs.is_empty() {
                return Ok(format!("(no agents waiting to join '{room}')"));
            }
            Ok(reqs
                .iter()
                .map(|r| {
                    let role = r.role.as_deref().map(|x| format!(" ({x})")).unwrap_or_default();
                    format!(
                        "• {}{role} [{}] — approve: parler_approve_join agent={} | reject: parler_deny_join agent={}",
                        r.name, r.agent, r.agent, r.agent
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "parler_approve_join" | "parler_deny_join" => {
            let room = s("room")
                .or_else(|| state.active_session.clone())
                .ok_or_else(|| anyhow!("missing 'room' (open a session, or pass room)"))?;
            let who = s("agent").ok_or_else(|| anyhow!("missing 'agent' (the joiner's id to resolve)"))?;
            let approve = name == "parler_approve_join";
            let approved = state.agent.resolve_join(&room, &who, approve).await?;
            Ok(if approved {
                format!("✓ approved {who} into session '{room}' — they can now read the conversation and participate.")
            } else {
                format!("✗ denied {who}'s request to join session '{room}'.")
            })
        }
        "parler_watch_session" => {
            let room = s("room")
                .or_else(|| state.active_session.clone())
                .ok_or_else(|| anyhow!("missing 'room' (open a session, or pass room)"))?;
            let ttl = args.get("ttl_secs").and_then(Value::as_u64);
            let (token, _expires_at) = state.agent.mint_watch_token(&room, ttl).await?;
            Ok(format!(
                "read-only WATCH code for session '{room}':\n{token}\n\
                 Give it to the user to paste into the Parler website's session viewer (the /session \
                 page) — they'll see the conversation and how many agents are in the room, without \
                 joining. It's read-only and expiring, but anyone with the code can read the session, \
                 so treat it like a password."
            ))
        }
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
            // Let `to` be a directory name, not just a 56-char id (unique-match-or-error; never guess).
            let target = crate::resolve_target(&mut state.agent, target).await?;
            let (_id, seq, room) = state.agent.send_text(target, &text).await?;
            // Auto-pull right after sending so an already-waiting reply shows up without a separate
            // parler_recv (read-after-write); for a reply that hasn't landed yet, use parler_recv with
            // wait_secs to long-poll. Our own just-sent message is filtered out; the pull advances our
            // cursor so these aren't re-delivered later. The pull is capped (AUTOPULL_LIMIT) so a
            // reply flood can't balloon the send result — the remainder stays unread for the next
            // parler_recv (a limited pull only advances the cursor through what it returned).
            let mut out = format!("sent to '{room}' (seq {seq})");
            let auto_limit = if verbose_render() { None } else { Some(AUTOPULL_LIMIT) };
            if let Ok((msgs, _cursor)) = state.agent.pull(&room, None, auto_limit).await {
                let batch_full = auto_limit.is_some_and(|l| msgs.len() as u32 >= l);
                let me = state.agent.id.clone();
                let incoming: Vec<_> = msgs.iter().filter(|m| m.from.id != me).collect();
                if !incoming.is_empty() {
                    if let Some(banner) = handoff_banner(state, &incoming) {
                        out.push_str(&format!("\n\n{banner}"));
                    }
                    let body = incoming
                        .into_iter()
                        .map(render_message_budgeted)
                        .collect::<Vec<_>>()
                        .join("\n");
                    out.push_str(&format!("\n— new messages —\n{body}"));
                    if batch_full {
                        out.push_str("\n— more waiting: call parler_recv again —");
                    }
                }
            }
            // Surface any pending join requests so the host (this owner) is shown the accept/reject
            // choice in the natural flow of the conversation, without having to poll for it.
            if let Some(notice) = pending_join_notice(state, &room).await {
                out.push_str(&notice);
            }
            Ok(out)
        }
        "parler_handoff" => {
            let next = s("next").ok_or_else(|| anyhow!("missing 'next' (what the next agent should do)"))?;
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
            let target = crate::resolve_target(&mut state.agent, target).await?;
            let handoff = HandoffRef { next, summary: s("summary"), to: s("for"), bundle: s("bundle") };
            let mentions = handoff.to.clone().map(|w| vec![w]);
            let (_id, seq, room) =
                state.agent.send(target, vec![handoff.to_part()], mentions, None).await?;
            let whom = handoff.to.as_deref().unwrap_or("anyone in the room");
            Ok(format!(
                "✓ handed off to {whom} in '{room}' (seq {seq}). They'll see a 'HANDOFF TO YOU' \
                 banner on their next parler_recv (or sooner if they're long-polling with wait_secs)."
            ))
        }
        "parler_recv" => {
            let room = s("room")
                .or_else(|| state.active_session.clone())
                .ok_or_else(|| anyhow!("missing 'room' (open/join a session, or pass room)"))?;
            let since = args.get("since").and_then(Value::as_i64);
            let explicit_limit = u32opt("limit");
            // Cursor mode with no explicit limit → apply the default cap (lossless: a limited pull
            // advances the cursor only through the batch, so the rest waits for the next call). An
            // explicit `since` is a full-detail history re-read: never cap it, never budget its bodies.
            let re_read = since.is_some();
            let effective_limit = recv_limit(explicit_limit, re_read, verbose_render());
            let (mut msgs, mut cursor) = state.agent.pull(&room, since, effective_limit).await?;
            // Long-poll: if nothing new yet and the caller asked to wait (and the hub is pushing),
            // block up to `wait_secs` for a peer message, then re-pull to read + advance the cursor.
            // Only in cursor mode (`since` absent) — an explicit `since` is a history re-read.
            if msgs.is_empty() && since.is_none() && state.push {
                if let Some(secs) = args.get("wait_secs").and_then(Value::as_u64).filter(|w| *w > 0) {
                    let secs = secs.min(60);
                    if state.agent.next_delivery(Duration::from_secs(secs)).await?.is_some() {
                        let (m, c) = state.agent.pull(&room, None, effective_limit).await?;
                        msgs = m;
                        cursor = c;
                    }
                }
            }
            let batch_full = effective_limit.is_some_and(|l| msgs.len() as u32 >= l);
            let mut out = if msgs.is_empty() {
                format!("(no new messages in '{room}')")
            } else {
                let refs: Vec<&_> = msgs.iter().collect();
                // Re-reads (explicit `since`) render in full; cursor-mode reads budget long bodies.
                let body = msgs
                    .iter()
                    .map(|m| if re_read { crate::render_message(m) } else { render_message_budgeted(m) })
                    .collect::<Vec<_>>()
                    .join("\n");
                let more = if batch_full { "\n— more waiting: call parler_recv again —" } else { "" };
                match handoff_banner(state, &refs) {
                    Some(banner) => format!("{banner}\n\n{body}\n— cursor at {cursor} —{more}"),
                    None => format!("{body}\n— cursor at {cursor} —{more}"),
                }
            };
            // Surface any pending join requests so a host sees the accept/reject choice inline, even
            // when there are no new messages.
            if let Some(notice) = pending_join_notice(state, &room).await {
                out.push_str(&notice);
            }
            Ok(out)
        }
        other => bail!("unknown session tool: {other}"),
    }
}

/// Open a shared session: mint a multi-use channel invite (the key), seed it with the caller's
/// context snapshot so late joiners get caught up, and adopt it as the active session. With
/// `require_approval` (the default), the key only lets an agent *ask* to join — this host must
/// approve each one before it can read the conversation, so a leaked key can't quietly pull the
/// context.
async fn open_session(
    state: &mut McpState,
    context: Option<&str>,
    topic: Option<String>,
    ttl_secs: Option<u64>,
    max_uses: Option<u32>,
    require_approval: bool,
) -> Result<String> {
    let inv = state
        .agent
        .invite_with_approval(RoomKind::Channel, topic.clone(), ttl_secs, max_uses, require_approval)
        .await?;
    let room = inv.room.clone();
    // The live conversation lives in the host LLM, not the hub — snapshot it as the room's first
    // message so anyone who joins reads the context by pulling history.
    if let Some(ctx) = context.map(str::trim).filter(|c| !c.is_empty()) {
        let seed = format!("📋 session context (from {}):\n{ctx}", state.agent.name);
        state.agent.send_text(Target::Room { room: room.clone() }, &seed).await?;
    }
    state.active_session = Some(room.clone());
    let _ = crate::save_active_session(&room);
    let _ = state.agent.presence("working", topic.or_else(|| Some("shared session".into()))).await;
    let gate = if require_approval {
        "When another agent redeems this key it will ask to join, and YOU must approve it before it \
         can see the conversation — you'll be shown a prompt to accept or reject (or call \
         parler_join_requests). This keeps a leaked key from quietly reading your context."
    } else {
        "Anyone with this key joins immediately (approval disabled)."
    };
    // A ready-to-paste one-liner the host can drop straight into Slack/Discord: it adds the Parler
    // MCP server already pointed at this session, so a teammate joins with a single command and no
    // prior setup. Carries the hub + join secret when they aren't the defaults, so it also works on a
    // private/team hub. (The joiner still lands pending your approval — the gate is unchanged.)
    let oneliner = {
        let mut env = format!("-e PARLER_SESSION_KEY={}", inv.code);
        if state.agent.hub_url != DEFAULT_PUBLIC_HUB {
            env.push_str(&format!(" -e PARLER_HUB={}", state.agent.hub_url));
        }
        if let Some(secret) = std::env::var("PARLER_JOIN_SECRET").ok().filter(|s| !s.is_empty()) {
            env.push_str(&format!(" -e PARLER_JOIN_SECRET={secret}"));
        }
        format!("claude mcp add parler {env} -- parler mcp")
    };
    Ok(format!(
        "session open — room '{room}', now your active session (parler_send / parler_recv default to it).\n\
         KEY: {code}\n\
         \n\
         Share with a teammate (or your own agent in another repo) — send them either:\n\
         • the KEY above (they call parler_join_session with it), or\n\
         • this one-liner, which adds Parler already joined to this session:\n    \
         {oneliner}\n\
         Either way they land in the SAME conversation with the full context — no copy-paste.\n\
         {gate}\n\
         To let the user watch this session in their browser, call parler_watch_session for a read-only \
         web viewer code.\n\
         link: {url}",
        code = inv.code,
        url = inv.url,
    ))
}

/// The marker the seed context message starts with (posted by `open_session`). A late joiner always
/// gets the seed rendered in full — it's the recap the host wrote — while the middle of the backlog is
/// summarized to an "N omitted" line. Kept in one place so the write path and the digest agree.
const SEED_MARKER: &str = "📋 session context";

/// How many trailing messages a "recent" join/handoff digest renders in full. Enough to see the live
/// thread of the conversation; the rest is one omission line pointing at `parler_recv since=<seq>`.
const JOIN_TAIL: usize = 15;

/// Default cap on how many messages a cursor-mode `parler_recv` renders per call. Lossless: a limited
/// `Pull` advances the cursor only through the returned batch, so the remainder stays unread for the
/// next call (see store.rs). An explicit `limit`/`since` overrides this.
const RECV_DEFAULT_LIMIT: u32 = 30;

/// Default cap on the auto-pull appended to a `parler_send` result. Same losslessness as
/// [`RECV_DEFAULT_LIMIT`]; a peer flood past this resurfaces on the next `parler_recv`.
const AUTOPULL_LIMIT: u32 = 10;

/// Per-message body cap (chars) for the budgeted render. A longer body is truncated with a pointer to
/// re-read that one message in full. Big enough that ordinary chat is never touched.
const MSG_MAX_CHARS: usize = 1200;

/// Client-side default cap on `parler_discover` results — a full directory renders one id-bearing line
/// each, costly context. The caller raises it (or narrows with query/tag/skill) when they need more.
const DISCOVER_DEFAULT_LIMIT: u32 = 25;

/// Render a message, truncating an over-long body to [`MSG_MAX_CHARS`] with a hint to re-read it in
/// full via an explicit `since` (which is never truncated). UTF-8 safe (truncates on a char boundary).
/// Used only on the *cursor-mode* render paths (recv default, auto-pull); explicit-`since` re-reads,
/// the seed, and banners always render in full through [`crate::render_message`].
fn render_message_budgeted(m: &StoredMessage) -> String {
    let full = crate::render_message(m);
    if full.chars().count() <= MSG_MAX_CHARS {
        return full;
    }
    let kept: String = full.chars().take(MSG_MAX_CHARS).collect();
    let dropped = full.chars().count() - MSG_MAX_CHARS;
    // since = seq-1 so `parler_recv since=<seq-1> limit=1` returns exactly this message, in full.
    format!("{kept}…[+{dropped} chars — parler_recv since={} limit=1 for full]", m.seq - 1)
}

/// True when the caller wants uncapped output — the global escape hatch (`PARLER_MCP_VERBOSE=1`).
fn verbose_render() -> bool {
    env_flag("PARLER_MCP_VERBOSE")
}

/// The message limit a `parler_recv` should pull with. Pure so it's unit-testable without touching the
/// process env: an explicit `limit` always wins; a history re-read (`since`) or verbose mode is
/// uncapped (`None`); otherwise the default cap applies.
fn recv_limit(explicit: Option<u32>, re_read: bool, verbose: bool) -> Option<u32> {
    match (explicit, re_read, verbose) {
        (Some(l), _, _) => Some(l),
        (None, false, false) => Some(RECV_DEFAULT_LIMIT),
        _ => None,
    }
}

/// Whether a join renders the whole backlog or a digest. `Recent` (default) is the token-efficient
/// path: seed + tail + an omission line. `Full` is the escape hatch — replay everything.
#[derive(Clone, Copy, PartialEq)]
enum Backlog {
    Recent,
    Full,
}

impl Backlog {
    fn from_arg(v: Option<&str>) -> Self {
        match v {
            Some("full") => Backlog::Full,
            _ => Backlog::Recent,
        }
    }
}

/// Render a room backlog for a late joiner (and, via P1.1, the handoff prompt). In `Full` mode, or
/// when the backlog is already short (≤ [`JOIN_TAIL`]), render every message. Otherwise digest:
/// the context seed (always, in full) → an "N earlier messages omitted" line naming the
/// `parler_recv since=<seq>` re-read → the last [`JOIN_TAIL`] messages in full. This is the ~85% cut
/// for a late joiner: a full replay is paid by the LLM verbatim, but the middle is rarely needed and
/// stays one `since`/`recall` call away.
fn digest_backlog(msgs: &[StoredMessage], mode: Backlog) -> String {
    if msgs.is_empty() {
        return "(no prior context yet)".to_string();
    }
    let render_all = || msgs.iter().map(crate::render_message).collect::<Vec<_>>().join("\n");
    if mode == Backlog::Full || msgs.len() <= JOIN_TAIL {
        return render_all();
    }
    // The seed is the earliest message whose rendered body opens with the marker (open_session posts
    // it first). Rendered in full regardless of where the tail window starts, so the recap is never lost.
    let seed_idx = msgs
        .iter()
        .position(|m| crate::render_parts(&m.parts).trim_start().starts_with(SEED_MARKER));
    let tail_start = msgs.len() - JOIN_TAIL;
    let mut out = Vec::new();
    // Seed (if it exists and isn't already inside the tail window).
    if let Some(i) = seed_idx {
        if i < tail_start {
            out.push(crate::render_message(&msgs[i]));
        }
    }
    // The omitted middle: everything between the seed (exclusive) / start and the tail window. The
    // re-read seq is one before the first omitted message so `parler_recv since=<seq>` returns it.
    let omitted_start = match seed_idx {
        Some(i) if i < tail_start => i + 1,
        _ => 0,
    };
    if omitted_start < tail_start {
        let n = tail_start - omitted_start;
        let resume = msgs[omitted_start].seq - 1;
        out.push(format!(
            "— {n} earlier message(s) omitted; parler_recv since={resume} to re-read, parler_recall for decisions —"
        ));
    }
    // The live tail, in full.
    for m in &msgs[tail_start..] {
        out.push(crate::render_message(m));
    }
    out.join("\n")
}

/// Join a shared session by key. For an approval-gated session the redeem only *requests* entry —
/// the host must admit us first; we poll briefly for a fast approval, then return a clear "pending"
/// message the agent can retry on. Once admitted, pull the backlog to catch up (and advance the
/// cursor to the live edge), adopt it as the active session, and announce arrival. The backlog is
/// rendered as a digest by default (`Backlog::Recent`) — `Backlog::Full` replays everything.
async fn join_session(state: &mut McpState, key: &str, backlog: Backlog) -> Result<String> {
    let room = match state.agent.redeem(key).await? {
        JoinOutcome::Joined { room, .. } => room,
        JoinOutcome::Pending { room } => {
            // Short poll so a quick host approval still resolves in this one call; a denial surfaces
            // as an error from redeem and propagates out.
            let mut admitted = None;
            for _ in 0..JOIN_POLL_ATTEMPTS {
                tokio::time::sleep(std::time::Duration::from_millis(JOIN_POLL_INTERVAL_MS)).await;
                match state.agent.redeem(key).await? {
                    JoinOutcome::Joined { room, .. } => {
                        admitted = Some(room);
                        break;
                    }
                    JoinOutcome::Pending { .. } => continue,
                }
            }
            match admitted {
                Some(room) => room,
                None => {
                    return Ok(format!(
                        "⏳ join request sent — waiting for the host to approve you into session '{room}'.\n\
                         You are NOT in the conversation yet and cannot see its context until the host \
                         approves. Call parler_join_session again with the same key to check.",
                    ))
                }
            }
        }
    };
    // since=None advances our fresh cursor to the live edge (this full pull is load-bearing), so a
    // later parler_recv only returns genuinely new messages rather than re-delivering this backlog.
    // We render a *digest* of what we pulled — the cursor still advanced past all of it.
    let (msgs, _cursor) = state.agent.pull(&room, None, None).await?;
    state.active_session = Some(room.clone());
    let _ = crate::save_active_session(&room);
    let _ = state
        .agent
        .send_text(Target::Room { room: room.clone() }, &format!("{} joined the session", state.agent.name))
        .await;
    let body = digest_backlog(&msgs, backlog);
    // Roster as a count, not a full listing — the join stays cheap; parler_roster gives the details.
    let roster_line = match state.agent.roster(&room).await {
        Ok(entries) => format!("\n— {} agent(s) in the room —", entries.len()),
        Err(_) => String::new(),
    };
    Ok(format!(
        "joined session — room '{room}', now your active session.\n\
         --- context so far ---\n{body}\n--- end context ---{roster_line}"
    ))
}

/// How long `join_session` waits for a host approval before returning a "pending" message: a short
/// poll (bounded by these) so a quick approval resolves in the same call, but a human-paced one
/// doesn't block the joiner indefinitely.
const JOIN_POLL_ATTEMPTS: usize = 3;
const JOIN_POLL_INTERVAL_MS: u64 = 500;

/// If the caller owns `room` and agents are waiting to join it, render an approval prompt to append
/// to a `parler_send`/`parler_recv` result — this is how the host is *shown* the accept/reject option
/// inline, instead of having to poll for it. Returns `None` for a non-owner (the `join_requests` call
/// is refused) or when nothing is pending.
async fn pending_join_notice(state: &mut McpState, room: &str) -> Option<String> {
    let reqs = state.agent.join_requests(room).await.ok()?;
    if reqs.is_empty() {
        return None;
    }
    let lines = reqs
        .iter()
        .map(|r| {
            let role = r.role.as_deref().map(|x| format!(" ({x})")).unwrap_or_default();
            format!("  • {}{role} [{}]", r.name, r.agent)
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "\n\n⏳ {n} agent(s) asking to JOIN this session — your approval is required before they can \
         see the conversation:\n{lines}\n\
         Ask the user, then approve with parler_approve_join (agent=<id>) or reject with \
         parler_deny_join (agent=<id>).",
        n = reqs.len(),
    ))
}

/// Build the "HANDOFF TO YOU" banner if any of `msgs` carries a [`HandoffRef`] addressed to this
/// agent (by name/role, or unaddressed). This is the nudge that turns a passive `parler_recv` into
/// autonomous continuation: the host agent sees an explicit instruction to act on now, not just a
/// transcript line it might skim past. Returns `None` when no incoming handoff is for us.
fn handoff_banner(state: &McpState, msgs: &[&StoredMessage]) -> Option<String> {
    let me = &state.agent;
    let mut items = Vec::new();
    for m in msgs {
        // Don't act on our own handoff echoed back to us.
        if m.from.id == me.id {
            continue;
        }
        for part in &m.parts {
            if let Some(h) = HandoffRef::from_part(part) {
                if h.is_for(&me.name, me.role.as_deref()) {
                    let mut line = format!("  • {}", h.next);
                    if let Some(s) = &h.summary {
                        line.push_str(&format!("\n    (context: {s})"));
                    }
                    if let Some(blob) = &h.bundle {
                        line.push_str(&format!("\n    (attached code: apply via parler_apply blob={blob})"));
                    }
                    line.push_str(&format!("\n    — from {}", m.from.name));
                    items.push(line);
                }
            }
        }
    }
    if items.is_empty() {
        return None;
    }
    Some(format!(
        "🤝 HANDOFF TO YOU — another agent handed you the turn. Act on this now:\n{}",
        items.join("\n")
    ))
}

/// Leave the active session: announce departure, go idle, and forget the session locally. The room
/// itself stays alive for the others; hub-side cleanup happens via the idle timeout / disconnect.
async fn close_session(state: &mut McpState) -> Result<String> {
    let Some(room) = state.active_session.take() else {
        return Ok("no active session to close".into());
    };
    let _ = crate::clear_active_session();
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
            "Open a shared live session; returns a KEY to hand another agent so it joins your conversation already caught up. `context` is posted as the first message — recap the task, decisions, files, state. Joiners need YOUR approval by default (you're shown an accept/reject prompt; confirm with the user). Becomes your active session (parler_send/parler_recv then need no room). Keep a durable recap current with parler_remember key=\"session-digest\" so late joiners get it cheaply.",
            json!({
                "context": { "type": "string", "description": "summary of the conversation/state used to catch up whoever joins" },
                "topic": { "type": "string", "description": "optional short name for the session" },
                "approval": { "type": "boolean", "description": "require your approval before a joiner is admitted (default true). Set false for an open paste-and-join key." },
                "ttl_secs": { "type": "integer", "description": "how long the key stays valid (default 24h)" },
                "max_uses": { "type": "integer", "description": "how many agents may join with the key (default 50)" }
            }),
            &[],
        ),
        tool(
            "parler_join_session",
            "Join a session with a KEY. If approval is required you're held pending until the host admits you (retry to check). Once in, you get a digest of the context (seed + recent tail); backlog:\"full\" replays everything, or parler_recv since=<seq> re-reads a range in full. Becomes your active session (parler_send/parler_recv need no room).",
            json!({
                "key": { "type": "string", "description": "the session key or link you were handed" },
                "backlog": { "type": "string", "enum": ["recent", "full"], "description": "recent (default): seed + recent tail; full: replay the entire backlog" }
            }),
            &["key"],
        ),
        tool(
            "parler_close_session",
            "Leave your active session (announces departure, goes idle). The session stays alive for the others.",
            json!({}),
            &[],
        ),
        tool(
            "parler_join_requests",
            "List agents waiting for your approval to join a session you opened (defaults to active session). Each line carries the joiner's id for parler_approve_join / parler_deny_join.",
            json!({ "room": { "type": "string", "description": "the session room (defaults to your active session)" } }),
            &[],
        ),
        tool(
            "parler_approve_join",
            "Approve a pending joiner (from parler_join_requests or the send/recv prompt) — they can then read and participate. Defaults to active session. Confirm with the user before approving.",
            json!({
                "agent": { "type": "string", "description": "the id of the joiner to admit" },
                "room": { "type": "string", "description": "the session room (defaults to your active session)" }
            }),
            &["agent"],
        ),
        tool(
            "parler_deny_join",
            "Reject a pending joiner — turned away, can't re-request. Pass the joiner's id. Defaults to active session.",
            json!({
                "agent": { "type": "string", "description": "the id of the joiner to reject" },
                "room": { "type": "string", "description": "the session room (defaults to your active session)" }
            }),
            &["agent"],
        ),
        tool(
            "parler_watch_session",
            "Mint a read-only WATCH code so the user can watch this session live from the Parler website (/session page). Owner-only and separate from the join key (which can't read the backlog), so it's the safe way to let a human view it. Defaults to active session; hand the code to the user.",
            json!({
                "room": { "type": "string", "description": "the session room (defaults to your active session)" },
                "ttl_secs": { "type": "integer", "description": "how long the watch code stays valid (default 1h)" }
            }),
            &[],
        ),
        tool(
            "parler_invite",
            "Mint an invite code/link for another agent. kind: dm (1:1, default), group (1:many channel), service (many:1 queue). Hand the code to the other agent.",
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
            "Send a message and get back replies already waiting in the room (read-after-write). Defaults to your active session; else give exactly one of room (channel), to (peer agent id or name, DM), service (queue). For a reply not landed yet, don't poll — parler_recv wait_secs long-polls for it.",
            json!({
                "room": { "type": "string" },
                "to": { "type": "string", "description": "a peer agent id or a directory name (resolved to a unique id)" },
                "service": { "type": "string" },
                "text": { "type": "string" }
            }),
            &["text"],
        ),
        tool(
            "parler_recv",
            "Pull new messages since your cursor (advances it). Defaults to active session; pass room for another. wait_secs long-polls that many seconds for a pushed reply instead of returning empty (cheaper than repeated calls). Long bodies are truncated with a refetch hint; since+limit re-reads that range in FULL (never truncated). Default batch is bounded — a 'more waiting' line means call again.",
            json!({
                "room": { "type": "string" },
                "since": { "type": "integer", "description": "re-read from this seq in full (no cursor advance, no truncation)" },
                "limit": { "type": "integer", "description": "max messages this call (default bounded; raise to read more at once)" },
                "wait_secs": { "type": "integer", "description": "block up to this many seconds for a pushed message when nothing is waiting (sub-second wake; max 60)" }
            }),
            &[],
        ),
        tool(
            "parler_handoff",
            "Hand the turn to another agent: posts a 'HANDOFF TO YOU' banner they see on their next parler_recv (instant if long-polling with wait_secs), so they continue without a human re-prompting. Use it when you finish your part. Defaults to active session; or target room/to/service. `for`: address by agent name or role (omit = anyone). `bundle`: attach a code blob id from parler_push.",
            json!({
                "next": { "type": "string", "description": "what the next agent should do — the instruction to act on" },
                "summary": { "type": "string", "description": "recap of what you just finished / current state, for the next agent's context" },
                "for": { "type": "string", "description": "address the handoff to a specific agent by name or role (default: anyone in the room)" },
                "bundle": { "type": "string", "description": "optional blob id of a code bundle handed off alongside (from parler_push)" },
                "room": { "type": "string" },
                "to": { "type": "string" },
                "service": { "type": "string" }
            }),
            &["next"],
        ),
        tool(
            "parler_remember",
            "Save a durable fact to shared memory instead of re-reading history. With a key, re-saving overwrites (idempotent) — e.g. key=\"session-digest\" room=<room> keeps a rolling recap late joiners get cheaply. Optionally scope to a room; pass an embedding for hybrid semantic recall.",
            json!({
                "text": { "type": "string" },
                "key": { "type": "string" },
                "room": { "type": "string" },
                "embedding": { "type": "array", "items": { "type": "number" }, "description": "embedding vector (float32 array, must match hub dimension)" },
                "embedding_model": { "type": "string", "description": "which model produced the embedding (e.g. text-embedding-3-small)" }
            }),
            &["text"],
        ),
        tool(
            "parler_recall",
            "Recall saved facts (BM25 full-text; hybrid BM25 + vector KNN when an embedding is given). Cheaper than re-reading history for durable state you saved with parler_remember.",
            json!({
                "query": { "type": "string" },
                "room": { "type": "string" },
                "limit": { "type": "integer" },
                "embedding": { "type": "array", "items": { "type": "number" }, "description": "query embedding vector for semantic recall" }
            }),
            &["query"],
        ),
        tool(
            "parler_push",
            "Hand off code: build a git bundle from the repo and push it to a room/peer/service (exactly one). With base, bundle only base..gitref (a thin patch series). The peer applies it with `parler apply <blob>`.",
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
            "Download a pushed bundle's bytes by blob id (from a com.parler.bundle message) to a file. Does NOT apply — verify/fetch with git yourself.",
            json!({
                "id": { "type": "string" },
                "out": { "type": "string", "description": "output file (default: <blob>.bundle)" }
            }),
            &["id"],
        ),
        tool("parler_rooms", "List the rooms you belong to, with unread counts.", json!({}), &[]),
        tool(
            "parler_roster",
            "List who is in a room (name (role) [status]). detail:true also shows each agent id.",
            json!({
                "room": { "type": "string" },
                "detail": { "type": "boolean", "description": "include agent ids (default false)" }
            }),
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
            "Publish your discovery card. visibility: private (default, same-hub) or public (anyone). Signed with your key, so it's tamper-evident.",
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
            "Discover agents. scope: hub (default) or public. Filter by query/tag/skill/status. Results are compact (no ids) and capped by default — you can parler_send to=<name> / parler_card <name> directly; pass detail:true for ids or raise limit for more.",
            json!({
                "scope": { "type": "string", "enum": ["hub", "public"] },
                "query": { "type": "string" },
                "tag": { "type": "string" },
                "skill": { "type": "string" },
                "status": { "type": "string" },
                "limit": { "type": "integer" },
                "detail": { "type": "boolean", "description": "include agent ids (default false)" }
            }),
            &[],
        ),
        tool(
            "parler_card",
            "Fetch a single agent's directory card (JSON with signature verification). id can be a full agent id or a directory name (resolved to a unique id).",
            json!({ "id": { "type": "string", "description": "a full agent id or a directory name" } }),
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

    // ---- Token-budget harness (P0.1) -------------------------------------------------------------
    //
    // Every MCP tool result / spec lands verbatim in each participating agent's LLM context, so the
    // *rendered char count* is the thing we're optimizing. These budgets are ceilings the render code
    // must stay under; each is tightened by the item that produces its saving (P0.2 → specs, P0.3 →
    // join, P0.4 → send). Tests print the measured number so a run's output is the savings receipt.
    // Fixed-size synthetic messages keep the counts deterministic; consts carry ~20% headroom.

    /// Serialized `tools/list` payload size — permanent context cost, paid by every agent every
    /// session. Pre-diet baseline 11,598 B; post-diet (P0.2) 11,030 B. Ceiling with ~5% headroom.
    const TOOL_SPECS_BUDGET: usize = 11_600;
    /// Just the human-readable descriptions (the part the diet targets; schema scaffolding is
    /// load-bearing). Pre-diet 5,261 B → post-diet (P0.2) 4,304 B; P1.2 adds ~230 B of cheap-path
    /// steering (name-based `to`/`card`, compact discover/roster, `detail`) that earns its bytes.
    /// Still ~730 B under the pre-diet baseline. Ceiling with headroom.
    const TOOL_DESC_BUDGET: usize = 4_700;
    /// Rendered `join_session` output with a ~100-message backlog. Full-replay baseline was 7,863
    /// chars; P0.3's digest render (seed + tail + omission line) brings it to ~1,458. Ceiling leaves
    /// headroom for a larger tail / longer messages.
    const JOIN_RENDER_BUDGET: usize = 3_000;
    /// Rendered `parler_send` output when ~20 replies are already waiting (auto-pull). Uncapped
    /// baseline was 1,657 chars; P0.4's AUTOPULL_LIMIT=10 cap brings it to ~740. Ceiling leaves
    /// headroom for longer bodies (each capped at MSG_MAX_CHARS).
    const SEND_RENDER_BUDGET: usize = 2_000;

    /// A body of exactly `len` ASCII chars (deterministic sizing for budget assertions).
    fn body_of(len: usize) -> String {
        "x".repeat(len)
    }

    /// Post `n` fixed-size messages (~60 chars each) into `room` from `poster` (which must already be
    /// a member), so budget assertions don't depend on message content. Returns after all are sent.
    async fn seed_room(poster: &mut McpState, room: &str, n: usize) {
        for i in 0..n {
            poster
                .agent
                .send_text(Target::Room { room: room.to_string() }, &format!("m{i} {}", body_of(60)))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn tool_specs_stay_lean() {
        let specs = tool_specs();
        let serialized = serde_json::to_string(&json!({ "tools": specs })).unwrap();
        let bytes = serialized.len();
        let desc_bytes: usize = specs
            .iter()
            .filter_map(|t| t.get("description").and_then(Value::as_str))
            .map(str::len)
            .sum();
        println!(
            "[budget] tool_specs: {} tools, {bytes} B serialized ({desc_bytes} B of descriptions)",
            specs.len()
        );
        assert!(
            bytes <= TOOL_SPECS_BUDGET,
            "tool specs {bytes} B exceed budget {TOOL_SPECS_BUDGET} B — trim descriptions"
        );
        assert!(
            desc_bytes <= TOOL_DESC_BUDGET,
            "tool descriptions {desc_bytes} B exceed budget {TOOL_DESC_BUDGET} B — keep them tight"
        );
    }

    #[tokio::test]
    async fn join_with_backlog_renders_under_budget() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        // Open with a real context seed, then seed ~100 more messages into the room.
        let opened = open_session(&mut alice, Some("catch-up context for the budget test"), Some("budget".into()), None, None, false)
            .await
            .unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);
        seed_room(&mut alice, &room, 100).await;

        let mut bob = state(&hub, "bob").await;
        let joined = join_session(&mut bob, &key, Backlog::Recent).await.unwrap();
        let chars = joined.chars().count();
        println!("[budget] join_session with ~100-msg backlog: {chars} chars rendered");
        assert!(
            joined.len() <= JOIN_RENDER_BUDGET,
            "join render {} B exceeds budget {JOIN_RENDER_BUDGET} B",
            joined.len()
        );
    }

    #[tokio::test]
    async fn join_digests_long_backlog() {
        // A late joiner gets a *digest*: the seed in full, an omission line naming the re-read seq,
        // and the last JOIN_TAIL messages — not the whole replay.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("the plan: ship auth by friday"), Some("plan".into()), None, None, false)
            .await
            .unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);
        // Seed enough that the middle is omitted (well over JOIN_TAIL). Give the last one a marker we
        // can assert appears (it's inside the tail window).
        seed_room(&mut alice, &room, 40).await;
        alice.agent.send_text(Target::Room { room: room.clone() }, "LAST_TAIL_MESSAGE").await.unwrap();

        let mut bob = state(&hub, "bob").await;
        let joined = join_session(&mut bob, &key, Backlog::Recent).await.unwrap();

        // Seed is always rendered in full (the host's recap).
        assert!(joined.contains("the plan: ship auth by friday"), "seed present in digest:\n{joined}");
        // The middle is summarized with a re-read pointer, not replayed.
        assert!(joined.contains("earlier message(s) omitted"), "omission line present:\n{joined}");
        assert!(joined.contains("parler_recv since="), "omission line names the re-read seq:\n{joined}");
        // The live tail is present in full.
        assert!(joined.contains("LAST_TAIL_MESSAGE"), "recent tail present:\n{joined}");
        // A middle message (m5) is NOT replayed (it's in the omitted range).
        assert!(!joined.contains("m5 "), "an omitted middle message must not be replayed:\n{joined}");
        // Roster is a count, not a full listing.
        assert!(joined.contains("agent(s) in the room"), "roster rendered as a count:\n{joined}");
    }

    #[tokio::test]
    async fn join_full_mode_renders_entire_backlog() {
        // The escape hatch: backlog:"full" replays every message (no digest, no omission line).
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), Some("plan".into()), None, None, false).await.unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);
        seed_room(&mut alice, &room, 40).await;

        let mut bob = state(&hub, "bob").await;
        let joined = join_session(&mut bob, &key, Backlog::Full).await.unwrap();

        // Full mode replays even a mid-backlog message and never emits the omission line.
        assert!(joined.contains("m5 "), "full mode replays the middle:\n{joined}");
        assert!(joined.contains("m30 "), "full mode replays late-middle messages too");
        assert!(!joined.contains("earlier message(s) omitted"), "full mode has no omission line");
    }

    #[tokio::test]
    async fn send_with_waiting_replies_renders_under_budget() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), Some("budget".into()), None, None, false).await.unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);

        // Bob joins and posts ~20 fixed-size replies that are waiting when alice next sends.
        let mut bob = state(&hub, "bob").await;
        join_session(&mut bob, &key, Backlog::Recent).await.unwrap();
        for i in 0..20 {
            bob.agent
                .send_text(Target::Room { room: room.clone() }, &format!("reply {i} {}", body_of(60)))
                .await
                .unwrap();
        }

        let sent = call_session_tool(&mut alice, "parler_send", &json!({ "text": "status?" })).await.unwrap();
        let chars = sent.chars().count();
        println!("[budget] parler_send with ~20 waiting replies: {chars} chars rendered");
        assert!(
            sent.len() <= SEND_RENDER_BUDGET,
            "send render {} B exceeds budget {SEND_RENDER_BUDGET} B",
            sent.len()
        );
    }

    #[tokio::test]
    async fn recv_caps_batch_but_drains_losslessly() {
        // A default recv is bounded (RECV_DEFAULT_LIMIT) and hints "more waiting"; a second recv
        // returns the remainder — no message is lost, because a limited pull advances the cursor
        // only through what it returned.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);
        let mut bob = state(&hub, "bob").await;
        join_session(&mut bob, &key, Backlog::Recent).await.unwrap();
        // Drain bob's own "joined" announce so his inbox starts empty relative to what alice posts.
        call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();

        // Alice posts 40 uniquely-tagged messages while bob isn't looking.
        for i in 0..40 {
            alice.agent.send_text(Target::Room { room: room.clone() }, &format!("u{i}_msg")).await.unwrap();
        }

        let first = call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();
        assert!(first.contains("more waiting"), "a full batch hints there's more:\n{first}");
        let second = call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();

        // Every one of the 40 messages appears across the two calls (lossless drain).
        for i in 0..40 {
            let tag = format!("u{i}_msg");
            assert!(
                first.contains(&tag) || second.contains(&tag),
                "message {tag} lost across the capped drain"
            );
        }
        // The second call cleared the backlog — a third recv is empty.
        let third = call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();
        assert!(third.contains("no new messages"), "backlog fully drained:\n{third}");
    }

    #[tokio::test]
    async fn autopull_hints_more_when_replies_overflow_the_cap() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);
        let mut bob = state(&hub, "bob").await;
        join_session(&mut bob, &key, Backlog::Recent).await.unwrap();

        // Bob leaves more than AUTOPULL_LIMIT replies waiting before alice sends.
        for i in 0..15 {
            bob.agent.send_text(Target::Room { room: room.clone() }, &format!("r{i}")).await.unwrap();
        }
        let sent = call_session_tool(&mut alice, "parler_send", &json!({ "text": "?" })).await.unwrap();
        assert!(sent.contains("more waiting"), "auto-pull hints there's more past the cap:\n{sent}");
    }

    #[tokio::test]
    async fn long_body_is_truncated_then_refetchable_in_full() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);
        let mut bob = state(&hub, "bob").await;
        join_session(&mut bob, &key, Backlog::Recent).await.unwrap();
        call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap(); // drain own announce

        // Alice posts a body well over MSG_MAX_CHARS.
        let long = "A".repeat(MSG_MAX_CHARS + 500);
        alice.agent.send_text(Target::Room { room: room.clone() }, &long).await.unwrap();

        // Bob's cursor-mode recv truncates it with a refetch pointer.
        let recv = call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();
        assert!(recv.contains("chars — parler_recv since="), "long body truncated with a hint:\n{}", &recv[..recv.len().min(200)]);
        assert!(recv.len() < long.len(), "truncated render is shorter than the full body");

        // Extract the seq the hint points at and re-read that one message: it comes back in full.
        let seq: i64 = recv
            .split("since=")
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse().ok())
            .expect("a since=<seq> in the truncation hint");
        let full = call_session_tool(&mut bob, "parler_recv", &json!({ "since": seq, "limit": 1 })).await.unwrap();
        assert!(full.contains(&long), "explicit since re-read returns the full body");
        assert!(!full.contains("chars — parler_recv since="), "re-reads are never truncated");
    }

    #[test]
    fn recv_limit_decides_the_cap() {
        // Explicit limit always wins (even in re-read / verbose).
        assert_eq!(recv_limit(Some(5), false, false), Some(5));
        assert_eq!(recv_limit(Some(5), true, true), Some(5));
        // Plain cursor read → the default cap.
        assert_eq!(recv_limit(None, false, false), Some(RECV_DEFAULT_LIMIT));
        // A history re-read (`since`) is uncapped (full detail).
        assert_eq!(recv_limit(None, true, false), None);
        // Verbose is the global escape hatch → uncapped.
        assert_eq!(recv_limit(None, false, true), None);
    }

    #[test]
    fn budgeted_render_truncates_only_over_the_cap() {
        // A short message renders identically to the plain render (no hint appended).
        let short = StoredMessage {
            seq: 7,
            id: "i".into(),
            room: "r".into(),
            from: parler_protocol::EndpointRef { id: "a".into(), name: "alice".into(), role: None },
            parts: vec![parler_protocol::Part::text("hi")],
            mentions: None,
            reply_to: None,
            ts: 0,
        };
        assert_eq!(render_message_budgeted(&short), crate::render_message(&short));
        assert!(!render_message_budgeted(&short).contains("parler_recv since="));

        // A long message is truncated with a pointer to re-read it in full at seq-1.
        let long = StoredMessage {
            parts: vec![parler_protocol::Part::text("z".repeat(MSG_MAX_CHARS + 100))],
            ..short.clone()
        };
        let out = render_message_budgeted(&long);
        assert!(out.chars().count() < MSG_MAX_CHARS + 200, "truncated to ~the cap");
        assert!(out.contains("parler_recv since=6 limit=1 for full"), "hint points at seq-1:\n{out}");
    }

    #[tokio::test]
    async fn open_then_join_shares_context_and_sets_active_session() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let opened = open_session(&mut alice, Some("designing the auth flow; see src/auth.rs"), Some("design".into()), None, None, false)
            .await
            .unwrap();
        assert!(opened.contains("KEY: "));
        // The output carries a ready-to-paste teammate one-liner with the session key preset.
        assert!(opened.contains("claude mcp add parler"), "shareable one-liner present:\n{opened}");
        assert!(opened.contains("PARLER_SESSION_KEY="), "one-liner presets the session key");
        assert!(alice.active_session.is_some());

        let key = key_of(&opened);
        let joined = join_session(&mut bob, &key, Backlog::Recent).await.unwrap();
        assert!(joined.contains("designing the auth flow"), "joiner should receive the seeded context");
        assert_eq!(bob.active_session, alice.active_session, "both share the same session room");
    }

    #[tokio::test]
    async fn open_without_context_posts_no_seed() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let opened = open_session(&mut alice, None, Some("empty".into()), None, None, false).await.unwrap();
        let key = key_of(&opened);
        // Bob joins; the only backlog should be his own "joined" announce — no seed context line.
        let joined = join_session(&mut bob, &key, Backlog::Recent).await.unwrap();
        assert!(!joined.contains("session context"), "no seed message when context is omitted");
    }

    #[tokio::test]
    async fn send_defaults_to_active_session_and_autopull_filters_own() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let opened = open_session(&mut alice, Some("seed"), Some("design".into()), None, None, false).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key, Backlog::Recent).await.unwrap();

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

        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key, Backlog::Recent).await.unwrap(); // advances bob's cursor to the live edge

        // Alice posts after bob is caught up; bob's recv (no room) picks it up from the active session.
        call_session_tool(&mut alice, "parler_send", &json!({ "text": "ping bob" })).await.unwrap();
        let recv = call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();
        assert!(recv.contains("ping bob"));
    }

    #[tokio::test]
    async fn handoff_addressed_to_recipient_shows_banner_only_for_them() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;
        let mut carol = state(&hub, "carol").await;

        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key, Backlog::Recent).await.unwrap();
        join_session(&mut carol, &key, Backlog::Recent).await.unwrap();
        // Drain each joiner's own "joined" announce so their cursors start clean.
        call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();
        call_session_tool(&mut carol, "parler_recv", &json!({})).await.unwrap();

        // Alice hands the turn explicitly to bob.
        let sent = call_session_tool(
            &mut alice,
            "parler_handoff",
            &json!({ "next": "build the page structure", "summary": "design locked", "for": "bob" }),
        )
        .await
        .unwrap();
        assert!(sent.contains("handed off to bob"));

        // Bob sees the actionable banner + the instruction.
        let bob_recv = call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();
        assert!(bob_recv.contains("HANDOFF TO YOU"), "addressee gets the nudge:\n{bob_recv}");
        assert!(bob_recv.contains("build the page structure"));

        // Carol is in the same room but the handoff isn't for her — no banner.
        let carol_recv = call_session_tool(&mut carol, "parler_recv", &json!({})).await.unwrap();
        assert!(!carol_recv.contains("HANDOFF TO YOU"), "non-addressee must not be nudged:\n{carol_recv}");
        // She still sees the message itself (it's a normal room post), rendered as a handoff line.
        assert!(carol_recv.contains("🤝 handoff → bob"));
    }

    #[tokio::test]
    async fn unaddressed_handoff_nudges_anyone() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key, Backlog::Recent).await.unwrap();
        call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();

        call_session_tool(&mut alice, "parler_handoff", &json!({ "next": "take it from here" }))
            .await
            .unwrap();
        let bob_recv = call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();
        assert!(bob_recv.contains("HANDOFF TO YOU"), "an unaddressed handoff is for anyone:\n{bob_recv}");
    }

    #[tokio::test]
    async fn recv_wait_secs_long_polls_for_a_push() {
        // With nothing waiting, `parler_recv` + wait_secs blocks until a peer's message is pushed,
        // then returns it — sub-second, no polling. (state() subscribes for push.)
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;
        assert!(bob.push, "the hub should support push so recv can long-poll");

        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key, Backlog::Recent).await.unwrap(); // bob caught up to the live edge
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

        open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        assert!(alice.active_session.is_some());
        let closed = close_session(&mut alice).await.unwrap();
        assert!(closed.contains("left session"));
        assert!(alice.active_session.is_none());
        // Closing again is a no-op, not an error.
        assert!(close_session(&mut alice).await.unwrap().contains("no active session"));
    }

    #[tokio::test]
    async fn approval_session_gates_joiner_until_host_approves() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        // Alice opens an approval-gated session (the default for parler_open_session).
        let opened =
            open_session(&mut alice, Some("secret plan: ship friday"), Some("plan".into()), None, None, true)
                .await
                .unwrap();
        assert!(opened.contains("approve"), "the host is told joiners need approval: {opened}");
        let key = key_of(&opened);

        // Bob redeems → held pending; he is NOT caught up and must not see the context.
        let pending = join_session(&mut bob, &key, Backlog::Recent).await.unwrap();
        assert!(pending.contains("waiting for the host"), "joiner is gated: {pending}");
        assert!(!pending.contains("secret plan"), "a pending joiner must not receive the context");
        assert!(bob.active_session.is_none(), "a pending joiner has no active session yet");

        // The host is shown the request inline when it next acts in the session.
        let on_send =
            call_session_tool(&mut alice, "parler_send", &json!({ "text": "anyone there?" })).await.unwrap();
        assert!(on_send.contains("asking to JOIN"), "host is shown the approval prompt: {on_send}");
        assert!(on_send.contains(&bob.agent.id), "the prompt names the joiner's id");

        // Alice lists, then approves bob.
        let reqs = call_session_tool(&mut alice, "parler_join_requests", &json!({})).await.unwrap();
        assert!(reqs.contains(&bob.agent.id));
        let approved = call_session_tool(&mut alice, "parler_approve_join", &json!({ "agent": bob.agent.id }))
            .await
            .unwrap();
        assert!(approved.contains("approved"));

        // Now bob's join succeeds and he receives the context in the same call.
        let joined = join_session(&mut bob, &key, Backlog::Recent).await.unwrap();
        assert!(joined.contains("secret plan: ship friday"), "an approved joiner gets the context: {joined}");
        assert_eq!(bob.active_session, alice.active_session, "both now share the session room");
    }

    #[tokio::test]
    async fn denied_joiner_cannot_enter_or_reapply() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut eve = state(&hub, "eve").await;

        let opened = open_session(&mut alice, Some("seed"), None, None, None, true).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut eve, &key, Backlog::Recent).await.unwrap(); // pending

        let denied = call_session_tool(&mut alice, "parler_deny_join", &json!({ "agent": eve.agent.id }))
            .await
            .unwrap();
        assert!(denied.contains("denied"));

        // The denial is terminal — eve's retry errors instead of letting her in.
        assert!(join_session(&mut eve, &key, Backlog::Recent).await.is_err());
    }

    #[tokio::test]
    async fn auto_register_self_lists_so_a_connected_agent_is_discoverable() {
        // "Connected" must mean "discoverable": after auto_register, a peer's `parler_discover`
        // (hub scope) finds this agent even though it never explicitly called parler_register.
        let hub = start_hub().await;
        let mut worker = state(&hub, "worker").await;
        auto_register(&mut worker.agent).await;

        let mut peer = state(&hub, "peer").await;
        let found = call_tool(&mut peer.agent, "parler_discover", &json!({})).await.unwrap();
        assert!(found.contains("worker"), "auto-registered agent should be discoverable: {found}");
    }

    #[tokio::test]
    async fn discover_is_compact_by_default_and_detailed_on_request() {
        let hub = start_hub().await;
        let mut worker = state(&hub, "worker").await;
        auto_register(&mut worker.agent).await;
        let mut peer = state(&hub, "peer").await;

        // Default: compact line, name present, id absent.
        let compact = call_tool(&mut peer.agent, "parler_discover", &json!({})).await.unwrap();
        assert!(compact.contains("worker"), "name present in compact line:\n{compact}");
        assert!(!compact.contains(&worker.agent.id), "id omitted by default:\n{compact}");
        // detail:true restores the id.
        let detailed = call_tool(&mut peer.agent, "parler_discover", &json!({ "detail": true })).await.unwrap();
        assert!(detailed.contains(&worker.agent.id), "detail:true shows the id:\n{detailed}");
    }

    #[tokio::test]
    async fn roster_hides_ids_by_default() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);
        let mut bob = state(&hub, "bob").await;
        join_session(&mut bob, &key, Backlog::Recent).await.unwrap();

        let plain = call_tool(&mut alice.agent, "parler_roster", &json!({ "room": room.clone() })).await.unwrap();
        assert!(plain.contains("alice"), "names present:\n{plain}");
        assert!(!plain.contains(&alice.agent.id), "ids hidden by default:\n{plain}");
        let detailed = call_tool(&mut alice.agent, "parler_roster", &json!({ "room": room, "detail": true })).await.unwrap();
        assert!(detailed.contains(&alice.agent.id), "detail:true shows ids:\n{detailed}");
    }

    #[tokio::test]
    async fn mcp_send_resolves_name_to_id() {
        // parler_send to=<directory name> resolves to the unique agent id (reusing resolve_target),
        // so a caller never needs the 56-char id. An unknown name errors instead of guessing.
        let hub = start_hub().await;
        let mut worker = state(&hub, "worker").await;
        auto_register(&mut worker.agent).await; // gives worker a discoverable card
        let mut peer = state(&hub, "peer").await;

        let sent = call_session_tool(&mut peer, "parler_send", &json!({ "to": "worker", "text": "hi worker" }))
            .await
            .unwrap();
        assert!(sent.contains("sent to"), "name resolved and message sent:\n{sent}");

        // An unknown name is an error, not a wrong-agent guess.
        let err = call_session_tool(&mut peer, "parler_send", &json!({ "to": "nobody-here", "text": "x" })).await;
        assert!(err.is_err(), "an unresolvable name must error, never guess");
    }

    #[test]
    fn env_flag_and_list_parse_as_documented() {
        std::env::set_var("PARLER_T_FLAG", "1");
        assert!(env_flag("PARLER_T_FLAG"));
        std::env::set_var("PARLER_T_FLAG", "false");
        assert!(!env_flag("PARLER_T_FLAG"));
        std::env::set_var("PARLER_T_FLAG", "");
        assert!(!env_flag("PARLER_T_FLAG"));
        std::env::remove_var("PARLER_T_FLAG");
        assert!(!env_flag("PARLER_T_FLAG"));

        std::env::set_var("PARLER_T_LIST", "coding, rust ,, ops");
        assert_eq!(env_list("PARLER_T_LIST"), vec!["coding", "rust", "ops"]);
        std::env::remove_var("PARLER_T_LIST");
        assert!(env_list("PARLER_T_LIST").is_empty());
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

    #[tokio::test]
    async fn session_handoff_prompt_digests_not_replays() {
        // The parler_session_handoff prompt returns a digest (seed + tail + omission line + a roster
        // count + a since/recall pointer), not the whole backlog replayed.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        open_session(&mut alice, Some("the handoff plan: finish auth"), Some("plan".into()), None, None, false)
            .await
            .unwrap();
        let room = alice.active_session.clone().unwrap();
        seed_room(&mut alice, &room, 40).await;

        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"prompts/get\",\"params\":{\"name\":\"parler_session_handoff\"}}\n";
        let mut output: Vec<u8> = Vec::new();
        run(&mut alice, BufReader::new(input.as_bytes()), &mut output).await.unwrap();
        let out = String::from_utf8(output).unwrap();

        assert!(out.contains("the handoff plan: finish auth"), "seed present in the prompt digest:\n{out}");
        assert!(out.contains("earlier message(s) omitted"), "middle summarized, not replayed:\n{out}");
        assert!(out.contains("agent(s) in the room"), "roster rendered as a count");
        assert!(out.contains("parler_recv since="), "prompt points at the full-detail re-read");
        // A mid-backlog message is NOT replayed in the prompt (JSON-escaped, so match the tag).
        assert!(!out.contains("m5 "), "an omitted middle message must not be in the prompt");
    }

    #[test]
    fn breadcrumb_log_roundtrips_and_trims() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("parler-mcplog-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp.log");

        // Write more than the cap; only the newest LOG_KEEP survive.
        for i in 0..(LOG_KEEP + 5) {
            log_event_at(&path, 1_000 + i as u64, &format!("event {i}"));
        }
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), LOG_KEEP, "log is trimmed to the cap");

        // `recent_log_at` returns the newest few, oldest-first, with a relative age vs. "now".
        let last_ts = 1_000 + (LOG_KEEP + 4) as u64; // ts of the final event written
        let now = last_ts + 7;
        let recent = recent_log_at(&path, now, 3).unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent.last().unwrap().1, format!("event {}", LOG_KEEP + 4));
        assert_eq!(recent.last().unwrap().0, "7s"); // newest entry is 7s old
        // Newlines in a message can't corrupt the line-oriented format.
        log_event_at(&path, now, "line1\nline2");
        let last = recent_log_at(&path, now, 1).unwrap();
        assert_eq!(last[0].1, "line1 line2");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
