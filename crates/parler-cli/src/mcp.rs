//! `parler mcp` — a minimal MCP (Model Context Protocol) server over stdio.
//!
//! MCP is JSON-RPC 2.0 with newline-delimited messages on stdio. We implement just the methods a
//! host needs — `initialize`, `tools/list`, `tools/call`, `ping` — and map each `parler_*` tool
//! onto the same [`MeshAgent`] the CLI uses. Hand-rolled on purpose: it keeps the dependency
//! surface tiny and gives exact control over the wire, which matters more than an SDK here.

use anyhow::{anyhow, bail, Result};
use parler_connector::{BundleMeta, Config, JoinOutcome, MeshAgent};
use parler_protocol::{
    AgentSkill, BundleRef, DiscoverScope, FileRef, HandoffRef, RoomKind, StoredMessage, Target,
    Visibility,
};
use serde_json::{json, Value};
use std::collections::HashMap;
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
    /// Per-room pre-approval allowlists (room → joiner names/ids the owner opted to auto-admit). A
    /// pending joiner matching its room's list is approved automatically the moment the owner's agent
    /// next surfaces requests (recv notice or `parler_join_requests`) — the Tailscale
    /// pre-approved-key pattern (#108), trading approval latency for the owner's explicit up-front
    /// trust. In-memory only: after an MCP restart the list is gone and listed joiners fall back to
    /// manual approval — a safe degradation that never over-admits.
    preapprovals: HashMap<String, Vec<String>>,
}

impl McpState {
    fn new(agent: MeshAgent) -> Self {
        McpState { agent, active_session: None, preapprovals: HashMap::new() }
    }
}

/// Opt out of per-workspace identity scoping: set this (truthy) to pin **one** identity for a
/// `PARLER_HOME` across every workspace, for a user who deliberately wants a single agent regardless
/// of where an agent-facing `parler` command is launched. Absent it, each workspace gets its own
/// identity (the default, so a live session shows every agent, not a single collapsed member).
const SHARED_IDENTITY_ENV: &str = "PARLER_SHARED_IDENTITY";
const AGENT_SESSION_ENV: &str = "PARLER_AGENT_SESSION";

/// The un-scoped home whose `mcp.log` breadcrumb we keep writing to even after scoping `PARLER_HOME`
/// down into a per-workspace subdir — so `parler doctor` (run outside the workspace) still finds the
/// log where it has always lived. Set once by [`scope_identity_to_workspace`].
static LOG_ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Namespace an agent process's identity by its **workspace** and, when needed, a stable host-session
/// id. Two agents launched on one machine then get
/// distinct hub identities instead of collapsing onto one saved `config.json` — the bug where a
/// 3-agent session shows a single member because every process re-registered the same id.
///
/// The seam is a single `PARLER_HOME` redirect done *before* any identity resolution, so the identity,
/// the active-session pointer, and everything else keyed off `home_dir()` follow consistently — no
/// scattered path plumbing. Restart-stable: the same workspace/session pair re-derives the same home,
/// so the agent keeps its id across restarts. MCP hosts that don't expose a session id retain the
/// existing per-workspace behavior. Conductor already gives every agent an isolated workspace, so
/// its automatic Codex/Claude thread id is deliberately omitted: the interactive agent and a
/// Conductor Run-script worker must resolve the same identity/active-session files. An explicit
/// `PARLER_AGENT_SESSION` still opts into a further split.
///
/// It can only ever *add* isolation, never regress an existing flat setup: it no-ops (keeping the one
/// flat identity) when [`SHARED_IDENTITY_ENV`] is set or the working directory can't be determined.
/// `parler connect`'s per-host `PARLER_HOME` still applies — this just subdivides it per workspace, so
/// two windows of the same wired host no longer share one identity.
pub(crate) fn scope_identity_to_workspace() {
    if env_flag(SHARED_IDENTITY_ENV) {
        return;
    }
    let Some(key) = workspace_key() else {
        return; // No usable CWD → leave the flat home (no worse than before).
    };
    let base = parler_connector::home_dir();
    // A legacy flat identity may carry the user's chosen local/team hub. Inherit only that routing
    // preference into a brand-new scoped identity; never copy the seed (the whole point is a new id)
    // or its stale display name (the fresh id gets its own unique default handle).
    let env_hub = std::env::var("PARLER_HUB").ok().filter(|value| !value.is_empty());
    let base_hub = if env_hub.is_none() {
        Config::load().ok().map(|cfg| cfg.hub_url)
    } else {
        None
    };
    // Pin the breadcrumb log to the un-scoped home so `parler doctor` keeps finding it in one place,
    // instead of it fragmenting into every per-workspace subdir.
    let _ = LOG_ROOT.set(base.join("mcp.log"));
    std::env::set_var("PARLER_HOME", workspace_home(&base, &key));
    if let Some(hub) = bootstrap_hub(Config::exists(), env_hub.as_deref(), base_hub.as_deref()) {
        std::env::set_var("PARLER_HUB", hub);
    }
}

fn bootstrap_hub(scoped_exists: bool, env_hub: Option<&str>, base_hub: Option<&str>) -> Option<String> {
    if scoped_exists || env_hub.is_some() {
        None
    } else {
        base_hub.map(str::to_string)
    }
}

/// The stable key identifying this agent instance: canonical workspace plus an optional host session
/// id. Codex exposes a thread id to shell commands; other hosts can set `PARLER_AGENT_SESSION`
/// explicitly. Values are hashed into the home name and never persisted or sent.
fn workspace_key() -> Option<String> {
    let conductor_workspace = std::env::var("CONDUCTOR_WORKSPACE_PATH")
        .ok()
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from);
    let path = match &conductor_workspace {
        Some(path) => path.clone(),
        None => std::env::current_dir().ok()?,
    };
    let canon = std::fs::canonicalize(&path).unwrap_or(path);
    let workspace = canon.to_string_lossy();
    let explicit = std::env::var(AGENT_SESSION_ENV).ok().filter(|value| !value.is_empty());
    let host_session = ["CODEX_THREAD_ID", "CLAUDE_CODE_SESSION_ID"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()));
    let session = scope_session(conductor_workspace.is_some(), explicit, host_session);
    Some(identity_scope_key(&workspace, session.as_deref()))
}

fn scope_session(
    conductor_workspace: bool,
    explicit: Option<String>,
    host_session: Option<String>,
) -> Option<String> {
    explicit.or_else(|| (!conductor_workspace).then_some(host_session).flatten())
}

fn identity_scope_key(workspace: &str, session: Option<&str>) -> String {
    match session {
        Some(session) => format!("{workspace}\0{session}"),
        None => workspace.to_string(),
    }
}

/// The per-workspace identity home: `<base>/ws/<hash(key)>`. A stable 64-bit FNV-1a hash keeps the
/// path short and — unlike `std`'s `DefaultHasher`, whose output isn't guaranteed stable across runs
/// or std versions — maps a given workspace to the *same* home every launch, so the identity persists
/// across restarts.
fn workspace_home(base: &std::path::Path, key: &str) -> std::path::PathBuf {
    base.join("ws").join(fnv1a_hex(key))
}

/// 64-bit FNV-1a as 16 lowercase hex chars — a tiny, dependency-free, deterministic hash (we only need
/// a stable short directory name from a path, not cryptographic strength).
fn fnv1a_hex(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
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
    let mut state = McpState::new(agent);

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
    // Clean shutdown (stdin EOF): best-effort commit any deferred acks left by auto-pull-on-send, so
    // the next MCP start doesn't re-deliver a batch we already returned to the host (#85).
    state.agent.flush_acks().await;
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
///   - `PARLER_NAME` — display name (default: a fun `adjective-animal-<tag>` handle)
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
    let explicit_name = std::env::var("PARLER_NAME").ok().filter(|s| !s.is_empty());
    // A placeholder base for `Config::create`; overwritten below with a fun handle when the name
    // wasn't set explicitly.
    let base = explicit_name.clone().unwrap_or_else(|| "agent".into());
    let role = std::env::var("PARLER_ROLE").ok().filter(|s| !s.is_empty());
    let mut cfg = Config::create(hub, base, role)?;
    // Fun-and-unique-by-default (issue #103): when the name wasn't set explicitly (bare `parler mcp`),
    // give the agent a playful `adjective-animal-<tag>` handle seeded on its freshly-minted, unique
    // agent id — so it reads like a character instead of `$USER`, and two fresh identities on the same
    // hub don't collide. An explicit PARLER_NAME (e.g. what `parler connect` wires) is honored verbatim.
    if explicit_name.is_none() {
        cfg.name = crate::names::fun_name(&cfg.identity.id);
    }
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

/// Where the MCP connection breadcrumb log lives (`~/.parler/mcp.log`). Pinned to the un-scoped home
/// captured by [`scope_identity_to_workspace`] when workspace scoping is active, so it doesn't
/// fragment into per-workspace subdirs and `parler doctor` keeps finding it in one place; otherwise
/// the plain `home_dir()` default.
fn mcp_log_path() -> std::path::PathBuf {
    LOG_ROOT.get().cloned().unwrap_or_else(|| parler_connector::home_dir().join("mcp.log"))
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
                | "parler_task" | "parler_join_requests" | "parler_approve_join" | "parler_deny_join"
                | "parler_watch_session" | "parler_bring" | "parler_fetch" => {
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
                        "You are joining a Parler Protocol collaborative session.\n\
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
                "invite ready — {kind} room '{room}'.\ncode: {code}\n\
                 The other agent calls parler_join with the portable code (carries this hub, so it \
                 works even if that agent's default hub differs):  {code}@{hub}\nlink: {url}",
                kind = inv.kind.as_str(),
                room = inv.room,
                code = inv.code,
                hub = agent.hub_url,
                url = inv.url,
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
        "parler_send_file" => {
            let target = select_target(s("room"), s("to"), s("service"))?
                .ok_or_else(|| anyhow!("provide exactly one of room / to / service"))?;
            let target = crate::resolve_target(agent, target).await?;
            let path = s("path").ok_or_else(|| anyhow!("missing 'path'"))?;
            let bytes = std::fs::read(&path).map_err(|e| anyhow!("cannot read '{path}': {e}"))?;
            let name = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow!("'{path}' has no file name to send"))?
                .to_string();
            let media_type = crate::guess_media_type(&name);
            let r = agent.send_file(target, &name, &bytes, media_type, s("note")).await?;
            Ok(format!(
                "sent file '{name}' to '{}' (seq {}, {} bytes).\n\
                 The peer runs (MCP): parler_fetch (auto-finds it) — or parler_fetch id={}\n\
                 The peer runs (CLI): parler fetch {} -o {name}",
                r.room,
                r.seq,
                bytes.len(),
                r.blob_id,
                r.blob_id,
            ))
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
/// Whether `s` is a content-addressed blob id — a lowercase-hex SHA-256 (exactly 64 hex chars).
/// Lets `parler_fetch` tell a real id from a filename/path the caller passed instead (a hint to
/// resolve against the room's recent transfers).
fn looks_like_blob_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Find a file (or code bundle) `room` recently shared, so `parler_fetch` needs no pasted blob id.
///
/// Pages the room's history with **pure `since` re-reads** (they never advance a delivery cursor, so
/// this can't perturb `parler_recv`), collecting every `com.parler.file`/`com.parler.bundle`
/// reference, and returns the **most recent** one — filtered to a name/basename substring match when
/// `name_hint` is given. Returns `(blob_id, suggested_out_name)`.
async fn resolve_recent_blob(
    agent: &mut MeshAgent,
    room: &str,
    name_hint: Option<&str>,
) -> Result<(String, String)> {
    // Match on the basename, case-insensitively — the sender only ever stores a bare basename, and a
    // caller may paste a full path (e.g. the one the host showed them).
    let hint = name_hint.map(|h| {
        std::path::Path::new(h)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(h)
            .to_ascii_lowercase()
    });
    let mut best: Option<(i64, String, String)> = None; // (seq, blob, out_name)
    let mut since = 0i64;
    loop {
        // 1000 is the hub's per-pull ceiling; page until a short batch drains the history.
        let (msgs, _) = agent.pull(room, Some(since), Some(1000)).await?;
        if msgs.is_empty() {
            break;
        }
        let batch = msgs.len();
        for m in &msgs {
            since = since.max(m.seq);
            for p in &m.parts {
                let (blob, out_name, match_text) = if let Some(f) = FileRef::from_part(p) {
                    (f.blob, f.name.clone(), f.name)
                } else if let Some(b) = BundleRef::from_part(p) {
                    // A bundle carries no filename; save it as `<short>.bundle`, but let a hint match
                    // its summary (the commit subject) so "fetch the auth bundle" can resolve.
                    let out = format!("{}.bundle", crate::short(&b.blob));
                    let text = b.summary.clone().unwrap_or_else(|| out.clone());
                    (b.blob, out, text)
                } else {
                    continue;
                };
                if let Some(h) = &hint {
                    if !match_text.to_ascii_lowercase().contains(h.as_str()) {
                        continue;
                    }
                }
                // Keep the highest-seq match (the most recent transfer).
                if best.as_ref().is_none_or(|(s, ..)| m.seq >= *s) {
                    best = Some((m.seq, blob, out_name));
                }
            }
        }
        if batch < 1000 {
            break;
        }
    }
    match best {
        Some((_, blob, out_name)) => Ok((blob, out_name)),
        None => match name_hint {
            Some(h) => bail!(
                "no shared file matching '{h}' found in '{room}' — run parler_recv to see what's been \
                 sent, or pass the exact id (parler_fetch id=<blob>)"
            ),
            None => bail!(
                "no file has been shared in '{room}' yet — run parler_recv to check for one, or pass \
                 id=<blob>"
            ),
        },
    }
}

async fn call_session_tool(state: &mut McpState, name: &str, args: &Value) -> Result<String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
    let u32opt = |k: &str| args.get(k).and_then(Value::as_u64).map(|x| x as u32);

    match name {
        "parler_open_session" => {
            let context = s("context");
            // Approval defaults ON: a session is a live conversation, so the host vets each joiner
            // before they can read it. Pass approval=false to revert to open (paste-and-join) keys.
            let approval = args.get("approval").and_then(Value::as_bool).unwrap_or(true);
            let preapprove = parse_name_list(args.get("preapprove"));
            let mut out = open_session(
                state,
                context.as_deref(),
                s("topic"),
                args.get("ttl_secs").and_then(Value::as_u64),
                u32opt("max_uses"),
                approval,
            )
            .await?;
            // Record the owner's pre-approval allowlist for the room just opened, so a listed joiner
            // is auto-admitted on the owner's next poll (see auto_approve_preapproved). Only meaningful
            // with the gate on — an open key already admits everyone. The signature of open_session is
            // left untouched; the tool layer owns this wiring end to end.
            let listed: Vec<String> =
                preapprove.iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
            if approval && !listed.is_empty() {
                if let Some(room) = state.active_session.clone() {
                    out.push_str(&format!(
                        "\nPre-approved (auto-admitted, no prompt): {}. Anyone else still needs your approval.",
                        listed.join(", ")
                    ));
                    state.preapprovals.insert(room, listed);
                }
            }
            Ok(out)
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
            let key = s("code").ok_or_else(|| anyhow!("missing 'code'"))?;
            // Accept a portable `<code>@<hub>`: redeem it when it names this hub, else say which hub
            // to relaunch on (a single-hub MCP agent can't cross hubs — #99) instead of a cryptic error.
            let hub = state.agent.hub_url.clone();
            let code = portable_code_for_hub(&key, &hub)?;
            let wait_secs = args.get("wait_secs").and_then(Value::as_u64).filter(|w| *w > 0);
            match state.agent.redeem(&code).await.map_err(|e| explain_unknown_code_mcp(e, &hub))? {
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
            // Admit pre-approved joiners first, then list whoever still needs a manual decision.
            let admitted = auto_approve_preapproved(state, &room).await;
            let auto_line = if admitted.is_empty() {
                String::new()
            } else {
                format!("✓ auto-admitted pre-approved: {}\n", admitted.join(", "))
            };
            let reqs = state.agent.join_requests(&room).await?;
            if reqs.is_empty() {
                return Ok(if admitted.is_empty() {
                    format!("(no agents waiting to join '{room}')")
                } else {
                    format!("{}(no other agents waiting to join '{room}')", auto_line)
                });
            }
            Ok(format!(
                "{}{}",
                auto_line,
                reqs.iter()
                    .map(|r| {
                        let role = r.role.as_deref().map(|x| format!(" ({x})")).unwrap_or_default();
                        format!(
                            "• {}{role} [{}] — approve: parler_approve_join agent={} | reject: parler_deny_join agent={}",
                            r.name, r.agent, r.agent, r.agent
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
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
            // wait_secs to long-poll. Our own just-sent message is filtered out; the pull records an
            // ack so this batch is committed by the next pull and not re-delivered later in the
            // session. The pull is capped (AUTOPULL_LIMIT) so a reply flood can't balloon the send
            // result — the remainder stays unread for the next parler_recv (a limited pull only
            // advances the cursor through what it returned). We deliberately do NOT commit_reads here:
            // one extra round trip per send isn't worth it; the ack (#85) rides the next real pull, and
            // a batch seen only via auto-pull before an MCP restart is re-read after — at-least-once.
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
                "✓ handed off to {whom} in '{room}' (seq {seq}). A live host hook / `parler work` \
                 worker acts automatically; otherwise they'll see 'HANDOFF TO YOU' on parler_recv."
            ))
        }
        "parler_task" => {
            let status_str = s("status").ok_or_else(|| anyhow!("missing 'status'"))?;
            let status = parler_protocol::TaskStatus::parse(&status_str).ok_or_else(|| {
                anyhow!("unknown status '{status_str}' — use one of: {}", parler_protocol::TaskStatus::ALL.join(" | "))
            })?;
            let target = match select_target(s("room"), s("to"), s("service"))? {
                Some(t) => t,
                None => match state.active_session.clone() {
                    Some(room) => Target::Room { room },
                    None => bail!("provide one of room / to / service, or open/join a session first"),
                },
            };
            let target = crate::resolve_target(&mut state.agent, target).await?;
            let task = parler_protocol::TaskRef {
                status,
                task: s("task"),
                note: s("note"),
                result: s("result"),
                tokens: args.get("tokens").and_then(Value::as_u64),
                elapsed_ms: args.get("elapsed_ms").and_then(Value::as_u64),
            };
            let (_id, seq, room) = state.agent.send(target, vec![task.to_part()], None, None).await?;
            let id = task.task.map(|i| format!(" ({i})")).unwrap_or_default();
            Ok(format!("{} task {}{id} posted to '{room}' (seq {seq})", status.marker(), status.label()))
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
            // A one-shot MCP recv does a single pull, so the deferred ack (#85) would die with the
            // process — commit the cursor-mode batch now it's rendered, so the next recv (even a cold
            // start after an MCP restart) returns only newer messages. A `since` re-read never commits.
            if !msgs.is_empty() && since.is_none() {
                state.agent.commit_reads(&room).await?;
            }
            // Surface any pending join requests so a host sees the accept/reject choice inline, even
            // when there are no new messages.
            if let Some(notice) = pending_join_notice(state, &room).await {
                out.push_str(&notice);
            }
            Ok(out)
        }
        "parler_fetch" => {
            // A blob id is a 64-char lowercase-hex SHA-256. If the caller passes one, download it
            // directly. Otherwise (no `id`, or a filename/path was passed instead) find the file the
            // room recently shared — so an agent asked to "fetch the file" just works, without a human
            // pasting the id. `out` (or `-o`) always wins; else default to the file's own name.
            let raw_id = s("id");
            let explicit = raw_id.clone().filter(|v| looks_like_blob_id(v));
            let (id, suggested) = match explicit {
                Some(blob) => (blob, None),
                None => {
                    let room = s("room").or_else(|| state.active_session.clone()).ok_or_else(|| {
                        anyhow!(
                            "no blob id, and no active session to search — pass id=<blob>, or open/join \
                             a session (or pass room=…) so I can find the shared file"
                        )
                    })?;
                    // `name` is the explicit hint; a non-id `id` (e.g. a pasted path) is a fallback hint.
                    let name_hint = s("name").or(raw_id);
                    let (blob, name) =
                        resolve_recent_blob(&mut state.agent, &room, name_hint.as_deref()).await?;
                    (blob, Some(name))
                }
            };
            let bytes = state.agent.fetch_blob(&id).await?;
            let out = s("out").or(suggested).unwrap_or_else(|| format!("{}.bundle", crate::short(&id)));
            let out_path = std::path::PathBuf::from(out);
            std::fs::write(&out_path, &bytes)?;
            let abs_out = std::fs::canonicalize(&out_path)
                .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default().join(&out_path));
            let abs_out_str = abs_out.to_string_lossy();
            Ok(format!(
                "wrote {} bytes to {} (if it's a git bundle, apply with: parler_apply blob={id})",
                bytes.len(),
                abs_out_str,
            ))
        }
        "parler_bring" => bring_second_opinion(state, args).await,
        other => bail!("unknown session tool: {other}"),
    }
}

/// `parler_bring`: get an independent second opinion from another agent (v1: codex) without
/// copy-paste. The review needs somewhere to land so the host reads it via `parler_recv`, so we
/// post it into the active session — opening one seeded with the context if there isn't one yet.
///
/// A real review is multi-minute, so we must **not** block this tool call (the host would time it
/// out). Instead we spawn the bundled `parler bring` detached — it runs the agent, posts the
/// review into the room, and exits — and return immediately. The context goes in over the child's
/// stdin (never argv, so a large recap can't overflow the command line) and a background task
/// reaps the child so it never lingers as a zombie.
async fn bring_second_opinion(state: &mut McpState, args: &Value) -> Result<String> {
    let agent = args
        .get("agent")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .unwrap_or("codex")
        .to_string();
    if !crate::bring::is_supported(&agent) {
        bail!(
            "don't know how to bring '{agent}'. Supported: {}",
            crate::bring::SUPPORTED_AGENTS.join(", ")
        );
    }
    let context = args
        .get("context")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| anyhow!("missing 'context' — what should {agent} give a second opinion on?"))?
        .to_string();

    // The review is delivered as a normal message, so it needs a room. Reuse the active session;
    // otherwise open one seeded with the same context (also gives the host a place to converse).
    let room = match state.active_session.clone() {
        Some(r) => r,
        None => {
            open_session(state, Some(&context), Some(format!("{agent}-review")), None, None, true).await?;
            state
                .active_session
                .clone()
                .ok_or_else(|| anyhow!("failed to open a session for the review"))?
        }
    };

    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("parler"));
    let mut child = tokio::process::Command::new(&exe)
        .arg("bring")
        .arg(&agent)
        .arg("--context-file")
        .arg("-") // read the recap from stdin
        .arg("--room")
        .arg(&room)
        .arg("--quiet")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow!("couldn't start `parler bring`: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let ctx = context.clone();
        tokio::spawn(async move {
            let _ = stdin.write_all(ctx.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    // Reap the child when it finishes so it doesn't become a zombie in this long-lived process.
    // We don't kill_on_drop: the review should outlive a dropped handle and run to completion.
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    Ok(format!(
        "Asked {agent} for a second opinion. It's reviewing now and will post its feedback into \
         session '{room}' within a few minutes — call parler_recv to read it when it lands. If it \
         fails (e.g. {agent} isn't installed or logged in), a ⚠ notice with the fix lands there \
         instead."
    ))
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
    // A ready-to-paste one-liner the host can drop straight into Slack/Discord: it adds the Parler Protocol
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
    // Mint the read-only WATCH code up front so the host already has it. The #1 confusion is pasting the
    // *join* KEY into the web/desktop viewer — that 401s and reads as "invalid or expired". Give the watch
    // code the same lifetime as the session key so it can't expire mid-session. Best-effort: an older hub
    // that can't mint one keeps the session open and falls back to the manual `parler_watch_session`.
    let watch_line = match state.agent.mint_watch_token(&room, Some(ttl_secs.unwrap_or(24 * 3600))).await {
        Ok((token, _)) => format!(
            "WATCH code — the user pastes THIS (not the KEY) into the web/desktop viewer to watch read-only: {token}\n"
        ),
        Err(_) => "parler_watch_session mints a read-only WATCH code for the web/desktop session viewer.\n".to_string(),
    };
    Ok(format!(
        "session open — room '{room}', now your active session.\n\
         you are '{me}' here — that's the name teammates see in this session.\n\
         KEY: {code}@{hub}\n\
         Give a teammate the KEY (they call parler_join_session) or this ready-to-run one-liner:\n    \
         {oneliner}\n\
         Either lands them in this same conversation, caught up — no copy-paste. The `@{hub}` on the \
         KEY carries this hub, so a teammate whose default hub differs still lands here.\n\
         {gate}\n\
         {watch_line}\
         Keep late joiners cheap: parler_remember key=\"session-digest\" room=\"{room}\" text=\"SESSION DIGEST: …\" (re-save to update).\n\
         link: {url}",
        me = state.agent.name,
        code = inv.code,
        hub = state.agent.hub_url,
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

/// Loose hub-URL equality: ignore the scheme (`ws`/`wss`/`http`/`https`) and any trailing slash, so
/// `wss://h`, `https://h`, and `h/` all compare equal. Enough to tell "same hub" from "different
/// hub" for a portable code — we're not routing on it, just deciding whether we can redeem locally.
fn same_hub(a: &str, b: &str) -> bool {
    fn norm(u: &str) -> String {
        u.split_once("://").map(|(_, rest)| rest).unwrap_or(u).trim_end_matches('/').to_ascii_lowercase()
    }
    norm(a) == norm(b)
}

/// Resolve a possibly-portable code (`<code>@<hub>`) for the single-hub MCP agent. The MCP server
/// dials exactly one hub for its whole life (#99), so — unlike the CLI's one-shot commands — it
/// can't transparently redeem on another hub without stranding every later `parler_send`/`_recv`.
/// So: a code carrying *this* hub (or no hub) yields the bare code to redeem; a code naming a
/// *different* hub fails with the exact fix (which hub to relaunch on) instead of the hub's cryptic
/// "invalid or unknown invite code".
fn portable_code_for_hub(key: &str, agent_hub: &str) -> Result<String> {
    let (code, hub) = crate::split_portable_key(key);
    if let Some(hub) = hub {
        if !same_hub(&hub, agent_hub) {
            bail!(
                "this invite is on hub {hub}, but your Parler MCP server is connected to {agent_hub}. \
                 Relaunch it with PARLER_HUB={hub} (e.g. `parler connect --hub {hub}`), then try again."
            );
        }
    }
    Ok(code)
}

/// A bare code the hub doesn't hold surfaces as "invalid or unknown invite code" — and for a
/// single-hub MCP agent the usual cause is that the code belongs to a *different* hub. Point the
/// agent at the fix (ask for the portable `<code>@<hub>`, or relaunch on that hub) rather than
/// leaving the raw dead-end. Other errors pass through untouched.
fn explain_unknown_code_mcp(err: anyhow::Error, agent_hub: &str) -> anyhow::Error {
    if err.to_string().contains("invalid or unknown invite code") {
        return anyhow!(
            "invalid or unknown invite code on hub {agent_hub}. If it was minted on a different hub, \
             ask for the portable form `<code>@<hub>`, or relaunch your Parler MCP server with \
             PARLER_HUB set to that hub."
        );
    }
    err
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
    // A portable key `<code>@<hub>` carries the hub that minted it. Redeem it here only if it names
    // this agent's hub; otherwise fail with the exact fix rather than a cryptic "unknown code".
    let hub = state.agent.hub_url.clone();
    let code = portable_code_for_hub(key, &hub)?;
    match state.agent.redeem(&code).await.map_err(|e| explain_unknown_code_mcp(e, &hub))? {
        JoinOutcome::Joined { room, .. } => enter_session(state, room, backlog).await,
        JoinOutcome::Pending { room } => match wait_for_approval(state, &code, wait_secs).await? {
            Some(room) => enter_session(state, room, backlog).await,
            None => Ok(pending_output(&room)),
        },
    }
}

/// Adopt a just-redeemed room as the active session and render the catch-up context. Shared by
/// `parler_join_session` and `parler_join` (#109) so a session key does the same thing through either
/// door — active session set, backlog digested, arrival announced.
async fn enter_session(state: &mut McpState, room: String, backlog: Backlog) -> Result<String> {
    // Pull the whole backlog (since=None) to seed the catch-up digest; the cursor advance is deferred
    // to an ack (#85), so commit_reads flushes it right after — otherwise a one-shot join exits with
    // the ack in memory and a later parler_recv re-delivers this whole backlog. We render a *digest*
    // of what we pulled — the committed cursor sits past all of it.
    let (msgs, _cursor) = state.agent.pull(&room, None, None).await?;
    state.agent.commit_reads(&room).await?;
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
    // Deterministic keyed fetch (#91): ask for the fact stored under the digest key directly, so FTS
    // ranking can't bury it. Against an older hub the `key` is ignored and this degrades to the old
    // BM25-by-sentinel behavior — the verification below still guards that fallback's false positives.
    let hits = agent
        .recall_keyed(SESSION_DIGEST_KEY, SESSION_DIGEST_SENTINEL, Some(room.to_string()), Some(1))
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

/// Parse a name-list tool argument an LLM may send either as a JSON array (`["bob","codex"]`) or a
/// single delimited string (`"bob, codex"`). Absent/empty entries are dropped.
fn parse_name_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::String(s)) => s
            .split([',', ' ', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Auto-admit any pending joiner the owner pre-approved for `room` (#108), returning the display
/// names admitted so the caller can tell the host it happened. A no-op — and zero hub round-trips —
/// when the room has no pre-approval list. Matching is by joiner name (case-insensitive) or exact id:
/// the owner names trusted peers up front (e.g. `"codex"`), and a full id also works. Security: this
/// only fires for names the owner explicitly listed at open time, so a leaked key still can't admit
/// anyone off the list; approval for everyone else is unchanged.
async fn auto_approve_preapproved(state: &mut McpState, room: &str) -> Vec<String> {
    let Some(allow) = state.preapprovals.get(room).cloned() else {
        return Vec::new();
    };
    // join_requests is owner-only; a refusal (non-owner) just yields nothing to admit.
    let Ok(reqs) = state.agent.join_requests(room).await else {
        return Vec::new();
    };
    let mut admitted = Vec::new();
    for r in reqs {
        let listed = allow.iter().any(|a| a.eq_ignore_ascii_case(&r.name) || a == &r.agent);
        if listed && state.agent.resolve_join(room, &r.agent, true).await.unwrap_or(false) {
            admitted.push(r.name);
        }
    }
    admitted
}

/// If the caller owns `room` and agents are waiting to join it, render an approval prompt to append
/// to a `parler_send`/`parler_recv` result — this is how the host is *shown* the accept/reject option
/// inline, instead of having to poll for it. Returns `None` for a non-owner (the `join_requests` call
/// is refused) or when nothing is pending.
async fn pending_join_notice(state: &mut McpState, room: &str) -> Option<String> {
    // First, silently admit anyone the owner pre-approved for this room (no-op when none is listed).
    let admitted = auto_approve_preapproved(state, room).await;
    let auto_line = if admitted.is_empty() {
        String::new()
    } else {
        format!("✓ auto-admitted pre-approved: {}\n", admitted.join(", "))
    };
    let reqs = state.agent.join_requests(room).await.ok()?;
    if reqs.is_empty() {
        // Nothing left to prompt on, but surface the auto-admit so it isn't silent.
        return (!admitted.is_empty()).then(|| format!("\n\n{}", auto_line.trim_end()));
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
        "\n\n{auto_line}⏳ {n} agent(s) asking to JOIN this session — your approval is required before they can \
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
    // Drop any pre-approval allowlist for this room — a later session reusing the same room id must
    // not inherit a stale auto-admit list.
    state.preapprovals.remove(&room);
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
            "Open a shared live session; returns a KEY to hand another agent so it joins already caught up, plus a read-only WATCH code the user pastes into the web/desktop viewer (do NOT paste the KEY there). `context` posts first — recap task/decisions/files/state. Joiners need your approval by default (confirm with the user). Becomes your active session (send/recv need no room).",
            json!({
                "context": { "type": "string", "description": "recap of the conversation/state to catch up joiners" },
                "topic": { "type": "string", "description": "short session name" },
                "approval": { "type": "boolean", "description": "require approval before a joiner is admitted (default true; false = open paste-and-join)" },
                "preapprove": { "type": "array", "items": { "type": "string" }, "description": "trusted names/ids to auto-admit with no prompt (e.g. [\"codex\"]); others still need approval, so a leaked key can't admit off-list" },
                "ttl_secs": { "type": "integer", "description": "key validity (default 24h)" },
                "max_uses": { "type": "integer", "description": "max joiners (default 50)" }
            }),
            &[],
        ),
        tool(
            "parler_join_session",
            "Join a session with a KEY. If approval is required you're held pending; pass wait_secs to wait for the host in this call. You get a context digest (seed + recent tail); backlog:\"full\" replays all. Becomes your active session (send/recv need no room).",
            json!({
                "key": { "type": "string", "description": "the session key or link you were handed" },
                "backlog": { "type": "string", "enum": ["recent", "full"], "description": "recent (default): seed + recent tail; full: whole backlog" },
                "wait_secs": { "type": "integer", "description": "approval-gated: seconds to wait for the host here (≤60); omit to return 'pending' now" }
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
            "Approve a pending joiner (from parler_join_requests) so they can read and participate. Defaults to active session. Confirm with the user first.",
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
            "Mint a read-only WATCH code so the user can watch this session live on the Parler Protocol website (/session). Owner-only, separate from the join key (the safe way to let a human view it). Defaults to active session; hand the code to the user.",
            json!({
                "room": { "type": "string", "description": "the session room (defaults to your active session)" },
                "ttl_secs": { "type": "integer", "description": "how long the watch code stays valid (default 1h)" }
            }),
            &[],
        ),
        tool(
            "parler_bring",
            "Get an independent second opinion from another AI agent (v1: codex) — no copy-paste. Posts its review into your active session (opening one if needed); call parler_recv to read it. Returns immediately; the review lands in a few minutes.",
            json!({
                "agent": { "type": "string", "description": "which agent to ask (default and v1-only: codex)" },
                "context": { "type": "string", "description": "what to review — recap of the code/decision and what you want a second opinion on" }
            }),
            &["context"],
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
            "Send conversational text; work a peer must execute belongs in parler_handoff. Returns waiting replies. Defaults to active session; else exactly one of room/to/service. For a later reply use parler_recv wait_secs — don't poll.",
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
            "Pull new messages since your cursor (advances it). Defaults to active session; or pass room. wait_secs long-polls that many seconds for a pushed reply (cheaper than re-calling). Long bodies truncate with a refetch hint; `since` re-reads a range in FULL. Batch is bounded — 'more waiting' means call again.",
            json!({
                "room": { "type": "string" },
                "since": { "type": "integer", "description": "read-only full replay from seq (no cursor advance)" },
                "limit": { "type": "integer", "description": "max messages this call (default bounded)" },
                "wait_secs": { "type": "integer", "description": "block up to N seconds for a pushed message when none waiting (max 60)" }
            }),
            &[],
        ),
        tool(
            "parler_handoff",
            "Assign work to another agent. A host hook or `parler work` executes it autonomously; otherwise their next recv says HANDOFF TO YOU. Defaults to active session or room/to/service. `for`: name/role; `bundle`: blob from parler_push.",
            json!({
                "next": { "type": "string", "description": "the instruction for the next agent to act on" },
                "summary": { "type": "string", "description": "recap of what you finished / current state, for context" },
                "for": { "type": "string", "description": "address by agent name or role (default: anyone in the room)" },
                "bundle": { "type": "string", "description": "blob id of a code bundle from parler_push" },
                "room": { "type": "string" },
                "to": { "type": "string" },
                "service": { "type": "string" }
            }),
            &["next"],
        ),
        tool(
            "parler_task",
            "Report task status (accepted|working|awaiting|done|failed|cancelled) over service-queue work; a signed done/failed is a receipt. Defaults to active session, or room/to/service. `task` correlates updates to one job; `result` is a done blob id.",
            json!({
                "status": { "type": "string", "enum": ["accepted", "working", "awaiting", "done", "failed", "cancelled"] },
                "task": { "type": "string", "description": "correlate updates to one job (the request's message id)" },
                "note": { "type": "string", "description": "one-liner; the question when awaiting" },
                "result": { "type": "string", "description": "blob id handed back on done" },
                "tokens": { "type": "integer", "description": "tokens used (receipts)" },
                "elapsed_ms": { "type": "integer", "description": "ms taken (receipts)" },
                "room": { "type": "string" },
                "to": { "type": "string" },
                "service": { "type": "string" }
            }),
            &["status"],
        ),
        tool(
            "parler_remember",
            "Save a fact. LOG reflex: after a decision, record what matters. Same key overwrites (idempotent); omit key to append. Reuse a small key set: status, strategy, progress, knowledge, session-digest.",
            json!({
                "text": { "type": "string" },
                "key": { "type": "string", "description": "stable key for idempotent state; omit to append a note" },
                "room": { "type": "string" },
                "embedding": { "type": "array", "items": { "type": "number" }, "description": "embedding vector (float32 array, must match hub dimension)" },
                "embedding_model": { "type": "string", "description": "which model produced the embedding (e.g. text-embedding-3-small)" }
            }),
            &["text"],
        ),
        tool(
            "parler_recall",
            "Recall saved facts. PLAN reflex: pull what you need before acting, not re-read history. BM25, or hybrid BM25+vector with an embedding. Query a key term or free text.",
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
            "parler_send_file",
            "Transfer a file (`path`) to a room/peer/service; the peer fetches it with parler_fetch.",
            json!({
                "path": { "type": "string" },
                "room": { "type": "string" },
                "to": { "type": "string" },
                "service": { "type": "string" },
                "note": { "type": "string" }
            }),
            &["path"],
        ),
        tool(
            "parler_fetch",
            "Download a shared file/bundle to disk (does NOT apply). Omit `id` for the latest file in \
             the session, or pass `name` to pick by filename; `id` (blob id) fetches an exact blob.",
            json!({
                "id": { "type": "string", "description": "blob id; omit to auto-find the latest file" },
                "name": { "type": "string", "description": "pick by filename when you have no id" },
                "room": { "type": "string", "description": "room to search (default: active session)" },
                "out": { "type": "string", "description": "output file (default: the file's name)" }
            }),
            &[],
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
        McpState::new(agent)
    }

    /// Like [`state`], but does **not** subscribe — exercises the long-poll path on a connection that
    /// holds no push subscription (the previously-degraded mode #90 fixes).
    async fn state_no_push(hub: &str, name: &str) -> McpState {
        let cfg = Config::create(hub.to_string(), name.to_string(), None).unwrap();
        let agent = MeshAgent::connect(&cfg).await.unwrap();
        McpState::new(agent)
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
    /// raised to 12,000 to keep a meaningful guardrail against large regressions. Adding the
    /// `parler_send_file` tool (arbitrary file transfer) grew it ~170 B to 12,169; ceiling → 12,400.
    /// Adding the `parler_bring` tool (one-line second opinion from another agent) grew it ~560 B to
    /// ~12,970; ceiling → 13,200 to keep ~20% headroom against real regressions. Adding the
    /// `parler_task` tool (task-lifecycle status/receipts, the ACP borrow) grew it ~750 B to ~13,946;
    /// ceiling → 14,200. A **description diet** (2026-07-10 audit) then re-tightened all 27 tools to
    /// 12,689 B — *below* the pre-`parler_task` baseline, so the new capability nets a reduction, not a
    /// cost; ceiling → 13,000 (a real cut from 13,200) to lock the savings against creep-back. Adding
    /// `parler_open_session`'s lean `preapprove` param (auto-admit trusted joiners, #108) grew it ~240 B
    /// to ~12,930; ceiling → 13,200 to restore headroom (still the pre-diet guardrail level). Making
    /// `parler_fetch` self-service (auto-find the latest shared file: `id` now optional, `name`/`room`
    /// params added so an agent asked to "fetch the file" needn't be handed a 64-char blob id) grew it
    /// ~300 B to ~13,237 — load-bearing schema, not description bloat (descriptions stayed under their
    /// own ceiling); ceiling → 13,500 to restore headroom.
    const TOOL_SPECS_BUDGET: usize = 13_500;
    /// Just the human-readable descriptions (the part the diet targets; schema scaffolding is
    /// load-bearing). Pre-diet 5,261 B → post-diet (P0.2) 4,304 B; P1.2 adds ~230 B of cheap-path
    /// steering (name-based `to`/`card`, compact discover/roster, `detail`) that earns its bytes.
    /// Still ~730 B under the pre-diet baseline. `parler_bring`'s description adds ~280 B; ceiling
    /// → 5,000 with headroom. `parler_task`'s lean description adds ~190 B to ~5,190; ceiling → 5,400.
    /// The 2026-07-10 description diet then cut all descriptions to 4,297 B (below even the P0.2
    /// post-diet baseline); ceiling → 4,600 (a real cut from 5,000) to hold the diet.
    const TOOL_DESC_BUDGET: usize = 4_600;
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
    /// bloating back up. Auto-minting the read-only WATCH code up front (so the host never pastes the
    /// join KEY into the viewer — that 401s) adds the 32-char token + a one-line label, ~200 B → ~815;
    /// ceiling → 900. Surfacing the agent's own name ("you are '<name>' here …") so the host can relay
    /// who's in the room adds ~70 B → ~885; ceiling → 960 to restore headroom.
    const OPEN_RESULT_BUDGET: usize = 960;

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

    // ---- per-workspace identity (two same-machine agents don't collapse into one member) ----------

    #[test]
    fn workspace_home_is_stable_per_workspace_and_distinct_across_them() {
        // The seam that fixes the collapse: each workspace maps to its *own* identity home under the
        // shared base, deterministically (same workspace → same home every launch, so the id is stable
        // across restarts) and distinctly (a different workspace → a different home → its own identity).
        let base = std::path::Path::new("/home/u/.parler/agents/claude-code");
        let a1 = workspace_home(base, "/work/proj-a");
        let a2 = workspace_home(base, "/work/proj-a");
        let b = workspace_home(base, "/work/proj-b");

        assert_eq!(a1, a2, "same workspace re-derives the same home → id persists across restarts");
        assert_ne!(a1, b, "a different workspace gets its own home → its own identity");
        assert!(a1.starts_with(base.join("ws")), "scoped under <base>/ws/");
        // The tag is the stable FNV-1a hash of the workspace key, not std's unstable DefaultHasher.
        assert_eq!(a1.file_name().unwrap().to_str().unwrap(), fnv1a_hex("/work/proj-a"));
        assert_eq!(fnv1a_hex("/work/proj-a").len(), 16, "16 hex chars");
        assert_ne!(fnv1a_hex("/work/proj-a"), fnv1a_hex("/work/proj-b"));

        // Two agent terminals in the same directory still split when their host provides a stable
        // session id; repeated commands in one terminal re-derive the same key.
        let terminal_a = identity_scope_key("/work/proj-a", Some("thread-a"));
        let terminal_a_again = identity_scope_key("/work/proj-a", Some("thread-a"));
        let terminal_b = identity_scope_key("/work/proj-a", Some("thread-b"));
        assert_eq!(terminal_a, terminal_a_again);
        assert_ne!(workspace_home(base, &terminal_a), workspace_home(base, &terminal_b));

        // Conductor's workspace is already the process boundary. Ignore its automatic thread id so
        // a Run-script `parler work` process (which has no CODEX_THREAD_ID) shares the interactive
        // agent's identity; an explicit override remains available for advanced multi-agent use.
        assert_eq!(scope_session(true, None, Some("thread-a".into())), None);
        assert_eq!(
            scope_session(true, Some("manual-split".into()), Some("thread-a".into())).as_deref(),
            Some("manual-split")
        );
        assert_eq!(scope_session(false, None, Some("thread-a".into())).as_deref(), Some("thread-a"));
    }

    #[test]
    fn fresh_scope_inherits_only_an_unoverridden_base_hub() {
        assert_eq!(bootstrap_hub(false, None, Some("ws://local")), Some("ws://local".into()));
        assert_eq!(bootstrap_hub(false, Some("wss://env"), Some("ws://local")), None);
        assert_eq!(bootstrap_hub(true, None, Some("ws://local")), None);
        assert_eq!(bootstrap_hub(false, None, None), None);
    }

    #[tokio::test]
    async fn two_same_machine_identities_show_as_two_members() {
        // End state the fix guarantees: two agents launched on one machine — which, before scoping,
        // shared one saved `config.json` and collapsed onto a *single* hub member (the reported bug) —
        // now carry the two distinct identities their two workspace homes mint, so a live session's
        // roster shows both. (`workspace_home_…` above proves the two homes are distinct; a fresh home
        // mints a fresh identity, exactly as these two Configs stand in for.)
        let hub = start_hub().await;
        let mut ws_a = state(&hub, "claude-code-tam").await; // same wired name…
        let mut ws_b = state(&hub, "claude-code-tam").await; // …distinct identity (distinct workspace)
        assert_ne!(
            ws_a.agent.id, ws_b.agent.id,
            "two workspaces = two identities, even under one wired PARLER_NAME"
        );

        // Workspace A opens a session (a multi-use channel key); workspace B joins it.
        let inv = ws_a.agent.invite(RoomKind::Channel, Some("tutor-project".into()), None, None).await.unwrap();
        ws_b.agent.join(&inv.code).await.unwrap();

        // The hub's roster — what the desktop app and the website `/session` viewer render — lists both,
        // keyed by their distinct ids, not one collapsed member.
        let roster = ws_a.agent.roster(&inv.room).await.unwrap();
        let ids: std::collections::HashSet<_> = roster.iter().map(|e| e.id.clone()).collect();
        assert_eq!(ids.len(), 2, "two distinct members, not one collapsed identity: {roster:?}");
        assert!(ids.contains(&ws_a.agent.id));
        assert!(ids.contains(&ws_b.agent.id));
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
            "tool specs {bytes} B exceed budget {TOOL_SPECS_BUDGET} B, trim descriptions"
        );
        assert!(
            desc_bytes <= TOOL_DESC_BUDGET,
            "tool descriptions {desc_bytes} B exceed budget {TOOL_DESC_BUDGET} B — keep them tight"
        );
    }

    /// The whitelist is the security boundary in front of a subprocess spawn: a non-whitelisted
    /// agent — including anything shell-shaped — must be rejected before any side effect (no
    /// session opened, nothing spawned).
    #[tokio::test]
    async fn bring_rejects_unknown_agent_before_any_side_effect() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        for bad in ["claude", "codex; rm -rf /", "../../bin/sh"] {
            let err = call_session_tool(
                &mut alice,
                "parler_bring",
                &json!({ "agent": bad, "context": "review this" }),
            )
            .await
            .expect_err("non-whitelisted agent must be rejected");
            assert!(err.to_string().contains("don't know how to bring"), "{bad}: {err}");
            assert!(alice.active_session.is_none(), "rejection must not open a session");
        }
    }

    /// Omitted/blank context is rejected up front — bring must never spawn a review with nothing
    /// to review (the failure would otherwise surface minutes later, detached).
    #[tokio::test]
    async fn bring_requires_context() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        for args in [json!({ "agent": "codex" }), json!({ "agent": "codex", "context": "   " })] {
            let err = call_session_tool(&mut alice, "parler_bring", &args)
                .await
                .expect_err("missing context must be rejected");
            assert!(err.to_string().contains("missing 'context'"), "{err}");
            assert!(alice.active_session.is_none(), "rejection must not open a session");
        }
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
    async fn join_surfaces_digest_via_keyed_fetch_past_bm25_decoys() {
        // #91: the digest is fetched by key, so it surfaces even when BM25 would rank a decoy above it
        // at limit 1 — the failure mode of the old sentinel-query heuristic.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), Some("plan".into()), None, None, false).await.unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);
        // The real digest (keyed, long body) plus short unkeyed decoys that out-rank it under BM25.
        let digest_text = format!("SESSION DIGEST: {}", "auth done, next is billing. ".repeat(20));
        alice.agent.remember(&digest_text, Some(SESSION_DIGEST_KEY.into()), Some(room.clone()), None, None).await.unwrap();
        for _ in 0..3 {
            alice.agent.remember("SESSION DIGEST", None, Some(room.clone()), None, None).await.unwrap();
        }

        let mut bob = state(&hub, "bob").await;
        let joined = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        assert!(joined.contains("session digest"), "digest header shown despite decoys:\n{joined}");
        assert!(joined.contains("next is billing"), "the keyed recap text surfaces, not a decoy:\n{joined}");
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
    async fn task_tool_posts_status_and_peer_sees_a_rendered_line() {
        // The ACP-borrow task lifecycle end-to-end: alice posts a `working` update and a terminal
        // `done` receipt into a session; bob (in the room) recvs and sees them rendered as one-line
        // statuses — the observability the fire-and-hope service flow lacked.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), Some("job".into()), None, None, false).await.unwrap();
        let key = key_of(&opened);
        let mut bob = state(&hub, "bob").await;
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        // Drain bob's own "joined" announce so his next recv starts from the task updates.
        call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();

        // An unknown status is a tool error, not a post.
        assert!(call_session_tool(&mut alice, "parler_task", &json!({ "status": "bogus" })).await.is_err());

        let working = call_session_tool(
            &mut alice,
            "parler_task",
            &json!({ "status": "working", "task": "review-42", "note": "compiling" }),
        )
        .await
        .unwrap();
        assert!(working.contains("🔧 task working (review-42)"), "poster confirmation:\n{working}");

        call_session_tool(
            &mut alice,
            "parler_task",
            &json!({ "status": "done", "task": "review-42", "note": "LGTM", "result": "deadbeef", "tokens": 900 }),
        )
        .await
        .unwrap();

        let seen = call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();
        assert!(seen.contains("🔧 task working (review-42): compiling"), "peer sees the working line:\n{seen}");
        assert!(
            seen.contains("✅ task done (review-42): LGTM — parler fetch deadbeef"),
            "peer sees the done receipt with the result fetch command:\n{seen}"
        );
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
    async fn mcp_recv_commits_cursor_across_a_restart() {
        // An MCP `parler_recv` does one pull per call; the deferred ack (#85) must commit durably, or
        // a cold start (the MCP server restarting) re-renders a batch the host already saw. The
        // "restart" is a fresh MeshAgent (empty in-memory pending_ack) for the SAME identity.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);

        // A stable identity we reconnect with (one Config reused across two connects).
        let bob_cfg = Config::create(hub.clone(), "bob".to_string(), None).unwrap();
        let mut bob = McpState::new(MeshAgent::connect(&bob_cfg).await.unwrap());
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        // Drain bob's own "joined" announce (it sits past his cursor) so his inbox starts clean.
        call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();

        alice.agent.send_text(Target::Room { room: room.clone() }, "only-once").await.unwrap();
        let first = call_session_tool(&mut bob, "parler_recv", &json!({})).await.unwrap();
        assert!(first.contains("only-once"), "first recv renders the message:\n{first}");

        // Restart: a fresh agent for the same identity must resume from the committed cursor.
        let mut bob2 =
            McpState { active_session: Some(room.clone()), ..McpState::new(MeshAgent::connect(&bob_cfg).await.unwrap()) };
        let second = call_session_tool(&mut bob2, "parler_recv", &json!({})).await.unwrap();
        assert!(second.contains("no new messages"), "restart resumes from the committed cursor:\n{second}");
        assert!(!second.contains("only-once"), "the first recv's batch is not re-read after a restart:\n{second}");
    }

    #[tokio::test]
    async fn mcp_run_loop_flushes_deferred_acks_on_exit() {
        // auto-pull-on-send advances the ack but deliberately doesn't commit it (one RTT saved per
        // send). The run loop's clean-exit `flush_acks` commits it, so a restart doesn't re-read the
        // auto-pulled batch. Drive a real `parler_send` through the run loop (stdin EOF ends it →
        // flush), then restart the same identity and prove the auto-pulled reply isn't re-rendered.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let opened = open_session(&mut alice, Some("seed"), None, None, None, false).await.unwrap();
        let room = alice.active_session.clone().unwrap();
        let key = key_of(&opened);

        let bob_cfg = Config::create(hub.clone(), "bob".to_string(), None).unwrap();
        let mut bob = McpState::new(MeshAgent::connect(&bob_cfg).await.unwrap());
        join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap(); // sets bob.active_session

        // Alice leaves a reply waiting so bob's auto-pull-on-send pulls it (advancing the ack) without
        // committing it — exactly the leftover the exit flush is for.
        alice.agent.send_text(Target::Room { room: room.clone() }, "reply-waiting").await.unwrap();

        // One parler_send driven through the run loop; the stdin EOF ends the loop → flush_acks runs.
        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"parler_send\",\"arguments\":{\"text\":\"hi\"}}}\n";
        let mut output: Vec<u8> = Vec::new();
        run(&mut bob, BufReader::new(input.as_bytes()), &mut output).await.unwrap();
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("reply-waiting"), "auto-pull surfaced the waiting reply:\n{out}");

        // Restart: a fresh agent for the same identity must not re-render the auto-pulled reply — the
        // exit flush committed it.
        let mut bob2 =
            McpState { active_session: Some(room.clone()), ..McpState::new(MeshAgent::connect(&bob_cfg).await.unwrap()) };
        let after = call_session_tool(&mut bob2, "parler_recv", &json!({})).await.unwrap();
        assert!(after.contains("no new messages"), "exit flush committed the auto-pulled batch:\n{after}");
        assert!(!after.contains("reply-waiting"), "the auto-pulled reply is not re-read after a restart:\n{after}");
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
    fn same_hub_ignores_scheme_and_trailing_slash() {
        assert!(same_hub("wss://parler-hub.fly.dev", "https://parler-hub.fly.dev"));
        assert!(same_hub("wss://parler-hub.fly.dev/", "wss://parler-hub.fly.dev"));
        assert!(same_hub("ws://127.0.0.1:7070", "http://127.0.0.1:7070"));
        assert!(!same_hub("wss://parler-hub.fly.dev", "ws://127.0.0.1:7070"));
    }

    #[test]
    fn portable_code_redeems_on_this_hub_and_signposts_a_different_one() {
        // No embedded hub → bare code, redeemed against the agent's own hub.
        assert_eq!(portable_code_for_hub("ZX6Y2QPX", "wss://parler-hub.fly.dev").unwrap(), "ZX6Y2QPX");
        // Embedded hub == this hub (scheme aside) → strip it and redeem the bare code locally.
        assert_eq!(
            portable_code_for_hub("ZX6Y2QPX@https://parler-hub.fly.dev", "wss://parler-hub.fly.dev").unwrap(),
            "ZX6Y2QPX"
        );
        assert_eq!(
            portable_code_for_hub(
                "parler://127.0.0.1:7071/join/ZX6Y2QPX",
                "ws://127.0.0.1:7071"
            )
            .unwrap(),
            "ZX6Y2QPX"
        );
        // Embedded hub != this hub → refuse with the exact fix instead of a cryptic "unknown code".
        let err = portable_code_for_hub(
            "parler://127.0.0.1:7071/join/ZX6Y2QPX",
            "wss://parler-hub.fly.dev",
        )
            .unwrap_err()
            .to_string();
        assert!(err.contains("parler://127.0.0.1:7071"), "names the invite's hub: {err}");
        assert!(err.contains("PARLER_HUB=parler://127.0.0.1:7071"), "shows the relaunch fix: {err}");
    }

    #[test]
    fn two_fresh_identities_get_distinct_default_names() {
        // #103 AC1: with no explicit PARLER_NAME, two freshly-minted identities must not collide —
        // the fun handle seeded on each unique agent id makes them distinct.
        let a = Config::create("ws://h", "agent", None).unwrap();
        let b = Config::create("ws://h", "agent", None).unwrap();
        assert_ne!(a.identity.id, b.identity.id, "two mints have distinct ids");
        let name_a = crate::names::fun_name(&a.identity.id);
        let name_b = crate::names::fun_name(&b.identity.id);
        assert_ne!(name_a, name_b, "distinct ids ⇒ distinct default names: {name_a} vs {name_b}");
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
        // Opening a session mints the read-only web/desktop viewer code up front (against a current hub),
        // so the host never reaches for the join KEY in the viewer (which 401s).
        assert!(opened.contains("WATCH code"), "watch-viewer code minted up front");
        // The opener's own name is surfaced so the host can relay who's in the room.
        assert!(
            opened.contains(&format!("you are '{}'", alice.agent.name)),
            "opener's name is surfaced:\n{opened}"
        );
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

    #[tokio::test]
    async fn preapproved_joiner_is_auto_admitted_without_a_manual_approve() {
        // The Tailscale pre-approved-key pattern (#108): a joiner the owner listed at open time is
        // admitted the moment the owner's agent next surfaces requests — no parler_approve_join call.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;

        // Alice opens a gated session but pre-approves "bob" (goes through the real tool path).
        let opened = call_session_tool(
            &mut alice,
            "parler_open_session",
            &json!({ "context": "secret plan: ship friday", "topic": "plan", "preapprove": ["bob"] }),
        )
        .await
        .unwrap();
        assert!(opened.to_lowercase().contains("pre-approved"), "the host is told bob is pre-approved: {opened}");
        let key = key_of(&opened);

        // Bob redeems → still pending: the auto-admit only fires once the owner's agent polls.
        let pending = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        assert!(pending.contains("waiting for the host"), "pending until the owner surfaces requests: {pending}");
        assert!(!pending.contains("secret plan"), "a pending joiner must not receive the context");

        // Alice merely lists requests — bob is auto-admitted, no explicit approval.
        let reqs = call_session_tool(&mut alice, "parler_join_requests", &json!({})).await.unwrap();
        assert!(reqs.to_lowercase().contains("auto-admitted"), "bob was auto-admitted: {reqs}");
        assert!(reqs.contains("bob"), "the auto-admit names the joiner: {reqs}");

        // Bob's next join now succeeds and catches him up — no manual approve_join ever ran.
        let joined = join_session(&mut bob, &key, Backlog::Recent, None).await.unwrap();
        assert!(joined.contains("secret plan: ship friday"), "auto-admitted joiner gets the context: {joined}");
        assert_eq!(bob.active_session, alice.active_session, "both now share the session room");
    }

    #[tokio::test]
    async fn preapproval_does_not_admit_an_unlisted_joiner() {
        // Pre-approving "bob" must not weaken the gate for anyone else — eve still needs approval.
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut eve = state(&hub, "eve").await;

        let opened = call_session_tool(
            &mut alice,
            "parler_open_session",
            &json!({ "context": "secret plan", "topic": "plan", "preapprove": ["bob"] }),
        )
        .await
        .unwrap();
        let key = key_of(&opened);

        let pending = join_session(&mut eve, &key, Backlog::Recent, None).await.unwrap();
        assert!(pending.contains("waiting for the host"), "unlisted joiner is gated: {pending}");

        // Alice lists: eve is NOT auto-admitted, still shown as needing a manual decision.
        let reqs = call_session_tool(&mut alice, "parler_join_requests", &json!({})).await.unwrap();
        assert!(reqs.contains(&eve.agent.id), "eve still needs manual approval: {reqs}");
        assert!(!reqs.to_lowercase().contains("auto-admitted"), "nothing was auto-admitted: {reqs}");

        // And eve still can't get in on her own.
        let still = join_session(&mut eve, &key, Backlog::Recent, None).await.unwrap();
        assert!(still.contains("waiting for the host"), "unlisted joiner stays gated: {still}");
        assert!(eve.active_session.is_none(), "eve is not admitted");
    }

    #[test]
    fn parse_name_list_accepts_array_or_delimited_string() {
        assert_eq!(parse_name_list(Some(&json!(["bob", " codex "]))), vec!["bob", "codex"]);
        assert_eq!(parse_name_list(Some(&json!("bob, codex"))), vec!["bob", "codex"]);
        assert_eq!(parse_name_list(Some(&json!("bob codex"))), vec!["bob", "codex"]);
        assert!(parse_name_list(Some(&json!(""))).is_empty());
        assert!(parse_name_list(None).is_empty());
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
        let fetch_res = call_session_tool(&mut bob, "parler_fetch", &fetch_args).await.unwrap();
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

    #[tokio::test]
    async fn test_mcp_send_file_recv_fetch_e2e() {
        let hub = start_hub().await;
        let mut alice = state(&hub, "alice").await;
        let mut bob = state(&hub, "bob").await;
        let inv = alice.agent.invite(RoomKind::Channel, Some("files".into()), None, None).await.unwrap();
        bob.agent.join(&inv.code).await.unwrap();

        // Alice writes a file and transfers it via the MCP tool.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("report.txt");
        let body: &[u8] = b"the quick brown fox\x00\x01\x02 binary too";
        std::fs::write(&src, body).unwrap();
        let send_args = json!({ "room": inv.room, "path": src.to_str().unwrap(), "note": "here's the report" });
        let send_res = call_tool(&mut alice.agent, "parler_send_file", &send_args).await.unwrap();
        assert!(send_res.contains("sent file 'report.txt'"), "{send_res}");

        // Pull the blob id out of the "parler_fetch id=<BLOB> out=..." hint.
        let idx = send_res.find("id=").unwrap() + 3;
        let rest = &send_res[idx..];
        let end = rest.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(rest.len());
        let blob_id = &rest[..end];

        // Bob sees the file reference (and the note) on recv.
        let (msgs, _) = bob.agent.pull(&inv.room, None, None).await.unwrap();
        let rendered = msgs.iter().map(crate::render_message).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("📎 report.txt"), "recv should show the file: {rendered}");
        assert!(rendered.contains("here's the report"), "recv should show the note: {rendered}");

        // Bob downloads the exact bytes via parler_fetch (explicit blob id).
        let out = dir.path().join("got.txt");
        let fetch_args = json!({ "id": blob_id, "out": out.to_str().unwrap() });
        let fetch_res = call_session_tool(&mut bob, "parler_fetch", &fetch_args).await.unwrap();
        assert!(fetch_res.contains("wrote"), "{fetch_res}");
        assert_eq!(std::fs::read(&out).unwrap(), body.to_vec());

        // Bob fetches with NO id — it auto-finds the file the room just shared. This is the "just
        // fetch the file" flow (explicit `out` here only so the test doesn't write to the cwd).
        let auto_out = out.with_file_name("auto.txt");
        let auto = call_session_tool(
            &mut bob,
            "parler_fetch",
            &json!({ "room": inv.room, "out": auto_out.to_str().unwrap() }),
        )
        .await
        .unwrap();
        assert!(auto.contains("wrote"), "{auto}");
        assert_eq!(std::fs::read(&auto_out).unwrap(), body.to_vec(), "auto-find resolved the blob");

        // Bob fetches by filename (no id), resolving the same blob.
        let by_name = out.with_file_name("byname.txt");
        let named = call_session_tool(
            &mut bob,
            "parler_fetch",
            &json!({ "room": inv.room, "name": "report.txt", "out": by_name.to_str().unwrap() }),
        )
        .await
        .unwrap();
        assert!(named.contains("wrote"), "{named}");
        assert_eq!(std::fs::read(&by_name).unwrap(), body.to_vec());

        // A name that matches nothing is a clear error, not a silent wrong file.
        let miss = call_session_tool(&mut bob, "parler_fetch", &json!({ "room": inv.room, "name": "nope.zip" }))
            .await;
        assert!(miss.is_err(), "unknown name should error");
    }

    /// The authoritative set of real `parler_*` MCP tools: everything `tool_specs()` advertises,
    /// plus the session-scoped tools that are advertised dynamically (not in the static specs).
    /// Both the source check and the docs check below validate against this one list, so there is a
    /// single source of truth for "is this a real tool?".
    fn valid_tool_names() -> std::collections::HashSet<String> {
        let mut valid: std::collections::HashSet<String> = tool_specs()
            .iter()
            .map(|t| t.get("name").unwrap().as_str().unwrap().to_string())
            .collect();
        // `parler_*` doc tokens that are NOT tools in `tool_specs()` above but must still be
        // whitelisted for the docs-drift scan: the two MCP *prompts* (`prompts/list`), which read like
        // tool tokens in the docs. There is no dynamic/on-the-fly tool registration — `tools/list` is
        // static (`tool_specs()`), so every real tool is already covered by the set built above.
        for t in ["parler_session_handoff", "parler_consolidate_session"] {
            valid.insert(t.to_string());
        }
        valid
    }

    /// `parler_*` identifiers that look like a tool token but are NOT tools, so the substring scan
    /// must skip them: the crate names (`parler_protocol`, …), internal probes, and a few unrelated
    /// identifiers that show up in the docs (a Fly volume name, a test name). Excluded from both
    /// drift checks.
    fn is_non_tool_token(r: &str) -> bool {
        matches!(
            r,
            // Cargo crate names.
            "parler_auth"
                | "parler_connector"
                | "parler_protocol"
                | "parler_hub"
                // Internal probe (not an MCP tool).
                | "parler_doctor_probe"
                // Non-tool identifiers that legitimately appear in the docs.
                | "parler_data"   // Fly.io volume name in deploy docs
                | "parler_fields" // fragment of a test name quoted in docs/a2a-interop.md
        )
    }

    /// Pull every `parler_<ident>` token (ident longer than the bare `parler_` prefix) out of a blob.
    fn scan_tool_tokens(src: &str) -> std::collections::HashSet<String> {
        let mut refs = std::collections::HashSet::new();
        let mut s = src;
        while let Some(idx) = s.find("parler_") {
            let s_sub = &s[idx..];
            let len = s_sub.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').count();
            let tool_ref = &s_sub[..len];
            if tool_ref.len() > 7 {
                refs.insert(tool_ref.to_string());
            }
            s = &s_sub[len.max(1)..];
        }
        refs
    }

    #[test]
    fn test_no_phantom_tool_references() {
        let valid_tools = valid_tool_names();

        let mcp_src = std::fs::read_to_string("src/mcp.rs").unwrap_or_else(|_| std::fs::read_to_string("crates/parler-cli/src/mcp.rs").unwrap());
        let lib_src = std::fs::read_to_string("src/lib.rs").unwrap_or_else(|_| std::fs::read_to_string("crates/parler-cli/src/lib.rs").unwrap());

        let mut references = std::collections::HashSet::new();
        for src in &[mcp_src, lib_src] {
            references.extend(scan_tool_tokens(src));
        }

        let mut invalid: Vec<String> = references
            .into_iter()
            .filter(|r| !is_non_tool_token(r) && !valid_tools.contains(r))
            .collect();
        invalid.sort();

        assert!(
            invalid.is_empty(),
            "Found references to parler_* tools/names that are not in the valid tools list: {:?}",
            invalid
        );
    }

    /// Docs must not drift from the tools: every `parler_<tool>` named in README/AGENTS/docs has
    /// to be a real tool. When a tool is renamed or removed in mcp.rs, this fails until the docs are
    /// updated — the mechanical half of the "docs track code" rule in CLAUDE.md / AGENTS.md.
    #[test]
    fn test_docs_reference_only_real_tools() {
        // Walk up from the crate dir (cargo test's CWD) to the repo root, spotted by AGENTS.md.
        let mut root = std::env::current_dir().unwrap();
        while !root.join("AGENTS.md").exists() {
            assert!(root.pop(), "could not find repo root (AGENTS.md) above the crate dir");
        }

        // User-facing doc surface. Recurse into docs/, skipping build/vendor dirs.
        let mut files: Vec<std::path::PathBuf> =
            vec![root.join("README.md"), root.join("AGENTS.md"), root.join("CLAUDE.md")];
        let mut stack = vec![root.join("docs")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if path.is_dir() {
                    if !matches!(name.as_ref(), "node_modules" | ".next" | "target" | ".git") {
                        stack.push(path);
                    }
                } else if matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("md" | "mdx" | "ts" | "tsx" | "js" | "jsx" | "json" | "html" | "txt")
                ) {
                    files.push(path);
                }
            }
        }

        let valid_tools = valid_tool_names();
        let mut invalid: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for file in files {
            let Ok(src) = std::fs::read_to_string(&file) else { continue };
            for r in scan_tool_tokens(&src) {
                if !is_non_tool_token(&r) && !valid_tools.contains(&r) {
                    let rel = file.strip_prefix(&root).unwrap_or(&file).display();
                    invalid.insert(format!("{r} ({rel})"));
                }
            }
        }

        assert!(
            invalid.is_empty(),
            "Docs reference parler_* tools that don't exist (rename/remove the tool ⇒ update the docs \
             in the same PR): {:?}",
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
