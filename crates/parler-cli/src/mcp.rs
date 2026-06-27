//! `parler mcp` — a minimal MCP (Model Context Protocol) server over stdio.
//!
//! MCP is JSON-RPC 2.0 with newline-delimited messages on stdio. We implement just the methods a
//! host needs — `initialize`, `tools/list`, `tools/call`, `ping` — and map each `parler_*` tool
//! onto the same [`MeshAgent`] the CLI uses. Hand-rolled on purpose: it keeps the dependency
//! surface tiny and gives exact control over the wire, which matters more than an SDK here.

use anyhow::{anyhow, bail, Result};
use parler_connector::{Config, MeshAgent};
use parler_protocol::{RoomKind, Target};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Connect to the hub, then serve the MCP JSON-RPC loop on stdin/stdout until EOF.
pub async fn serve_stdio() -> Result<()> {
    let cfg = Config::load()?;
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
        other => bail!("unknown tool: {other}"),
    }
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
    ]
}
