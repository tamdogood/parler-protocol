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
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// The always-on, world-readable hub a fresh agent joins by default (override with `PARLER_HUB`).
const DEFAULT_PUBLIC_HUB: &str = "wss://parler-hub.fly.dev";

/// Connect to the hub, then serve the MCP JSON-RPC loop on stdin/stdout until EOF.
pub async fn serve_stdio() -> Result<()> {
    let cfg = load_or_bootstrap_config()?;
    let mut agent = MeshAgent::connect(&cfg).await?;

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

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

        let result = handle(&mut agent, method, params).await;

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
        stdout.write_all(s.as_bytes()).await?;
        stdout.flush().await?;
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
async fn handle(agent: &mut MeshAgent, method: &str, params: Value) -> Result<Value, (i64, String)> {
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
            // Per MCP, a tool's own failure is a result with isError=true, not a protocol error.
            match call_tool(agent, &name, &args).await {
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
        "parler_send" => {
            let text = s("text").ok_or_else(|| anyhow!("missing 'text'"))?;
            let target = if let Some(r) = s("room") {
                Target::Room { room: r }
            } else if let Some(t) = s("to") {
                Target::Dm { agent: t }
            } else if let Some(sv) = s("service") {
                Target::Service { service: sv }
            } else {
                bail!("provide exactly one of room / to / service");
            };
            let (_id, seq, room) = agent.send_text(target, &text).await?;
            Ok(format!("sent to '{room}' (seq {seq})"))
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
        "parler_recv" => {
            let room = s("room").ok_or_else(|| anyhow!("missing 'room'"))?;
            let since = args.get("since").and_then(Value::as_i64);
            let (msgs, cursor) = agent.pull(&room, since, u32opt("limit")).await?;
            if msgs.is_empty() {
                return Ok(format!("(no new messages in '{room}')"));
            }
            let body = msgs.iter().map(crate::render_message).collect::<Vec<_>>().join("\n");
            Ok(format!("{body}\n— cursor at {cursor} —"))
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
            "Send a message. Provide exactly one of: room (1:many channel), to (a peer agent id, 1:1 DM), or service (many:1 queue).",
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
            "Pull new messages for a room since your cursor (which it advances). Use since/limit to re-read history.",
            json!({
                "room": { "type": "string" },
                "since": { "type": "integer" },
                "limit": { "type": "integer" }
            }),
            &["room"],
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
