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
use std::time::{Duration, Instant};
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
}

/// Connect to the hub, then serve the MCP JSON-RPC loop on stdin/stdout until EOF.
pub async fn serve_stdio() -> Result<()> {
    let cfg = match load_or_bootstrap_config() {
        Ok(c) => c,
        Err(e) => {
            log_event(&format!("bootstrap FAILED: {e} (run `parler doctor` to troubleshoot)"));
            return Err(e);
        }
    };
    let mut agent = match connect_with_retry(&cfg).await {
        Ok(a) => a,
        Err(e) => {
            // Leave a breadcrumb before we exit: a GUI host swallows stderr, so `parler doctor`
            // reading this log is often the only way a user learns *why* the agent went dark.
            log_event(&format!("connect FAILED → {}: {e} (run `parler doctor` to troubleshoot)", cfg.hub_url));
            return Err(e);
        }
    };
    log_event(&format!("connected as {} ({}) → {}", cfg.name, cfg.identity.id, cfg.hub_url));
    // Opt into sub-second push as a *latency optimization* (best-effort; a no-op against an older
    // hub). It's no longer load-bearing for long-poll — `parler_recv wait_secs` uses the hub's
    // server-side wait, which works with zero push machinery — so a failed subscribe here just means
    // we lean on the server-side wait, not that long-poll is unavailable. The live subscription state
    // lives in the connector (queried via `push_active()`), not cached here.
    let _ = agent.subscribe().await;
    // Self-list on the hub the moment we connect, so a freshly wired agent is visible to same-hub
    // peers (and shows up under the desktop app's Agents) without a human having to call
    // `parler_register` first — "connected" should mean "discoverable". Private by default
    // (same-hub only); opt into the public directory or enrich the card via env. Best-effort.
    auto_register(&mut agent).await;
    let mut state = McpState { agent, active_session: None };

    // Spin-up convenience: if a session key was handed in via the environment, join it now so a
    // freshly launched agent is already in the shared conversation (with its context) before the
    // host makes a single tool call. Failures are non-fatal — log to stderr (stdout is the
    // protocol channel) and carry on.
    if let Some(key) = std::env::var("PARLER_SESSION_KEY").ok().filter(|s| !s.is_empty()) {
        // Auto-join is a spin-up convenience, not an interactive call — don't block boot on a human
        // approval; a pending join returns its "waiting" message and the agent proceeds.
        match join_session(&mut state, &key, Backlog::Recent, None).await {
            Ok(msg) => {
                eprintln!("parler: {msg}");
                log_event(&format!("session join SUCCESS ({key}): {msg}"));
            }
            Err(e) => {
                eprintln!("parler: PARLER_SESSION_KEY join failed: {e}");
                log_event(&format!("session join FAILED ({key}): {e} (run `parler doctor` to troubleshoot)"));
            }
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

/// How long `parler mcp` keeps retrying a down hub at startup before giving up. A host often
/// launches the agent the instant the user opens the editor — possibly *before* their `--local`
/// hub is up — so dying on the first refused connection would leave a dead MCP server with no
/// visible cause (issue #102). Retrying for a short window lets the agent ride out a hub that's
/// still coming up (or that the user starts a few seconds later).
const CONNECT_RETRY_WINDOW: Duration = Duration::from_secs(30);
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Connect to the hub, retrying a *reachability* failure for [`CONNECT_RETRY_WINDOW`] instead of
/// dying instantly. An auth failure (bad/absent join secret) is returned immediately — retrying
/// can't fix it and would only delay the breadcrumb. Each retry leaves a log line so `parler
/// doctor` can show the agent was waiting on the hub, and names the exact start command.
async fn connect_with_retry(cfg: &Config) -> Result<MeshAgent> {
    let start = std::time::Instant::now();
    let mut announced = false;
    loop {
        match MeshAgent::connect(cfg).await {
            Ok(a) => return Ok(a),
            Err(e) => {
                // A join-secret / auth rejection won't heal by waiting — surface it now.
                let msg = e.to_string();
                let is_auth = msg.contains("authentication failed") || msg.contains("join secret");
                if is_auth || start.elapsed() >= CONNECT_RETRY_WINDOW {
                    return Err(e);
                }
                if !announced {
                    announced = true;
                    let hint = start_hub_hint(&cfg.hub_url);
                    eprintln!("parler: hub {} not reachable yet — retrying for {}s. {hint}", cfg.hub_url, CONNECT_RETRY_WINDOW.as_secs());
                    log_event(&format!("hub {} down at startup — retrying up to {}s ({hint})", cfg.hub_url, CONNECT_RETRY_WINDOW.as_secs()));
                }
                tokio::time::sleep(CONNECT_RETRY_INTERVAL).await;
            }
        }
    }
}

/// The one command that starts the hub an agent is waiting on — a loopback URL means `parler hub
/// --local`; anything else is a remote hub the user must bring up (or already have running).
pub(crate) fn start_hub_hint(hub_url: &str) -> String {
    if hub_url.contains("127.0.0.1") || hub_url.contains("localhost") {
        "Start it with:  parler hub --local".to_string()
    } else {
        "Start/reach the hub, then it will connect automatically.".to_string()
    }
}

/// Load the saved identity, or — for zero-setup onboarding — mint one on first launch, then apply
/// the one env/config precedence rule identically for the CLI and the MCP server.
///
/// A new user shouldn't have to run `parler init` before wiring up the MCP server: the first time
/// an MCP host starts `parler mcp`, we create an Ed25519 identity pointed at the public hub and
/// persist it to `PARLER_HOME`, so the agent's id stays stable across restarts. Override any of the
/// defaults with env vars in the MCP server config:
///   - `PARLER_HUB`  — hub to dial (default: the public hub; use `ws://host:port` for a private one)
///   - `PARLER_NAME` — display name (default: `$USER`, else `agent`)
///   - `PARLER_ROLE` — role advertised on the card (planner, reviewer, …)
///
/// The precedence is **explicit env var > saved config > default**, matching how
/// `PARLER_JOIN_SECRET` is already read live from the environment on every connect. This is the
/// single source of truth: both `parler mcp` and the CLI's [`crate::connect`]-time agent resolve
/// their hub/name/role through here, so a re-run of `parler connect` that rewrites the env block
/// genuinely moves/renames the agent on next launch instead of being silently ignored.
pub(crate) fn load_or_bootstrap_config() -> Result<Config> {
    if Config::exists() {
        // A saved identity keeps its stable id + seed, but its hub/name/role follow the live env so
        // that re-wiring via `parler connect` (which rewrites the env block) takes effect.
        return Ok(apply_env_overrides(Config::load()?));
    }
    // First run — mint the identity from the env (falling back to $USER / the public hub).
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

/// Apply the `explicit env var > saved config > default` rule to a *loaded* config's hub/name/role.
///
/// The identity (id + seed) is untouched — only the mutable, re-wireable fields follow the live
/// environment, so pointing an already-bootstrapped agent at a new hub is a matter of rewriting its
/// `PARLER_HUB` env (what `parler connect` does) rather than editing `config.json`. Each field the
/// env actually changes is announced once — stderr for the user, `log_event` so `parler doctor` can
/// show which hub was chosen and why (clig.dev: "if you change state, tell the user").
pub(crate) fn apply_env_overrides(cfg: Config) -> Config {
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
    apply_overrides(cfg, env("PARLER_HUB"), env("PARLER_NAME"), env("PARLER_ROLE"), &mut |line| {
        // Announce each real change once: stderr for the user, `log_event` so `parler doctor` can
        // show which hub was chosen and why (clig.dev: "if you change state, tell the user").
        eprintln!("parler: {line}");
        log_event(&line);
    })
}

/// The pure precedence rule, factored out of [`apply_env_overrides`] so it can be unit-tested
/// without touching (racy, process-global) environment variables — each present env value wins over
/// the saved config; the identity (id + seed) is never touched. `note` is called once per field the
/// env actually changes, so the caller decides how to surface it.
fn apply_overrides(
    mut cfg: Config,
    env_hub: Option<String>,
    env_name: Option<String>,
    env_role: Option<String>,
    note: &mut dyn FnMut(String),
) -> Config {
    if let Some(hub) = env_hub {
        if hub != cfg.hub_url {
            note(format!("PARLER_HUB overrides saved hub — dialing {hub} (was {})", cfg.hub_url));
            cfg.hub_url = hub;
        }
    }
    if let Some(name) = env_name {
        if name != cfg.name {
            note(format!("PARLER_NAME overrides saved name — '{name}' (was '{}')", cfg.name));
            cfg.name = name;
        }
    }
    if let Some(role) = env_role {
        if cfg.role.as_deref() != Some(role.as_str()) {
            let was = cfg.role.as_deref().unwrap_or("none").to_string();
            note(format!("PARLER_ROLE overrides saved role — '{role}' (was '{was}')"));
            cfg.role = Some(role);
        }
    }
    cfg
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
                | "parler_join" | "parler_send" | "parler_recv" | "parler_handoff"
                | "parler_join_requests" | "parler_approve_join" | "parler_deny_join"
                | "parler_watch_session" => {
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
            // Validate against the documented enum; an unknown kind errors instead of silently
            // becoming a DM (#110 case 2). "channel" stays an accepted alias of "group" for older
            // callers, but a typo no longer maps to dm.
            let kind = match s("kind").as_deref() {
                None | Some("dm") => RoomKind::Dm,
                Some("group") | Some("channel") => RoomKind::Channel,
                Some("service") => RoomKind::Service,
                Some(other) => bail!("unknown invite kind '{other}' — use one of: dm, group, service"),
            };
            let inv = agent
                .invite(kind, s("name"), args.get("ttl_secs").and_then(Value::as_u64), u32opt("max_uses"))
                .await?;
            Ok(format!(
                "invite ready — {} room '{}'.\ncode: {}\nlink: {}\nThe other agent calls parler_join with the code.",
                inv.kind.as_str(),
                inv.room,
                inv.code,
                inv.url,
            ))
        }
        "parler_serve" => {
            let svc = s("service").ok_or_else(|| anyhow!("missing 'service'"))?;
            let room = agent.serve(&svc).await?;
            Ok(format!("serving '{svc}' (room '{room}')"))
        }
        "parler_push" => {
            let target = select_target(s("room"), s("to"), s("service"))?
                .ok_or_else(|| anyhow!("provide exactly one of room / to / service"))?;
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
                "pushed git bundle to '{}' (seq {}, {} bytes). tip: {} {summary}\n\
                 The peer runs (MCP): parler_apply blob={}\n\
                 The peer runs (CLI): parler apply {}",
                r.room,
                r.seq,
                bytes.len(),
                tip,
                r.blob_id,
                r.blob_id,
            ))
        }
        "parler_fetch" => {
            let id = s("id").ok_or_else(|| anyhow!("missing 'id'"))?;
            let bytes = agent.fetch_blob(&id).await?;
            let out = s("out").unwrap_or_else(|| format!("{}.bundle", &id[..id.len().min(12)]));
            let out_path = std::path::PathBuf::from(out);
            std::fs::write(&out_path, &bytes)?;
            let abs_out = std::fs::canonicalize(&out_path).unwrap_or_else(|_| {
                std::env::current_dir().unwrap_or_default().join(&out_path)
            });
            let abs_out_str = abs_out.to_string_lossy();
            Ok(format!("wrote {} bytes to {} (apply with: git bundle verify {} && git fetch {})", bytes.len(), abs_out_str, abs_out_str, abs_out_str))
        }
        "parler_apply" => {
            let blob = s("blob").ok_or_else(|| anyhow!("missing 'blob'"))?;
            let path_val = s("path");
            let resolved_dir = match path_val.as_deref() {
                Some(d) => std::fs::canonicalize(std::path::Path::new(d))
                    .map_err(|e| anyhow!("invalid path '{d}': {e}"))?,
                None => std::env::current_dir()?,
            };
            let repo_str = resolved_dir.to_str().ok_or_else(|| anyhow!("non-UTF8 repo path"))?;
            if crate::git_in(Some(repo_str), &["rev-parse", "--git-dir"]).is_err() {
                bail!("not inside a git repository — run `parler_apply` from the repo you want to import into (path: {})", repo_str);
            }
            let bytes = agent.fetch_blob(&blob).await?;
            let tmp = std::env::temp_dir().join(format!("parler-apply-{}.bundle", std::process::id()));
            std::fs::write(&tmp, &bytes)?;
            let refname = format!("refs/parler/{}", crate::short(&blob));
            let result = (|| -> Result<String> {
                let tmp_s = crate::path_str(&tmp)?;
                if let Err(e) = crate::git_in(Some(repo_str), &["bundle", "verify", tmp_s]) {
                    bail!("bundle verify failed (you may be missing the base commit it is thin against): {e}");
                }
                crate::git_in(Some(repo_str), &["fetch", tmp_s])?;
                let heads = crate::git_in(Some(repo_str), &["bundle", "list-heads", tmp_s])?;
                let tip_sha = heads.split_whitespace().next().unwrap_or_default().to_string();
                if !tip_sha.is_empty() {
                    crate::git_in(Some(repo_str), &["update-ref", &refname, &tip_sha])?;
                }
                Ok(heads)
            })();
            let _ = std::fs::remove_file(&tmp);
            let heads = result?;
            let abs_dir_str = resolved_dir.to_string_lossy();
            Ok(format!(
                "imported into {} in repository {} (working tree untouched).\nheads:\n{}",
                refname,
                abs_dir_str,
                heads
            ))
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
            // Merge semantics (#110 case 4): every connect already auto-registers a card from the
            // PARLER_* env, so a field the caller *omits* must keep its current card value. Otherwise a
            // no-arg parler_register silently flipped a PARLER_PUBLIC card back to private and dropped
            // env tags/skills. Only fields explicitly passed are changed; the result states the diff.
            let my_id = agent.id.clone();
            let current = agent.lookup(&my_id).await.ok().flatten();
            let cur_card = current.as_ref().map(|e| e.card.clone());

            let visibility = match args.get("visibility").and_then(Value::as_str) {
                Some(_) => visibility, // explicit → honor it (public/anything-else = private, as before)
                None => current.as_ref().map(|e| e.visibility).unwrap_or(visibility),
            };
            let tags = if args.get("tags").is_some() {
                str_list(args, "tags")
            } else {
                cur_card.as_ref().and_then(|c| c.tags.clone()).unwrap_or_default()
            };
            let skills = if args.get("skills").is_some() {
                str_list(args, "skills")
                    .into_iter()
                    .map(|k| AgentSkill { id: k.clone(), name: k, description: None })
                    .collect()
            } else {
                cur_card.as_ref().and_then(|c| c.skills.clone()).unwrap_or_default()
            };
            let description = match s("description") {
                Some(d) => Some(d),
                None => cur_card.as_ref().and_then(|c| c.description.clone()),
            };
            let tag_n = tags.len();
            let (visibility, verified) = agent.register(visibility, tags, skills, description).await?;
            Ok(format!(
                "registered as {} ({}) — {} tag(s) kept/set",
                visibility.as_str(),
                if verified { "signature verified" } else { "unsigned" },
                tag_n
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

/// Enforce "exactly one of room / to / service". Returns `None` when the caller passed no selector
/// (the caller then supplies its own fallback — the active session for send/handoff, or an error for
/// push). Errors naming the conflict when more than one selector is given, so a mistaken double
/// target no longer resolves silently by if-else precedence. (#110 case 1.)
fn select_target(
    room: Option<String>,
    to: Option<String>,
    service: Option<String>,
) -> Result<Option<Target>> {
    let mut chosen: Option<Target> = None;
    let mut names: Vec<&str> = Vec::new();
    if let Some(r) = room {
        names.push("room");
        chosen = Some(Target::Room { room: r });
    }
    if let Some(t) = to {
        names.push("to");
        chosen = Some(Target::Dm { agent: t });
    }
    if let Some(sv) = service {
        names.push("service");
        chosen = Some(Target::Service { service: sv });
    }
    if names.len() > 1 {
        bail!("provide exactly one target, but got {} ({}) — pick a single room, to, or service", names.len(), names.join(" + "));
    }
    Ok(chosen)
}

/// Resolve an approve/deny target against a room's pending join requests. The owner sees the
/// joiner's *name* in the pending-join notice, so accept an id or a unique pending name (matching
/// what `parler_send` does for directory names). Returns the joiner's id to pass to the hub.
async fn resolve_pending_joiner(agent: &mut MeshAgent, room: &str, who: &str) -> Result<String> {
    let pending = agent.join_requests(room).await?;
    // Exact id match wins outright (also the old-behavior fast path).
    if pending.iter().any(|r| r.agent == who) {
        return Ok(who.to_string());
    }
    let by_name: Vec<&parler_protocol::JoinRequest> =
        pending.iter().filter(|r| r.name.eq_ignore_ascii_case(who)).collect();
    match by_name.len() {
        1 => Ok(by_name[0].agent.clone()),
        0 => bail!(
            "no pending join request from '{who}' in session '{room}' — run parler_join_requests to \
             see who is waiting (each line shows a name and an id)"
        ),
        _ => {
            let list = by_name
                .iter()
                .map(|r| format!("  {}  {}", r.name, r.agent))
                .collect::<Vec<_>>()
                .join("\n");
            bail!("'{who}' matches more than one pending joiner in session '{room}' — pass the id instead:\n{list}")
        }
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
            let wait_secs = args.get("wait_secs").and_then(Value::as_u64).filter(|w| *w > 0);
            join_session(state, &key, Backlog::from_arg(s("backlog").as_deref()), wait_secs).await
        }
        "parler_join" => {
            // One code, one door (#109): a session key redeemed through parler_join behaves exactly
            // like parler_join_session — a Channel room (what open_session mints) adopts session
            // semantics (active session + digest + backlog), and a pending approval reads as
            // success-in-progress, not an error. Plain DM/service invites keep the lightweight join.
            let code = s("code").ok_or_else(|| anyhow!("missing 'code'"))?;
            let wait_secs = args.get("wait_secs").and_then(Value::as_u64).filter(|w| *w > 0);
            match state.agent.redeem(&code).await? {
                JoinOutcome::Pending { room } => match wait_for_approval(state, &code, wait_secs).await? {
                    Some(room) => enter_session(state, room, Backlog::Recent).await,
                    None => Ok(pending_output(&room)),
                },
                JoinOutcome::Joined { room, kind } => match kind {
                    RoomKind::Channel => enter_session(state, room, Backlog::Recent).await,
                    _ => Ok(format!("joined {} room '{}'", kind.as_str(), room)),
                },
            }
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
            let who = s("agent").ok_or_else(|| anyhow!("missing 'agent' (the joiner's id or name, as shown in the pending-join notice)"))?;
            let approve = name == "parler_approve_join";
            // Resolve `who` the way the owner saw it: the pending-join notice shows a name, so accept
            // an id *or* a unique pending name (ambiguity → error listing candidates with ids), like
            // parler_send resolves directory names. Only reach for the hub round-trip when it isn't
            // already an exact id match against the pending set.
            let who = resolve_pending_joiner(&mut state.agent, &room, &who).await?;
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
                 Give it to the user to paste into the website's /session viewer (they'll see the \
                 conversation + agent count, without joining). Anyone with the code can read the \
                 session, so treat it like a password."
            ))
        }
        "parler_send" => {
            let text = s("text").ok_or_else(|| anyhow!("missing 'text'"))?;
            // Exactly one explicit target; otherwise default to the active session.
            let target = match select_target(s("room"), s("to"), s("service"))? {
                Some(t) => t,
                None => match state.active_session.clone() {
                    Some(room) => Target::Room { room },
                    None => bail!("provide one of room / to / service, or open/join a session first"),
                },
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
            let target = match select_target(s("room"), s("to"), s("service"))? {
                Some(t) => t,
                None => match state.active_session.clone() {
                    Some(room) => Target::Room { room },
                    None => bail!("provide one of room / to / service, or open/join a session first"),
                },
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
            let effective_limit = recv_limit(explicit_limit, verbose_render());
            let (mut msgs, mut cursor) = state.agent.pull(&room, since, effective_limit).await?;
            // Long-poll: if nothing new yet and the caller asked to wait, prefer the hub's
            // **server-side wait** (`pull_wait`) — it works with zero push machinery (even on a
            // connection whose `Subscribe` failed), heartbeats the socket during the wait, and
            // transparently reconnects a half-open transport. Only in cursor mode (`since` absent) —
            // an explicit `since` is a full-detail history re-read, never a live tail.
            let wait_secs = args.get("wait_secs").and_then(Value::as_u64).filter(|w| *w > 0);
            let mut degraded = false;
            if msgs.is_empty() && since.is_none() {
                if let Some(secs) = wait_secs {
                    let secs = secs.min(WAIT_SECS_MAX);
                    // Retry a failed initial subscribe once (a latency win when it succeeds); the wait
                    // itself doesn't depend on it.
                    let _ = state.agent.resubscribe_if_needed().await;
                    let (m, c, waited) = state.agent.pull_wait(&room, effective_limit, secs).await?;
                    msgs = m;
                    cursor = c;
                    degraded = degraded_wait(msgs.is_empty(), waited, state.agent.push_active());
                }
            }
            let batch_full = effective_limit.is_some_and(|l| msgs.len() as u32 >= l);
            let mut out = if msgs.is_empty() {
                // Honest degraded mode: only when a wait was requested but genuinely couldn't happen
                // (old hub, no push) — so the agent knows this returned immediately and can pace
                // itself instead of hammering. One short line; never shown when the wait did work.
                if degraded {
                    format!("(no new messages in '{room}') — long-poll unavailable (hub too old for server-side wait, no push); polling instead")
                } else {
                    format!("(no new messages in '{room}')")
                }
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
        "Joiners ask to join; YOU approve each (a prompt appears, or parler_join_requests) before they \
         can read it — a leaked key can't quietly pull your context."
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
        "session open — room '{room}', now your active session.\n\
         KEY: {code}\n\
         Give a teammate the KEY (they call parler_join_session) or this ready-to-run one-liner:\n    \
         {oneliner}\n\
         Either lands them in this same conversation, caught up — no copy-paste.\n\
         {gate}\n\
         Keep late joiners cheap: parler_remember key=\"session-digest\" room=\"{room}\" text=\"SESSION DIGEST: …\" (re-save to update).\n\
         parler_watch_session gives the user a read-only web viewer code.\n\
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

/// The reserved key for the host's rolling session recap. Re-saving it overwrites (idempotent upsert),
/// so it's always the *current* summary. A late join recalls and surfaces it above the tail.
const SESSION_DIGEST_KEY: &str = "session-digest";
/// The sentinel the digest text starts with — both the query we recall by and the check that a hit is
/// genuinely the digest (guards against a BM25 false positive matching only the query words).
const SESSION_DIGEST_SENTINEL: &str = "SESSION DIGEST";

/// Default cap on how many messages a cursor-mode `parler_recv` renders per call. Lossless: a limited
/// `Pull` advances the cursor only through the returned batch, so the remainder stays unread for the
/// next call (see store.rs). An explicit `limit`/`since` overrides this.
const RECV_DEFAULT_LIMIT: u32 = 30;

/// Hard cap on a `parler_recv wait_secs` / `parler_join_session wait_secs` long-poll, matching the
/// hub's own `MAX_WAIT_SECS` park bound so the client never asks the hub to hold a request longer
/// than the hub will honor.
const WAIT_SECS_MAX: u64 = 60;

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
fn recv_limit(explicit: Option<u32>, verbose: bool) -> Option<u32> {
    match (explicit, verbose) {
        // An explicit limit always wins; otherwise the default cap applies to *both* cursor reads and
        // `since` re-reads (#110 case 3 — a `since` poll used to silently drop the limit and replay
        // unbounded full-detail history). Verbose stays the global uncapped escape hatch.
        (Some(l), _) => Some(l),
        (None, false) => Some(RECV_DEFAULT_LIMIT),
        (None, true) => None,
    }
}

/// Whether a `parler_recv wait_secs` is **genuinely degraded** — i.e. the wait was requested but
/// couldn't actually happen — so the result should carry an honest "polling instead" note. Pure so
/// it's testable without a hub: true only when the room stayed empty AND no server-side wait occurred
/// (`waited == false`) AND there's no push subscription to fall back on. Against a current hub the
/// hub parks the request (`waited == true`), so this is `false` and the note never appears.
fn degraded_wait(empty: bool, waited: bool, push_active: bool) -> bool {
    empty && !waited && !push_active
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

/// Join a shared session by key. For an approval-gated session the redeem only *requests* entry — the
/// host must admit us first. With `wait_secs` this **one call** spans the approval wait (re-redeeming
/// on an interval until the host decides or the budget runs out, keeping the socket alive with
/// heartbeats), instead of returning "pending" and asking the agent to call again N times. Without
/// `wait_secs` it keeps the original short poll (a quick approval still resolves in-call). A denial
/// surfaces as an error from redeem and propagates out. Once admitted, pull the backlog to catch up
/// (advancing the cursor to the live edge), adopt it as the active session, and announce arrival. The
/// backlog renders as a digest by default (`Backlog::Recent`) — `Backlog::Full` replays everything.
async fn join_session(
    state: &mut McpState,
    key: &str,
    backlog: Backlog,
    wait_secs: Option<u64>,
) -> Result<String> {
    match state.agent.redeem(key).await? {
        JoinOutcome::Joined { room, .. } => enter_session(state, room, backlog).await,
        JoinOutcome::Pending { room } => match wait_for_approval(state, key, wait_secs).await? {
            Some(room) => enter_session(state, room, backlog).await,
            None => Ok(pending_output(&room)),
        },
    }
}

/// Adopt a just-redeemed room as the active session and render the catch-up context. Shared by
/// `parler_join_session` and `parler_join` (#109) so a session key does the same thing through either
/// door — active session set, backlog digested, arrival announced.
async fn enter_session(state: &mut McpState, room: String, backlog: Backlog) -> Result<String> {
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
    // If the host maintains a rolling `session-digest` fact (P1.3 convention), surface it above the
    // tail — it's a human-written recap that beats re-reading the raw backlog. Silent when absent.
    let digest_line = session_digest(&mut state.agent, &room)
        .await
        .map(|d| format!("--- session digest ---\n{d}\n"))
        .unwrap_or_default();
    // Roster as a count, not a full listing — the join stays cheap; parler_roster gives the details.
    let roster_line = match state.agent.roster(&room).await {
        Ok(entries) => format!("\n— {} agent(s) in the room —", entries.len()),
        Err(_) => String::new(),
    };
    Ok(format!(
        "joined session — room '{room}', now your active session.\n\
         {digest_line}--- context so far ---\n{body}\n--- end context ---{roster_line}"
    ))
}

/// The success-in-progress result for an approval-gated redeem the host hasn't decided yet. NOT an
/// error — the request *was* filed — and identical whether reached via parler_join_session or
/// parler_join (#109).
fn pending_output(room: &str) -> String {
    format!(
        "⏳ join request sent — waiting for the host to approve you into session '{room}'.\n\
         You are NOT in the conversation yet and cannot see its context until the host \
         approves. Call parler_join_session again with the same key (add wait_secs to \
         hold this one call open until the host decides)."
    )
}

/// The rolling session digest, if the host maintains one. Convention (P1.3): the host upserts a
/// room-scoped fact `remember(key="session-digest", room, text="SESSION DIGEST: …")`, so a late joiner
/// gets a human-written recap for free. We recall the top hit and accept it only when it's actually
/// that keyed fact (key match) **and** carries the sentinel — belt-and-suspenders against a BM25 false
/// positive matching the query words. `None` (silent) when there's no digest or the recall fails.
/// A deterministic fetch-by-key (no BM25) is the P2.1 upgrade.
async fn session_digest(agent: &mut MeshAgent, room: &str) -> Option<String> {
    let hits = agent
        .recall(SESSION_DIGEST_SENTINEL, Some(room.to_string()), Some(1), None)
        .await
        .ok()?;
    let hit = hits.into_iter().next()?;
    let is_the_key = hit.key.as_deref() == Some(SESSION_DIGEST_KEY);
    let has_sentinel = hit.text.trim_start().starts_with(SESSION_DIGEST_SENTINEL);
    (is_the_key && has_sentinel).then_some(hit.text)
}

/// How long `join_session` waits for a host approval when the caller gave **no** `wait_secs`: a short
/// poll (bounded by these) so a quick approval resolves in the same call, but a human-paced one
/// doesn't block the joiner indefinitely — it returns "pending" and the agent retries.
const JOIN_POLL_ATTEMPTS: usize = 3;
const JOIN_POLL_INTERVAL_MS: u64 = 500;

/// How often the `wait_secs` approval wait re-redeems to check the host's decision. A pending joiner
/// isn't a room member, so it can't park on a server-side `Pull` wait (that's member-gated) — instead
/// it re-redeems (an idempotent poll: it charges no extra use and adds no queue entry) on this cadence,
/// while heartbeats keep the socket alive across a multi-minute human approval.
const APPROVAL_POLL_INTERVAL: Duration = Duration::from_millis(750);

/// Wait for a host to approve a pending join, spanning **one** tool call. With `wait_secs` set, re-redeem
/// `key` every [`APPROVAL_POLL_INTERVAL`] (heartbeating between polls to catch a half-open socket) until
/// the host admits us (`Some(room)`), the budget runs out (`None` ⇒ still pending), or a denial errors
/// out. Without `wait_secs`, fall back to the original short poll. A denial from `redeem` propagates as
/// an error either way.
async fn wait_for_approval(
    state: &mut McpState,
    key: &str,
    wait_secs: Option<u64>,
) -> Result<Option<String>> {
    match wait_secs {
        // No budget: the original short poll (a fast approval still resolves in-call).
        None => {
            for _ in 0..JOIN_POLL_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(JOIN_POLL_INTERVAL_MS)).await;
                if let JoinOutcome::Joined { room, .. } = state.agent.redeem(key).await? {
                    return Ok(Some(room));
                }
            }
            Ok(None)
        }
        // Budgeted long-poll: hold this one call open until the host decides or the window closes.
        Some(secs) => {
            let deadline = Instant::now() + Duration::from_secs(secs.min(WAIT_SECS_MAX));
            loop {
                if let JoinOutcome::Joined { room, .. } = state.agent.redeem(key).await? {
                    return Ok(Some(room));
                }
                if Instant::now() >= deadline {
                    return Ok(None); // still pending when the window closed
                }
                // Keep the connection alive across a human-paced approval (detects + heals a half-open
                // socket so the next re-redeem lands on a live connection), then poll again.
                state.agent.heartbeat().await;
                let remaining = deadline.saturating_duration_since(Instant::now());
                tokio::time::sleep(APPROVAL_POLL_INTERVAL.min(remaining)).await;
            }
        }
    }
}

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
            "Join a session with a KEY. If approval is required you're held pending until the host admits you; pass wait_secs to hold this call open until they decide. Once in, you get a digest of the context (seed + recent tail); backlog:\"full\" replays everything, or parler_recv since=<seq> re-reads a range in full. Becomes your active session (parler_send/parler_recv need no room).",
            json!({
                "key": { "type": "string", "description": "the session key or link you were handed" },
                "backlog": { "type": "string", "enum": ["recent", "full"], "description": "recent (default): seed + recent tail; full: replay the entire backlog" },
                "wait_secs": { "type": "integer", "description": "approval-gated join: seconds to wait in THIS call for the host to approve (≤60); resolves the moment they do. Omit to return 'pending' now." }
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
                "agent": { "type": "string", "description": "the joiner's name or id (from parler_join_requests)" },
                "room": { "type": "string", "description": "the session room (defaults to your active session)" }
            }),
            &["agent"],
        ),
        tool(
            "parler_deny_join",
            "Reject a pending joiner — turned away, can't re-request. Pass the joiner's name or id. Defaults to active session.",
            json!({
                "agent": { "type": "string", "description": "the joiner's name or id (from parler_join_requests)" },
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
            "Redeem any pasted code/link. A session key acts like parler_join_session (active session + digest; gated = 'waiting for host', not an error); a DM/service invite just joins.",
            json!({
                "code": { "type": "string", "description": "the code, link, or session key you were handed" }
            }),
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
                "since": { "type": "integer", "description": "read-only replay from seq in full (no cursor advance); default limit applies unless you pass limit" },
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
            "Save fact to shared memory. Re-saving with key overwrites (idempotent) — e.g. key=\"session-digest\" keeps rolling recap. Optionally scope to room or pass embedding.",
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
            "Recall saved facts (BM25 full-text; hybrid BM25 + vector KNN when embedding is given). Cheaper than re-reading history for state saved with parler_remember.",
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
            "Build and push a git bundle from the repo to a target. With base, bundle only base..gitref (thin patch).",
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
            "Download a pushed bundle's bytes by blob id to a file. Does NOT apply.",
            json!({
                "id": { "type": "string" },
                "out": { "type": "string", "description": "output file (default: <blob>.bundle)" }
            }),
            &["id"],
        ),
        tool(
            "parler_apply",
            "Download a pushed bundle and apply the git bundle to the target repo.",
            json!({
                "blob": { "type": "string" },
                "path": { "type": "string", "description": "repository directory path (default: current directory)" }
            }),
            &["blob"],
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
            "Publish your discovery card. visibility: private (default, same-hub) or public (anyone). Signed with your key.",
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
            "Discover agents on hub (default) or public. Filter by query/tag/skill/status. Results are compact; pass detail:true for ids.",
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
        // Subscribe for the push latency optimization (best-effort). Long-poll no longer depends on
        // it — server-side wait works without push — so a failed subscribe here doesn't degrade recv.
        let _ = agent.subscribe().await;
        McpState { agent, active_session: None }
    }

    /// Like [`state`], but does **not** subscribe — exercises the long-poll path on a connection that
    /// holds no push subscription (the previously-degraded mode #90 fixes).
    async fn state_no_push(hub: &str, name: &str) -> McpState {
        let cfg = Config::create(hub.to_string(), name.to_string(), None).unwrap();
        let agent = MeshAgent::connect(&cfg).await.unwrap();
        McpState { agent, active_session: None }
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
    /// session. Pre-diet baseline 11,598 B; post-diet (P0.2) 11,030 B. The UX-accuracy round
    /// (#107/#109/#110) added ~250 B so descriptions match the newly-enforced behavior (one-door
    /// parler_join, name-resolving approve/deny, strict inputs) — the point of those issues. Ceiling
    /// raised to 12,000 to keep a meaningful guardrail against large regressions.
    const TOOL_SPECS_BUDGET: usize = 12_000;
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
    /// `open_session` result string (P1.4 trim). Measured 615 chars on the public hub; a private-hub
    /// one-liner adds a PARLER_HUB/PARLER_JOIN_SECRET env, so leave headroom. Keeps the prose from
    /// bloating back up.
    const OPEN_RESULT_BUDGET: usize = 800;

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
        let joined = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
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
        let joined = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();

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
        let joined = join_session(&mut bob, &key, Backlog::Full, None).await.unwrap();

        // Full mode replays even a mid-backlog message and never emits the omission line.
        assert!(joined.contains("m5 "), "full mode replays the middle:\n{joined}");
        assert!(joined.contains("m30 "), "full mode replays late-middle messages too");
        assert!(!joined.contains("earlier message(s) omitted"), "full mode has no omission line");
    }

    #[tokio::test]
    async fn join_surfaces_session_digest_fact() {
        // When the host maintains the rolling session-digest fact, a late joiner sees it above the tail.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), Some("plan".into()), None, None, false).await.unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);
        // Host writes the keyed recap.
        alice
            .agent
            .remember("SESSION DIGEST: auth done, next is billing", Some(SESSION_DIGEST_KEY.into()), Some(room.clone()), None, None)
            .await
            .unwrap();

        let mut bob = state(&hub, "bob").await;
        let joined = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        assert!(joined.contains("session digest"), "the digest header is shown:\n{joined}");
        assert!(joined.contains("auth done, next is billing"), "the recap text is surfaced:\n{joined}");
    }

    #[tokio::test]
    async fn join_without_digest_fact_is_silent() {
        // No digest fact → no header, no error (silent skip).
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), Some("plan".into()), None, None, false).await.unwrap();
        let key = key_of(&opened);
        let mut bob = state(&hub, "bob").await;
        let joined = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        assert!(!joined.contains("session digest"), "no digest header when the fact is absent:\n{joined}");
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
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
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
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
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
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();

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
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
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
        // Explicit limit always wins (even in verbose).
        assert_eq!(recv_limit(Some(5), false), Some(5));
        assert_eq!(recv_limit(Some(5), true), Some(5));
        // Plain cursor read → the default cap.
        assert_eq!(recv_limit(None, false), Some(RECV_DEFAULT_LIMIT));
        // A history re-read (`since`) now also gets the default cap unless a limit is explicit (#110).
        assert_eq!(recv_limit(None, false), Some(RECV_DEFAULT_LIMIT));
        // Verbose is the global escape hatch → uncapped.
        assert_eq!(recv_limit(None, true), None);
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
        // Every actionable artifact survives the P1.4 trim.
        assert!(opened.contains("link:"), "share link present");
        assert!(opened.contains("session-digest"), "digest guidance present");
        assert!(opened.contains("parler_watch_session"), "watch-viewer pointer present");
        assert!(alice.active_session.is_some());
        // P1.4: the result is trimmed. The one-liner + a variable-length key/link dominate; assert a
        // ceiling so the prose can't bloat back up.
        println!("[budget] open_session result: {} chars", opened.chars().count());
        assert!(opened.len() <= OPEN_RESULT_BUDGET, "open_session result {} B over budget {OPEN_RESULT_BUDGET} B", opened.len());

        let key = key_of(&opened);
        let joined = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
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
        let joined = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        assert!(!joined.contains("session context"), "no seed message when context is omitted");
    }

    #[tokio::test]
    async fn send_defaults_to_active_session_and_autopull_filters_own() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let opened = open_session(&mut alice, Some("seed"), Some("design".into()), None, None, false).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();

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
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap(); // advances bob's cursor to the live edge

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
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        join_session(&mut carol, &key, Backlog::Recent, None).await.unwrap();
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
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
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
        assert!(bob.agent.push_active(), "the hub should support push so recv can long-poll");

        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap(); // bob caught up to the live edge
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
    async fn recv_wait_secs_long_polls_without_a_push_subscription() {
        // #90 / #87: `parler_recv wait_secs` delivers on a connection that started with NO push
        // subscription — the previously-degraded mode. Bob uses `state_no_push` (never subscribed);
        // the message arrives via the hub's server-side wait (recv may opportunistically re-subscribe
        // for future latency, per #87 — but the *wait itself* did not need it).
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state_no_push(&hub, "bob").await;
        assert!(!bob.agent.push_active(), "bob starts with no push subscription");

        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        // Drain bob's own "joined" announce so the long-poll starts from an empty inbox.
        call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();

        let send_args = json!({ "text": "no sub needed" });
        let recv_args = json!({ "wait_secs": 8 });
        let send = async {
            tokio::time::sleep(Duration::from_millis(120)).await;
            call_session_tool(&mut alice, "parler_send", &send_args).await.unwrap();
        };
        let recv = call_session_tool(&mut bob, "parler_recv", &recv_args);
        let (_s, got) = tokio::join!(send, recv);
        let got = got.unwrap();
        assert!(got.contains("no sub needed"), "server-side wait delivered without a subscription:\n{got}");
        assert!(!got.contains("long-poll unavailable"), "the wait worked, so no degraded note:\n{got}");
    }

    #[tokio::test]
    async fn recv_wait_secs_shows_no_degraded_note_against_a_current_hub() {
        // The "never otherwise" half of the degraded-note AC: even when the wait times out empty
        // against a current (parking) hub, the note is absent — the wait *did* happen, it just found
        // nothing. The note is reserved for a genuinely-old hub.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;
        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap(); // drain the announce

        // A short wait with nobody sending → empty, but NOT degraded (the hub parked us).
        let out = call_session_tool(&mut bob, "parler_recv", &json!({ "wait_secs": 1 })).await.unwrap();
        assert!(out.contains("no new messages"), "empty at timeout: {out}");
        assert!(!out.contains("long-poll unavailable"), "a real park is not degraded: {out}");
    }

    #[test]
    fn degraded_wait_note_only_when_the_wait_truly_could_not_happen() {
        // Pure decision table for the honest-degraded-mode note (issue #87 AC).
        // Degraded ONLY when: room empty AND no server-side wait AND no push fallback.
        assert!(degraded_wait(true, false, false), "empty + no wait + no push ⇒ degraded");
        // Not degraded if the hub actually parked (waited)…
        assert!(!degraded_wait(true, true, false));
        // …or a push subscription exists to fall back on…
        assert!(!degraded_wait(true, false, true));
        // …or messages arrived (not empty), regardless of the rest.
        assert!(!degraded_wait(false, false, false));
        assert!(!degraded_wait(false, true, true));
    }

    #[tokio::test]
    async fn join_session_wait_secs_resolves_when_host_approves_in_window() {
        // #90: an approval-gated join with `wait_secs` resolves within ONE call when the host approves
        // during the window — no manual retrying. Alice approves ~150ms into bob's 10s wait.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let opened = open_session(
            &mut alice,
            Some("secret plan: ship friday"),
            Some("plan".into()),
            None,
            None,
            true, // approval-gated
        )
        .await
        .unwrap();
        let key = key_of(&opened);
        let bob_id = bob.agent.id.clone();

        // Bob joins with a wait; concurrently alice approves him shortly after his request lands.
        let approve = async {
            // Give bob's redeem time to register the pending request, then approve.
            tokio::time::sleep(Duration::from_millis(300)).await;
            // Poll until the request shows up, then approve (robust against scheduling jitter).
            for _ in 0..40 {
                let reqs = alice.agent.join_requests(alice.active_session.as_ref().unwrap()).await.unwrap();
                if reqs.iter().any(|r| r.agent == bob_id) {
                    alice
                        .agent
                        .resolve_join(alice.active_session.as_ref().unwrap(), &bob_id, true)
                        .await
                        .unwrap();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            panic!("bob's join request never appeared");
        };
        let join = join_session(&mut bob, &key, Backlog::Recent, Some(10));
        let (_a, joined) = tokio::join!(approve, join);
        let joined = joined.unwrap();
        assert!(
            joined.contains("secret plan: ship friday"),
            "the join resolved in one call and got the context:\n{joined}"
        );
        assert_eq!(bob.active_session, alice.active_session, "bob is now in the session");
    }

    #[tokio::test]
    async fn join_session_wait_secs_returns_pending_if_host_never_decides() {
        // The wait is bounded: if nobody approves within the window, it returns the "pending" message
        // (one honest call), not an error and not a hang.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;
        let opened =
            open_session(&mut alice, Some("hidden"), Some("plan".into()), None, None, true).await.unwrap();
        let key = key_of(&opened);

        let started = std::time::Instant::now();
        let out = join_session(&mut bob, &key, Backlog::Recent, Some(1)).await.unwrap();
        assert!(started.elapsed() >= Duration::from_secs(1), "it held the call open for the window");
        assert!(out.contains("waiting for the host"), "still pending after the window: {out}");
        assert!(!out.contains("hidden"), "a pending joiner must not receive the context");
        assert!(bob.active_session.is_none(), "not admitted");
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
        let pending = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
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
        let joined = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        assert!(joined.contains("secret plan: ship friday"), "an approved joiner gets the context: {joined}");
        assert_eq!(bob.active_session, alice.active_session, "both now share the session room");
    }

    #[test]
    fn select_target_enforces_exactly_one() {
        // Zero selectors → None (caller supplies the fallback).
        assert!(select_target(None, None, None).unwrap().is_none());
        // Exactly one → that target.
        assert!(matches!(select_target(Some("r".into()), None, None).unwrap(), Some(Target::Room { .. })));
        assert!(matches!(select_target(None, Some("a".into()), None).unwrap(), Some(Target::Dm { .. })));
        // More than one → an error that names the conflict (no silent precedence).
        let err = select_target(Some("r".into()), Some("a".into()), None).unwrap_err().to_string();
        assert!(err.contains("exactly one") && err.contains("room") && err.contains("to"), "err: {err}");
        assert!(select_target(Some("r".into()), Some("a".into()), Some("s".into())).is_err());
    }

    #[tokio::test]
    async fn invite_rejects_an_unknown_kind() {
        // #110 case 2: a typo'd kind errors instead of silently becoming a DM.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let bad = call_tool(&mut alice.agent, "parler_invite", &json!({ "kind": "grup" })).await;
        assert!(bad.is_err(), "unknown kind must error");
        assert!(bad.unwrap_err().to_string().contains("dm, group, service"), "error names the valid kinds");
        // A valid kind still works.
        assert!(call_tool(&mut alice.agent, "parler_invite", &json!({ "kind": "group", "name": "x" })).await.is_ok());
    }

    #[tokio::test]
    async fn register_no_args_keeps_public_visibility_and_tags() {
        // #110 case 4: a no-arg parler_register on an env-configured card is a no-op — it must not
        // flip a public card private or drop its tags.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        // Establish a public card with tags (as auto-register from PARLER_* env would).
        call_tool(&mut alice.agent, "parler_register", &json!({ "visibility": "public", "tags": ["rust", "infra"] }))
            .await
            .unwrap();
        // No-arg re-register: visibility and tags survive.
        let out = call_tool(&mut alice.agent, "parler_register", &json!({})).await.unwrap();
        assert!(out.contains("public"), "no-arg register kept public visibility: {out}");
        assert!(out.contains("2 tag"), "no-arg register kept the tags: {out}");
    }

    #[tokio::test]
    async fn join_tool_and_join_session_agree_on_a_session_key() {
        // #109 "one code, one door": a session key redeemed via parler_join sets up the session the
        // same way parler_join_session does — active session set + context digest delivered.
        let hub = start_hub().await;
        let mut host = state(&hub, "host").await;
        let opened =
            open_session(&mut host, Some("blueprint: ship the thing"), None, None, None, false).await.unwrap();
        let key = key_of(&opened);

        let mut bob = state(&hub, "bob").await;
        let via_join = call_session_tool(&mut bob, "parler_join", &json!({ "code": key })).await.unwrap();
        assert!(via_join.contains("blueprint: ship the thing"), "parler_join delivers the context: {via_join}");
        assert!(via_join.contains("active session"), "parler_join adopts the active session: {via_join}");
        assert_eq!(bob.active_session, host.active_session, "parler_join joined the session room");
    }

    #[tokio::test]
    async fn join_tool_on_a_gated_session_is_pending_not_an_error() {
        // #109: an approval-gated session key via parler_join returns success-in-progress, never an
        // error (the request is actually filed — the old parler_join.join() bailed here).
        let hub = start_hub().await;
        let mut host = state(&hub, "host").await;
        let opened = open_session(&mut host, Some("secret"), None, None, None, true).await.unwrap();
        let key = key_of(&opened);

        let mut bob = state(&hub, "bob").await;
        let out = call_session_tool(&mut bob, "parler_join", &json!({ "code": key })).await;
        assert!(out.is_ok(), "a pending join must be a normal result, not an error");
        let out = out.unwrap();
        assert!(out.contains("waiting for the host"), "reads as success-in-progress: {out}");
        assert!(bob.active_session.is_none(), "not in the session until approved");
        // The request really was filed — the host sees it pending.
        let reqs = host.agent.join_requests(host.active_session.as_ref().unwrap()).await.unwrap();
        assert!(reqs.iter().any(|r| r.agent == bob.agent.id), "the join request was recorded");
    }

    #[tokio::test]
    async fn approve_join_resolves_a_pending_name_like_send_does() {
        // #107: the owner sees the joiner's *name* in the pending notice, so approve/deny must accept
        // a unique name (not just the 56-char id). Ambiguity errors with candidates; a bad name errors.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let opened = open_session(&mut alice, Some("secret plan"), None, None, None, true).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap(); // pending

        // A name that nobody is waiting under errors actionably (points at parler_join_requests).
        let miss = call_session_tool(&mut alice, "parler_approve_join", &json!({ "agent": "nobody" })).await;
        assert!(miss.is_err(), "an unknown name must not silently pass through to the hub");

        // Approving by the pending name (case-insensitive) admits bob, exactly like approving by id.
        let approved = call_session_tool(&mut alice, "parler_approve_join", &json!({ "agent": "BOB" }))
            .await
            .unwrap();
        assert!(approved.contains("approved"), "approve-by-name admits the joiner: {approved}");
        let joined = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        assert!(joined.contains("secret plan"), "the name-approved joiner gets the context: {joined}");
    }

    #[tokio::test]
    async fn denied_joiner_cannot_enter_or_reapply() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut eve = state(&hub, "eve").await;

        let opened = open_session(&mut alice, Some("seed"), None, None, None, true).await.unwrap();
        let key = key_of(&opened);
        join_session(&mut eve, &key, Backlog::Recent, None).await.unwrap(); // pending

        let denied = call_session_tool(&mut alice, "parler_deny_join", &json!({ "agent": eve.agent.id }))
            .await
            .unwrap();
        assert!(denied.contains("denied"));

        // The denial is terminal — eve's retry errors instead of letting her in.
        assert!(join_session(&mut eve, &key, Backlog::Recent, None).await.is_err());
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
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();

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

    #[tokio::test]
    async fn test_mcp_push_handoff_apply_e2e() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        let inv = alice.agent.invite(RoomKind::Channel, Some("dev".into()), None, None).await.unwrap();
        bob.agent.join(&inv.code).await.unwrap();

        // Set up temporary directories for repositories
        let alice_dir = tempfile::tempdir().unwrap();
        let bob_dir = tempfile::tempdir().unwrap();

        let init_git_repo = |path: &std::path::Path| {
            let run = |args: &[&str]| {
                let status = std::process::Command::new("git")
                    .current_dir(path)
                    .args(args)
                    .status()
                    .unwrap();
                assert!(status.success());
            };
            run(&["init", "--initial-branch=main"]);
            run(&["config", "user.name", "Test User"]);
            run(&["config", "user.email", "test@example.com"]);
        };

        init_git_repo(alice_dir.path());
        init_git_repo(bob_dir.path());

        // Create a commit in Alice's repo
        std::fs::write(alice_dir.path().join("file.txt"), "hello").unwrap();
        let commit_sha = {
            let run = |args: &[&str]| {
                let out = std::process::Command::new("git")
                    .current_dir(alice_dir.path())
                    .args(args)
                    .output()
                    .unwrap();
                assert!(out.status.success());
                out
            };
            run(&["add", "file.txt"]);
            run(&["commit", "-m", "first commit"]);
            let sha_out = run(&["rev-parse", "HEAD"]);
            String::from_utf8(sha_out.stdout).unwrap().trim().to_string()
        };

        // Alice pushes the git bundle via MCP tool
        let push_args = json!({
            "room": inv.room,
            "repo": alice_dir.path().to_str().unwrap(),
            "gitref": "HEAD",
        });
        let push_res = call_tool(&mut alice.agent, "parler_push", &push_args).await.unwrap();
        assert!(push_res.contains("pushed git bundle"));

        // Extract blob ID from response
        let blob_idx = push_res.find("blob=").unwrap() + 5;
        let rest = &push_res[blob_idx..];
        let end_idx = rest.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(rest.len());
        let blob_id = &rest[..end_idx];

        // Bob fetches the bundle via parler_fetch
        let bundle_out = bob_dir.path().join("fetched.bundle");
        let fetch_args = json!({
            "id": blob_id,
            "out": bundle_out.to_str().unwrap(),
        });
        let fetch_res = call_tool(&mut bob.agent, "parler_fetch", &fetch_args).await.unwrap();
        assert!(fetch_res.contains("wrote"));
        assert!(bundle_out.exists());

        // Bob applies the git bundle via parler_apply
        let apply_args = json!({
            "blob": blob_id,
            "path": bob_dir.path().to_str().unwrap(),
        });
        let apply_res = call_tool(&mut bob.agent, "parler_apply", &apply_args).await.unwrap();
        assert!(apply_res.contains("imported into refs/parler/"));

        // Verify Bob's repo tip matches Alice's commit SHA
        let refname = format!("refs/parler/{}", crate::short(blob_id));
        let verify_out = std::process::Command::new("git")
            .current_dir(bob_dir.path())
            .args(["rev-parse", &refname])
            .output()
            .unwrap();
        assert!(verify_out.status.success());
        let bob_sha = String::from_utf8(verify_out.stdout).unwrap().trim().to_string();
        assert_eq!(bob_sha, commit_sha);
    }

    #[test]
    fn test_no_phantom_tool_references() {
        let specs = tool_specs();
        let mut valid_tools: std::collections::HashSet<String> = specs
            .iter()
            .map(|t| t.get("name").unwrap().as_str().unwrap().to_string())
            .collect();
        
        valid_tools.insert("parler_open_session".to_string());
        valid_tools.insert("parler_join_session".to_string());
        valid_tools.insert("parler_close_session".to_string());
        valid_tools.insert("parler_join_requests".to_string());
        valid_tools.insert("parler_approve_join".to_string());
        valid_tools.insert("parler_deny_join".to_string());
        valid_tools.insert("parler_watch_session".to_string());
        valid_tools.insert("parler_session_handoff".to_string());
        valid_tools.insert("parler_consolidate_session".to_string());
        
        let mcp_src = std::fs::read_to_string("src/mcp.rs").unwrap_or_else(|_| std::fs::read_to_string("crates/parler-cli/src/mcp.rs").unwrap());
        let lib_src = std::fs::read_to_string("src/lib.rs").unwrap_or_else(|_| std::fs::read_to_string("crates/parler-cli/src/lib.rs").unwrap());
        
        let mut references = std::collections::HashSet::new();
        for src in &[mcp_src, lib_src] {
            let mut s = src.as_str();
            while let Some(idx) = s.find("parler_") {
                let s_sub = &s[idx..];
                let len = s_sub.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').count();
                let tool_ref = &s_sub[..len];
                if tool_ref.len() > 7 {
                    references.insert(tool_ref.to_string());
                }
                s = &s_sub[len.max(1)..];
            }
        }
        
        let mut invalid = Vec::new();
        for r in &references {
            if r == "parler_auth" || r == "parler_connector" || r == "parler_protocol" || r == "parler_hub" {
                continue;
            }
            if r == "parler_doctor_probe" {
                continue;
            }
            if !valid_tools.contains(r) {
                invalid.push(r.clone());
            }
        }
        
        assert!(
            invalid.is_empty(),
            "Found references to parler_* tools/names that are not in the valid tools list: {:?}",
            invalid
        );
    }

    // ---- #99: one env/config precedence rule (explicit env > saved config > default) -------------

    /// Build a config with the given saved hub/name/role (identity is a throwaway in-memory one).
    fn saved(hub: &str, name: &str, role: Option<&str>) -> Config {
        Config::create(hub.to_string(), name.to_string(), role.map(String::from)).unwrap()
    }

    #[test]
    fn env_hub_overrides_saved_config() {
        // An existing config.json + PARLER_HUB=ws://other ⇒ the resolved hub is ws://other, so
        // launching `parler mcp` (or any CLI command) dials the env hub, not the saved one. The
        // identity is untouched.
        let cfg = saved("wss://parler-hub.fly.dev", "codex", None);
        let saved_id = cfg.identity.id.clone();
        let mut notes = Vec::new();
        let out = apply_overrides(cfg, Some("ws://other:7070".into()), None, None, &mut |l| notes.push(l));
        assert_eq!(out.hub_url, "ws://other:7070", "env PARLER_HUB must win over saved config");
        assert_eq!(out.identity.id, saved_id, "identity (id/seed) is never rewritten by an env override");
        assert_eq!(out.name, "codex", "name untouched when PARLER_NAME is unset");
        assert!(notes.iter().any(|n| n.contains("PARLER_HUB overrides")), "override is announced once: {notes:?}");
    }

    #[test]
    fn env_name_and_role_take_effect_over_saved_config() {
        // PARLER_NAME / PARLER_ROLE env changes take effect (so re-wiring via `parler connect`,
        // which rewrites the env block, genuinely renames/re-roles the agent on next launch).
        let cfg = saved("ws://h:1", "old-name", Some("planner"));
        let mut notes = Vec::new();
        let out = apply_overrides(cfg, None, Some("new-name".into()), Some("reviewer".into()), &mut |l| notes.push(l));
        assert_eq!(out.name, "new-name");
        assert_eq!(out.role.as_deref(), Some("reviewer"));
        assert_eq!(out.hub_url, "ws://h:1", "hub untouched when PARLER_HUB is unset");
        assert_eq!(notes.len(), 2, "one note per changed field: {notes:?}");
    }

    #[test]
    fn saved_config_wins_when_env_absent_and_matching_env_is_silent() {
        // Absent env ⇒ the saved values stand (default precedence). And an env that merely *matches*
        // the saved value is not announced (no spurious "override" line, no false state-change).
        let cfg = saved("ws://h:1", "keep", Some("planner"));
        let mut notes = Vec::new();
        let out = apply_overrides(cfg, Some("ws://h:1".into()), Some("keep".into()), Some("planner".into()), &mut |l| notes.push(l));
        assert_eq!(out.hub_url, "ws://h:1");
        assert_eq!(out.name, "keep");
        assert_eq!(out.role.as_deref(), Some("planner"));
        assert!(notes.is_empty(), "matching env is not a state change: {notes:?}");
    }

    // ---- #102: MCP client retries a down hub instead of dying instantly --------------------------

    #[test]
    fn start_hub_hint_names_the_local_start_command() {
        // A loopback hub that's down has one obvious fix — the exact command belongs in the hint the
        // MCP retry logs and `parler doctor` echoes, so an agent-before-hub launch is never a silent
        // dead server (issue #102).
        assert!(start_hub_hint("ws://127.0.0.1:7070").contains("parler hub --local"));
        assert!(start_hub_hint("ws://localhost:7070").contains("parler hub --local"));
        // A remote hub isn't something we tell the user to `parler hub --local` — different guidance.
        assert!(!start_hub_hint("wss://parler-hub.fly.dev").contains("parler hub --local"));
    }

    #[tokio::test]
    async fn connect_with_retry_rides_out_a_hub_that_starts_late() {
        // Agent launches BEFORE the hub is up: bind the listener but don't serve yet, so the first
        // dials are refused; then start serving. `connect_with_retry` must ride the short window and
        // succeed rather than die on the first refusal (the core #102 acceptance).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hub = format!("ws://{addr}");
        // Start serving after a delay that spans a couple of retry intervals.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let store = parler_hub::Store::open(None).unwrap();
            let state = Arc::new(parler_hub::HubState::new(
                store, "parler://test".into(), "late hub".into(), parler_hub::HubMode::Private,
            ));
            let _ = parler_hub::serve(listener, state).await;
        });
        let cfg = Config::create(hub, "late-joiner".to_string(), None).unwrap();
        let agent = connect_with_retry(&cfg).await;
        assert!(agent.is_ok(), "must connect once the late hub comes up, not die on the first refusal");
    }
}

