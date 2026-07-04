//! `parler connect` — wire every AI agent on this machine to Parler in one command.
//!
//! This is the **single source of truth** for MCP-host setup. The CLI calls it directly; the desktop
//! app shells out to `parler connect --json`. It writes each host's MCP server config pointing at a
//! *per-host* `PARLER_HOME` plus the chosen hub — it never touches the seed itself, because
//! `parler mcp` mints the identity lazily on first launch (see [`crate::mcp`]) from the very env vars
//! we write here. So "set up an agent" collapses to "write one config block", and re-running is safe.
//!
//! ## The hub is a ladder, not a fork
//!
//! The only question a user actually has at setup time is *"does my agents' chat leave this
//! machine?"* — so that is the only axis we expose, with a sane default:
//!
//! * **shared** (default) — the always-on hub the project runs. Nothing to install or start.
//! * `--local` — a hub bound to loopback on this box. Nothing leaves. No secret needed (only local
//!   processes can reach `127.0.0.1`).
//! * `--team` — a hub reachable by teammates on your LAN, gated by a generated join secret.
//!
//! Directory *visibility* (being findable by strangers) is deliberately **not** here — it's a
//! separate, opt-in `parler register --public`, so nobody has to reason about it just to say hello.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the traffic goes. Detangles the one setup question from directory visibility + hub topology.
#[derive(Debug, Clone)]
pub enum Hub {
    /// The always-on shared hub run by the project (default).
    Shared,
    /// A hub on this machine, bound to loopback — nothing leaves the box.
    Local { port: u16 },
    /// A hub on this machine reachable by teammates — gated by a generated join secret.
    Team { port: u16 },
    /// An explicit hub URL (advanced escape hatch).
    Explicit(String),
}

impl Hub {
    /// The `PARLER_HUB` URL agents on *this* machine should dial.
    fn url(&self) -> String {
        match self {
            Hub::Shared => crate::mcp::DEFAULT_PUBLIC_HUB.to_string(),
            Hub::Local { port } | Hub::Team { port } => format!("ws://127.0.0.1:{port}"),
            Hub::Explicit(u) => u.clone(),
        }
    }

    fn mode(&self) -> &'static str {
        match self {
            Hub::Shared => "shared",
            Hub::Local { .. } => "local",
            Hub::Team { .. } => "team",
            Hub::Explicit(_) => "explicit",
        }
    }
}

/// Options for a `parler connect` run (built from the CLI args, kept clap-free so this stays reusable).
#[derive(Debug, Clone)]
pub struct Options {
    /// Explicit agents to wire (ids/aliases). Empty = auto-detect and wire everything installed.
    pub hosts: Vec<String>,
    pub hub: Hub,
    /// Display-name base for this machine's agents (default: the agent id, e.g. `codex`).
    pub name: Option<String>,
    /// Explicit join secret to write for every host (for a secret-gated hub, e.g. via `--hub`).
    /// `--team` generates one when this is unset. The desktop app passes its local hub's secret here.
    pub join_secret: Option<String>,
    /// Don't write anything — just print the config to paste yourself.
    pub print: bool,
    /// List detected agents + their Parler status and exit; write nothing.
    pub list: bool,
    /// Remove Parler from the named hosts (or every configured host when `hosts` is empty). The
    /// inverse of the default wire action — the desktop app's "Disconnect" drives this.
    pub remove: bool,
    /// Emit machine-readable JSON (used by the desktop app).
    pub json: bool,
    /// Whether a hub was chosen explicitly (`--shared`/`--local`/`--team`/`--hub`). When false — a
    /// bare `parler connect` — a host that is already wired **keeps its current hub and join
    /// secret** instead of being silently moved to the default, so a CLI re-run can't undo the hub
    /// the desktop app (or an earlier `--local`/`--team` run) chose.
    pub hub_pinned: bool,
    /// Mint a *fresh* `--team` join secret even if this hub already has one wired. Off by default so
    /// re-running `parler connect --team` reuses the existing secret (a new one would strand the
    /// already-running hub, which still enforces the old secret — issue #101). Turning it on prints
    /// the exact `parler hub …` restart line the operator must run with the new secret.
    pub rotate_secret: bool,
}

/// One successfully wired agent, as [`run`] reports it back — everything `--verify` needs to watch
/// the agent actually dial its hub.
#[derive(Debug, Clone)]
pub struct WiredAgent {
    /// The display name written as `PARLER_NAME` (what the directory will show).
    pub name: String,
    /// The hub URL written as `PARLER_HUB`.
    pub hub: String,
    /// The join secret written alongside, when the hub is gated.
    pub secret: Option<String>,
}

/// How a given MCP host stores its server config — the knowledge that used to be scattered across
/// docs and the Electron app, centralized so one code path serves the CLI *and* the desktop.
#[derive(Debug, Clone)]
pub(crate) enum Wiring {
    /// Claude Code — driven through its own `claude mcp add` CLI (the supported API).
    ClaudeCli,
    /// A JSON file with a top-level `mcpServers` object (Cursor, Windsurf, Gemini, Claude Desktop).
    Json(PathBuf),
    /// A TOML file with `[mcp_servers.<name>]` tables (Codex).
    Toml(PathBuf),
}

pub(crate) struct HostDef {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) wiring: Wiring,
    /// If any of these paths exists, the host is considered installed.
    pub(crate) hints: Vec<PathBuf>,
}

/// The MCP server name we register under, in every host.
const SERVER_NAME: &str = "parler";

// ---------------------------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------------------------

pub(crate) fn registry() -> Vec<HostDef> {
    let home = user_home();
    let claude_desktop_dir = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Claude")
    } else {
        home.join(".config/Claude")
    };
    vec![
        HostDef {
            id: "claude-code",
            name: "Claude Code",
            wiring: Wiring::ClaudeCli,
            hints: vec![home.join(".claude"), home.join(".claude.json")],
        },
        HostDef {
            id: "codex",
            name: "Codex",
            wiring: Wiring::Toml(home.join(".codex/config.toml")),
            hints: vec![home.join(".codex")],
        },
        HostDef {
            id: "cursor",
            name: "Cursor",
            wiring: Wiring::Json(home.join(".cursor/mcp.json")),
            hints: vec![home.join(".cursor"), PathBuf::from("/Applications/Cursor.app")],
        },
        HostDef {
            id: "windsurf",
            name: "Windsurf",
            wiring: Wiring::Json(home.join(".codeium/windsurf/mcp_config.json")),
            hints: vec![home.join(".codeium/windsurf"), PathBuf::from("/Applications/Windsurf.app")],
        },
        HostDef {
            id: "gemini",
            name: "Gemini CLI",
            wiring: Wiring::Json(home.join(".gemini/settings.json")),
            hints: vec![home.join(".gemini")],
        },
        HostDef {
            id: "claude-desktop",
            name: "Claude Desktop",
            wiring: Wiring::Json(claude_desktop_dir.join("claude_desktop_config.json")),
            hints: vec![claude_desktop_dir, PathBuf::from("/Applications/Claude.app")],
        },
    ]
}

/// How a host picks the new config up — per-host truth so "restart them" is never vague. Claude
/// Code notably needs **no** restart: user-scope MCP servers load on the next session.
fn restart_hint(id: &str) -> &'static str {
    match id {
        "claude-code" => "picks it up in your next session — no restart needed",
        "codex" => "takes effect on the next `codex` run",
        "gemini" => "takes effect on the next `gemini` run",
        "cursor" => "restart Cursor (or toggle the parler server under Settings → MCP)",
        "windsurf" => "restart Windsurf to load it",
        "claude-desktop" => "quit and reopen Claude Desktop",
        _ => "restart it to load Parler",
    }
}

/// Map a user-typed host token to a known id, tolerating the obvious aliases.
fn canonical_id(token: &str) -> Option<&'static str> {
    match token.trim().to_ascii_lowercase().replace([' ', '_'], "-").as_str() {
        "claude" | "claude-code" | "claudecode" | "cc" => Some("claude-code"),
        "claude-desktop" | "claudedesktop" | "desktop" => Some("claude-desktop"),
        "codex" => Some("codex"),
        "cursor" => Some("cursor"),
        "windsurf" => Some("windsurf"),
        "gemini" | "gemini-cli" => Some("gemini"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------------------------

pub(crate) fn is_installed(def: &HostDef) -> bool {
    match &def.wiring {
        Wiring::ClaudeCli => resolve_claude().is_some() || def.hints.iter().any(|p| p.exists()),
        _ => def.hints.iter().any(|p| p.exists()),
    }
}

/// Whether this host already has a `parler` server configured (so `--list` can say "connected").
pub(crate) fn is_configured(def: &HostDef) -> bool {
    match &def.wiring {
        Wiring::ClaudeCli => resolve_claude()
            .map(|claude| {
                Command::new(claude)
                    .args(["mcp", "get", SERVER_NAME])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            })
            .unwrap_or(false),
        Wiring::Json(path) => read_json(path)
            .ok()
            .and_then(|v| v.get("mcpServers").and_then(|s| s.get(SERVER_NAME)).cloned())
            .is_some(),
        Wiring::Toml(path) => std::fs::read_to_string(path)
            .ok()
            .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
            .map(|d| d.get("mcp_servers").and_then(|t| t.get(SERVER_NAME)).is_some())
            .unwrap_or(false),
    }
}

/// The hub a host is currently wired to, plus the join secret written beside it (so `--list --json`
/// can tell the desktop app where an agent points, and a bare re-run can keep both). Best-effort:
/// `None` if unconfigured/unreadable.
fn configured_env(def: &HostDef) -> Option<(String, Option<String>)> {
    match &def.wiring {
        Wiring::ClaudeCli => {
            let claude = resolve_claude()?;
            let out = Command::new(claude).args(["mcp", "get", SERVER_NAME]).output().ok()?;
            if !out.status.success() {
                return None;
            }
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            let hub = parse_env_from_text(&text, "PARLER_HUB")?;
            Some((hub, parse_env_from_text(&text, "PARLER_JOIN_SECRET")))
        }
        Wiring::Json(path) => {
            let root = read_json(path).ok()?;
            let env = root.get("mcpServers")?.get(SERVER_NAME)?.get("env")?;
            let hub = env.get("PARLER_HUB")?.as_str().map(String::from)?;
            let secret = env.get("PARLER_JOIN_SECRET").and_then(Value::as_str).map(String::from);
            Some((hub, secret))
        }
        Wiring::Toml(path) => {
            let doc: toml_edit::DocumentMut = std::fs::read_to_string(path).ok()?.parse().ok()?;
            let env = doc.get("mcp_servers")?.get(SERVER_NAME)?.get("env")?;
            let hub = env.get("PARLER_HUB")?.as_str().map(String::from)?;
            let secret =
                env.get("PARLER_JOIN_SECRET").and_then(|v| v.as_str()).map(String::from);
            Some((hub, secret))
        }
    }
}

/// Pull the first `<key>=<value>` (or `<key>: <value>`) token out of free-form text — used to read
/// env vars back from `claude mcp get`'s human output without pulling in a regex dependency.
fn parse_env_from_text(s: &str, key: &str) -> Option<String> {
    let idx = s.find(key)?;
    let rest = s[idx + key.len()..].trim_start_matches([':', '=', ' ', '"']);
    let val: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '"' && *c != ',')
        .collect();
    (!val.is_empty()).then_some(val)
}

/// Resolve the absolute `claude` path. Unlike a GUI app, the CLI inherits the login shell's PATH, but
/// we still probe the common install spots first so this works even from a bare environment.
fn resolve_claude() -> Option<PathBuf> {
    let home = user_home();
    let fixed = [
        home.join(".local/bin/claude"),
        home.join(".claude/local/claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
        PathBuf::from("/usr/bin/claude"),
    ];
    for c in fixed {
        if c.is_file() {
            return Some(c);
        }
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).map(|d| d.join("claude")).find(|p| p.is_file())
}

// ---------------------------------------------------------------------------------------------
// Config writers — idempotent, never clobber the user's other servers.
// ---------------------------------------------------------------------------------------------

/// Write our server into a JSON `mcpServers` file, preserving everything else in it.
fn write_json(path: &Path, env: &[(String, String)], binpath: &str) -> Result<()> {
    let mut root = if path.exists() {
        read_json(path)?
    } else {
        json!({})
    };
    let obj = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} is not a JSON object — move it aside and re-run", path.display()))?;
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} has a non-object \"mcpServers\" — fix it and re-run", path.display()))?;
    servers.insert(SERVER_NAME.to_string(), server_json(env, binpath));
    write_secure(path, &(serde_json::to_string_pretty(&root)? + "\n"))
}

/// Write our server into a TOML `[mcp_servers.parler]` table (Codex), preserving comments + others.
fn write_toml(path: &Path, env: &[(String, String)], binpath: &str) -> Result<()> {
    use toml_edit::{DocumentMut, Item, Table};
    let mut doc: DocumentMut = if path.exists() {
        std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?
            .parse()
            .with_context(|| format!("{} isn't valid TOML — fix or move it, then re-run (or use --print)", path.display()))?
    } else {
        DocumentMut::new()
    };
    // Materialize `mcp_servers` as a real (implicit) table first: indexing it into existence on a
    // doc that doesn't have one yields an empty inline `mcp_servers = {}` that drops our entry.
    let mut holder = Table::new();
    holder.set_implicit(true);
    let servers = doc.entry("mcp_servers").or_insert(Item::Table(holder));
    // Replace our table wholesale (idempotent) while leaving the user's other servers untouched.
    servers[SERVER_NAME] = parler_toml_item(env, binpath);
    write_secure(path, &doc.to_string())
}

/// Our server as an explicit (non-implicit) TOML table, so it always renders as a readable
/// `[mcp_servers.parler]` header rather than scattered `mcp_servers.parler.* = …` dotted keys.
fn parler_toml_item(env: &[(String, String)], binpath: &str) -> toml_edit::Item {
    use toml_edit::{value, Array, Item, Table};
    let mut args = Array::new();
    args.push("mcp");
    let mut t = Table::new();
    t["command"] = value(binpath);
    t["args"] = value(args);
    let mut env_tbl = Table::new();
    for (k, v) in env {
        env_tbl[k.as_str()] = value(v.as_str());
    }
    t["env"] = Item::Table(env_tbl);
    Item::Table(t)
}

fn read_json(path: &Path) -> Result<Value> {
    let txt = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if txt.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&txt)
        .with_context(|| format!("{} isn't valid JSON — fix or move it, then re-run (or use --print)", path.display()))
}

fn server_json(env: &[(String, String)], binpath: &str) -> Value {
    let mut e = serde_json::Map::new();
    for (k, v) in env {
        e.insert(k.clone(), Value::String(v.clone()));
    }
    json!({ "command": binpath, "args": ["mcp"], "env": Value::Object(e) })
}

/// Create parents, write, and lock to `0600` — configs we author can carry a join secret (team mode),
/// so we hold them to the same least-privilege posture as the identity seed.
fn write_secure(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Wire a known host in place. Returns a short human description of where it landed.
fn wire(def: &HostDef, env: &[(String, String)], binpath: &str) -> Result<String> {
    match &def.wiring {
        Wiring::ClaudeCli => {
            let claude = resolve_claude()
                .ok_or_else(|| anyhow!("Claude Code CLI not found on PATH — install it, then re-run"))?;
            // Idempotent: drop any prior entry first (ignore failure if it wasn't there).
            let _ = Command::new(&claude).args(["mcp", "remove", SERVER_NAME, "--scope", "user"]).output();
            let mut args: Vec<String> =
                ["mcp", "add", SERVER_NAME, "--scope", "user"].iter().map(|s| s.to_string()).collect();
            for (k, v) in env {
                args.push("--env".into());
                args.push(format!("{k}={v}"));
            }
            args.push("--".into());
            args.push(binpath.to_string());
            args.push("mcp".into());
            let out = Command::new(&claude).args(&args).output().context("running claude mcp add")?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                bail!("claude mcp add failed: {}", err.lines().next().unwrap_or("unknown error").trim());
            }
            Ok("claude mcp add (user scope)".to_string())
        }
        Wiring::Json(path) => {
            write_json(path, env, binpath)?;
            Ok(display_path(path))
        }
        Wiring::Toml(path) => {
            write_toml(path, env, binpath)?;
            Ok(display_path(path))
        }
    }
}

/// Remove Parler from a host in place, leaving the user's other servers untouched. `Ok(true)` if an
/// entry was actually removed, `Ok(false)` if there was nothing to remove.
fn unwire(def: &HostDef) -> Result<bool> {
    match &def.wiring {
        Wiring::ClaudeCli => {
            let claude = resolve_claude()
                .ok_or_else(|| anyhow!("Claude Code CLI not found on PATH — install it, then re-run"))?;
            // `claude mcp remove` exits non-zero when it wasn't configured; treat that as "nothing
            // to remove" rather than an error.
            let out = Command::new(&claude)
                .args(["mcp", "remove", SERVER_NAME, "--scope", "user"])
                .output()
                .context("running claude mcp remove")?;
            Ok(out.status.success())
        }
        Wiring::Json(path) => remove_json(path),
        Wiring::Toml(path) => remove_toml(path),
    }
}

/// Drop `mcpServers.parler` from a JSON config, preserving everything else. `Ok(false)` if absent.
fn remove_json(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let mut root = read_json(path)?;
    let removed = root
        .get_mut("mcpServers")
        .and_then(|s| s.as_object_mut())
        .map(|servers| servers.remove(SERVER_NAME).is_some())
        .unwrap_or(false);
    if removed {
        write_secure(path, &(serde_json::to_string_pretty(&root)? + "\n"))?;
    }
    Ok(removed)
}

/// Drop `[mcp_servers.parler]` from a TOML config, preserving comments + other tables. `Ok(false)`
/// if absent.
fn remove_toml(path: &Path) -> Result<bool> {
    use toml_edit::DocumentMut;
    if !path.exists() {
        return Ok(false);
    }
    let mut doc: DocumentMut = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?
        .parse()
        .with_context(|| format!("{} isn't valid TOML — fix or move it, then re-run", path.display()))?;
    let removed = doc
        .get_mut("mcp_servers")
        .and_then(|t| t.as_table_mut())
        .map(|t| t.remove(SERVER_NAME).is_some())
        .unwrap_or(false);
    if removed {
        write_secure(path, &doc.to_string())?;
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------------------------
// Snippet rendering (for --print and unknown hosts)
// ---------------------------------------------------------------------------------------------

fn render_shell_add(env: &[(String, String)], binpath: &str) -> String {
    let flags: String = env.iter().map(|(k, v)| format!(" --env {k}={v}")).collect();
    format!("claude mcp add {SERVER_NAME} --scope user{flags} -- {binpath} mcp")
}

fn render_json_block(env: &[(String, String)], binpath: &str) -> String {
    let block = json!({ "mcpServers": { SERVER_NAME: server_json(env, binpath) } });
    serde_json::to_string_pretty(&block).unwrap_or_default()
}

fn render_toml_block(env: &[(String, String)], binpath: &str) -> String {
    let mut doc = toml_edit::DocumentMut::new();
    doc["mcp_servers"][SERVER_NAME] = parler_toml_item(env, binpath);
    doc.to_string()
}

fn snippet_for(def: &HostDef, env: &[(String, String)], binpath: &str) -> String {
    match &def.wiring {
        Wiring::ClaudeCli => render_shell_add(env, binpath),
        Wiring::Json(path) => format!("→ {}\n{}", display_path(path), render_json_block(env, binpath)),
        Wiring::Toml(path) => format!("→ {}\n{}", display_path(path), render_toml_block(env, binpath)),
    }
}

/// The portable snippet for an unknown host (e.g. `parler connect hermes`) — most MCP hosts read the
/// JSON shape, and we also show the raw invocation for anything bespoke.
fn generic_snippet(env: &[(String, String)], binpath: &str) -> String {
    format!(
        "Most MCP hosts take this JSON (add it under their \"mcpServers\"):\n{}\n\nOr run this binary directly with these env vars:\n{}",
        render_json_block(env, binpath),
        env.iter().map(|(k, v)| format!("{k}={v} ")).collect::<String>() + binpath + " mcp"
    )
}

// ---------------------------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------------------------

enum Target {
    Known(HostDef),
    /// A host we don't know how to write for — we print a paste-snippet instead.
    Unknown(String),
}

struct Report {
    id: String,
    name: String,
    status: &'static str, // wired | printed | error
    detail: String,
    config: Option<String>,
    /// The hub this host was wired to (differs from the run's default when it was kept).
    hub: Option<String>,
    /// True when a bare re-run preserved the host's previously configured hub + secret.
    kept: bool,
}

/// `parler connect` entry point. Returns the successfully wired agents so the caller can offer to
/// `--verify` them (watch each one dial its hub).
pub fn run(opts: Options) -> Result<Vec<WiredAgent>> {
    let binpath = binary_path();
    let hub_url = opts.hub.url();
    // Resolve the join secret with reuse-by-default for `--team`:
    //   1. an explicit `--join-secret` / `PARLER_JOIN_SECRET` always wins;
    //   2. else, for `--team`, reuse the secret already wired for this hub (so a re-run doesn't
    //      strand the running hub with a stale secret — issue #101) unless `--rotate-secret`;
    //   3. else mint a fresh one (first `--team`, or an explicit rotation).
    // `minted_secret` is true only in case 3, so we print the hub-restart line just then.
    let is_team = matches!(opts.hub, Hub::Team { .. });
    let existing = is_team.then(|| existing_team_secret(&hub_url)).flatten();
    let (secret, minted_secret) = pick_team_secret(
        opts.join_secret.clone().filter(|s| !s.is_empty()),
        is_team,
        opts.rotate_secret,
        existing,
    );

    if opts.list {
        print_list(opts.json)?;
        return Ok(Vec::new());
    }

    if opts.remove {
        run_remove(&opts)?;
        return Ok(Vec::new());
    }

    // Resolve the set of targets.
    let reg = registry();
    let mut targets: Vec<Target> = Vec::new();
    if opts.hosts.is_empty() {
        // Auto: wire everything installed.
        for def in reg {
            if is_installed(&def) {
                targets.push(Target::Known(def));
            }
        }
        if targets.is_empty() {
            // Nothing detected — guide the user with a portable snippet rather than silence.
            targets.push(Target::Unknown("agent".to_string()));
        }
    } else {
        for token in &opts.hosts {
            match canonical_id(token) {
                Some(id) => {
                    let def = registry().into_iter().find(|d| d.id == id).expect("id from canonical_id");
                    targets.push(Target::Known(def));
                }
                None => targets.push(Target::Unknown(token.clone())),
            }
        }
    }

    let mut reports: Vec<Report> = Vec::new();
    let mut snippets: Vec<(String, String)> = Vec::new();
    let mut wired: Vec<WiredAgent> = Vec::new();
    // If the user already minted an identity with a manual `parler mcp` (a bare `~/.parler/config.json`,
    // no PARLER_HOME) before running connect, reuse it for the first host we wire — pointing that host
    // at `~/.parler` — rather than minting a fresh per-agent identity and orphaning the one they
    // already registered/joined a session with. Only when the bare identity dials the *same* hub we're
    // wiring this host to, since `parler mcp` loads a saved config verbatim (it wouldn't follow a
    // different PARLER_HUB). Non-destructive: nothing is moved or deleted; the other hosts still get
    // their own per-agent identities.
    let bare_hub = bare_identity_hub();
    let mut primary_wired = false;
    for t in &targets {
        match t {
            Target::Known(def) => {
                // A bare re-run keeps an already-wired host on its current hub (+ its secret); an
                // explicit hub flag moves everyone to the chosen hub.
                let kept_env = if opts.hub_pinned { None } else { configured_env(def) };
                let kept = kept_env.is_some();
                let (host_hub, host_secret) =
                    kept_env.unwrap_or_else(|| (hub_url.clone(), secret.clone()));
                let adopt_bare = !primary_wired && bare_hub.as_deref() == Some(host_hub.as_str());
                primary_wired = true;
                let env = env_for(def.id, &opts, &host_hub, host_secret.as_deref(), adopt_bare);
                if opts.print {
                    snippets.push((def.name.to_string(), snippet_for(def, &env, &binpath)));
                    reports.push(Report {
                        id: def.id.into(),
                        name: def.name.into(),
                        status: "printed",
                        detail: "snippet printed".into(),
                        config: None,
                        hub: Some(host_hub),
                        kept,
                    });
                } else {
                    match wire(def, &env, &binpath) {
                        Ok(where_) => {
                            wired.push(WiredAgent {
                                name: opts.name.clone().unwrap_or_else(|| def.id.to_string()),
                                hub: host_hub.clone(),
                                secret: host_secret.clone(),
                            });
                            reports.push(Report {
                                id: def.id.into(),
                                name: def.name.into(),
                                status: "wired",
                                detail: where_.clone(),
                                config: Some(where_),
                                hub: Some(host_hub),
                                kept,
                            });
                        }
                        Err(e) => reports.push(Report {
                            id: def.id.into(),
                            name: def.name.into(),
                            status: "error",
                            detail: format!("{e}"),
                            config: None,
                            hub: Some(host_hub),
                            kept,
                        }),
                    }
                }
            }
            Target::Unknown(name) => {
                let env = env_for(name, &opts, &hub_url, secret.as_deref(), false);
                snippets.push((name.clone(), generic_snippet(&env, &binpath)));
                reports.push(Report {
                    id: name.clone(),
                    name: name.clone(),
                    status: "printed",
                    detail: "snippet printed".into(),
                    config: None,
                    hub: Some(hub_url.clone()),
                    kept: false,
                });
            }
        }
    }

    if opts.json {
        emit_json(&opts, &hub_url, secret.as_deref(), &binpath, &reports, &snippets);
    } else {
        emit_human(&opts, &hub_url, secret.as_deref(), minted_secret, &reports, &snippets);
    }
    Ok(wired)
}

/// `parler connect --remove` — the inverse of a wire. Unwires the named hosts, or every configured
/// host when none are named. Reuses the same registry so it stays symmetric with wiring.
fn run_remove(opts: &Options) -> Result<()> {
    let mut targets: Vec<Target> = Vec::new();
    if opts.hosts.is_empty() {
        for def in registry() {
            if is_installed(&def) {
                targets.push(Target::Known(def));
            }
        }
    } else {
        for token in &opts.hosts {
            match canonical_id(token) {
                Some(id) => {
                    let def = registry().into_iter().find(|d| d.id == id).expect("id from canonical_id");
                    targets.push(Target::Known(def));
                }
                None => targets.push(Target::Unknown(token.clone())),
            }
        }
    }

    let mut reports: Vec<Report> = Vec::new();
    for t in &targets {
        match t {
            Target::Known(def) => match unwire(def) {
                Ok(true) => reports.push(Report {
                    id: def.id.into(),
                    name: def.name.into(),
                    status: "removed",
                    detail: "removed".into(),
                    config: None,
                    hub: None,
                    kept: false,
                }),
                Ok(false) => reports.push(Report {
                    id: def.id.into(),
                    name: def.name.into(),
                    status: "not-configured",
                    detail: "wasn't connected".into(),
                    config: None,
                    hub: None,
                    kept: false,
                }),
                Err(e) => reports.push(Report {
                    id: def.id.into(),
                    name: def.name.into(),
                    status: "error",
                    detail: format!("{e}"),
                    config: None,
                    hub: None,
                    kept: false,
                }),
            },
            Target::Unknown(name) => reports.push(Report {
                id: name.clone(),
                name: name.clone(),
                status: "not-configured",
                detail: "unknown host — nothing to remove".into(),
                config: None,
                hub: None,
                kept: false,
            }),
        }
    }

    if opts.json {
        let results: Vec<Value> = reports
            .iter()
            .map(|r| json!({ "id": r.id, "name": r.name, "status": r.status, "detail": r.detail }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "action": "remove", "results": results })).unwrap_or_default()
        );
    } else {
        println!("\nParler · disconnecting your agents\n");
        for r in &reports {
            let mark = match r.status {
                "removed" => "  ✓",
                "error" => "  ✗",
                _ => "  ·",
            };
            println!("{mark} {:<15} {}", r.name, r.detail);
        }
        if reports.iter().any(|r| r.status == "removed") {
            println!("\nRestart those apps to unload Parler.");
        }
        println!();
    }
    Ok(())
}

/// The env block written into each host — the whole of what makes an agent "an agent" on the mesh.
///
/// Each host normally gets its own `PARLER_HOME` under `~/.parler/agents/<id>`, so its identity is
/// stable and distinct. The one exception: `adopt_bare` points the *primary* host at `~/.parler`
/// itself, reusing an identity the user already minted with a manual `parler mcp` (a bare
/// `~/.parler/config.json`) instead of orphaning it and creating a second — see [`run`].
fn env_for(id: &str, opts: &Options, hub_url: &str, secret: Option<&str>, adopt_bare: bool) -> Vec<(String, String)> {
    let home = if adopt_bare { parler_root() } else { parler_root().join("agents").join(id) };
    let name = opts.name.clone().unwrap_or_else(|| id.to_string());
    let mut v = vec![
        ("PARLER_HOME".to_string(), home.to_string_lossy().into_owned()),
        ("PARLER_HUB".to_string(), hub_url.to_string()),
        ("PARLER_NAME".to_string(), name),
    ];
    if let Some(s) = secret {
        v.push(("PARLER_JOIN_SECRET".to_string(), s.to_string()));
    }
    v
}

// ---------------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------------

fn emit_human(opts: &Options, hub_url: &str, secret: Option<&str>, minted_secret: bool, reports: &[Report], snippets: &[(String, String)]) {
    let where_ = match &opts.hub {
        Hub::Shared => "the shared hub".to_string(),
        Hub::Local { .. } => "a hub on THIS machine (nothing leaves the box)".to_string(),
        Hub::Team { .. } => "your team hub".to_string(),
        Hub::Explicit(_) => "the hub you named".to_string(),
    };
    println!("\nParler · connecting your agents to {where_}");
    println!("         {hub_url}");
    if !opts.hub_pinned && reports.iter().any(|r| r.kept) {
        println!("         (already-wired agents keep their hub — move them with --shared / --local / --team)");
    }
    println!();

    let installed_ids: Vec<&'static str> = registry().iter().filter(|d| is_installed(d)).map(|d| d.id).collect();
    for r in reports {
        let mark = match r.status {
            "wired" => "  ✓",
            "error" => "  ✗",
            _ => "  ·",
        };
        // Name the hub inline whenever it isn't the run's default, so a kept host is never a surprise.
        let hub_note = match &r.hub {
            Some(h) if r.kept => format!("  → {h} (kept)"),
            Some(h) if h != hub_url => format!("  → {h}"),
            _ => String::new(),
        };
        println!("{mark} {:<15} {}{hub_note}", r.name, r.detail);
    }
    // In an auto run, name the known hosts we skipped because they weren't detected.
    if opts.hosts.is_empty() && !opts.print {
        for def in registry() {
            if !installed_ids.contains(&def.id) {
                // Name the path we checked so a user whose agent lives elsewhere knows *why* it was
                // missed, and point at `--print` as the manual escape hatch for that case.
                let looked = def.hints.first().map(|p| display_path(p)).unwrap_or_default();
                println!(
                    "  · {:<15} not detected (looked in {}) — installed elsewhere? parler connect {} --print",
                    def.name, looked, def.id
                );
            }
        }
    }

    if !snippets.is_empty() {
        println!("\nPaste these where the host reads MCP servers:");
        for (label, text) in snippets {
            println!("\n── {label} ──\n{text}");
        }
    }

    let wired = reports.iter().any(|r| r.status == "wired");
    if wired {
        println!("\nLoad it — per host:");
        for r in reports.iter().filter(|r| r.status == "wired") {
            println!("  {:<15} {}", r.name, restart_hint(&r.id));
        }
        println!("\nEach agent gets its own stable identity under {} (an agent you already set up keeps the one it has).", display_path(&parler_root().join("agents")));
        println!("Watch them come online:  parler connect --verify   (or check later: parler connect --list)");
        println!("Troubleshoot setup:      parler doctor");
    }

    // The hub-run + teammate instructions, only where they apply.
    match &opts.hub {
        Hub::Local { port } => {
            println!("\nStart the local hub in its own terminal (keep it running), then restart your agents:");
            println!("  {}", local_hub_cmd(*port));
        }
        Hub::Team { port } => {
            if let Some(s) = secret {
                if minted_secret {
                    // A fresh secret (first `--team`, or `--rotate-secret`): the operator must (re)start
                    // the hub with THIS secret, or already-wired agents will fail auth against the old one.
                    println!("\nJoin secret (share out-of-band — not on a shared screen):\n  {s}");
                    if opts.rotate_secret {
                        println!("\n⚠ New secret minted — RESTART the hub with it, or agents fail auth on the old one:");
                    } else {
                        println!("\nRun your team hub (keep it running):");
                    }
                    println!("  {}", team_hub_cmd(*port, s));
                } else {
                    // Reused the secret already wired for this hub — the running hub keeps working, so
                    // don't imply a restart. (Rotate deliberately with --rotate-secret.)
                    println!("\nReusing this hub's existing join secret (the running hub stays valid — no restart).");
                    println!("  Rotate it deliberately with:  parler connect --team --rotate-secret");
                }
                let ip = detect_lan_ip().unwrap_or_else(|| "<this-machine-ip>".to_string());
                println!("\nTeammates connect their agents with:\n  PARLER_HUB=ws://{ip}:{port} PARLER_JOIN_SECRET={s} parler connect");
            }
        }
        _ => {}
    }

    // The one honest sentence about confidentiality — only for the shared hub.
    if matches!(opts.hub, Hub::Shared) {
        println!(
            "\nYour agents' chats stay between them — but whoever runs the shared hub could read what\npasses through it. For anything sensitive, run `parler connect --local` and nothing leaves\nthis Mac."
        );
    }
    println!();
}

fn emit_json(opts: &Options, hub_url: &str, secret: Option<&str>, binpath: &str, reports: &[Report], snippets: &[(String, String)]) {
    let results: Vec<Value> = reports
        .iter()
        .map(|r| json!({ "id": r.id, "name": r.name, "status": r.status, "detail": r.detail, "config": r.config,
                          "hub": r.hub, "kept": r.kept, "restart": restart_hint(&r.id) }))
        .collect();
    let snips: Vec<Value> = snippets.iter().map(|(l, t)| json!({ "label": l, "text": t })).collect();
    let run_hub = match &opts.hub {
        Hub::Local { port } => Some(local_hub_cmd(*port)),
        Hub::Team { port } => secret.map(|s| team_hub_cmd(*port, s)),
        _ => None,
    };
    let teammate = match (&opts.hub, secret) {
        (Hub::Team { port }, Some(s)) => {
            let ip = detect_lan_ip().unwrap_or_else(|| "<this-machine-ip>".to_string());
            Some(format!("PARLER_HUB=ws://{ip}:{port} PARLER_JOIN_SECRET={s} parler connect"))
        }
        _ => None,
    };
    let out = json!({
        "hub": hub_url,
        "mode": opts.hub.mode(),
        "secret": secret,
        "binary": binpath,
        "results": results,
        "snippets": snips,
        "run_hub": run_hub,
        "teammate": teammate,
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
}

fn print_list(as_json: bool) -> Result<()> {
    let reg = registry();
    if as_json {
        let hosts: Vec<Value> = reg
            .iter()
            .map(|d| {
                let path = match &d.wiring {
                    Wiring::ClaudeCli => "claude mcp (user scope)".to_string(),
                    Wiring::Json(p) | Wiring::Toml(p) => display_path(p),
                };
                let connected = is_configured(d);
                let hub = connected.then(|| configured_env(d)).flatten().map(|(h, _)| h);
                json!({ "id": d.id, "name": d.name, "installed": is_installed(d), "connected": connected, "config": path, "hub": hub })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json!({ "hosts": hosts })).unwrap_or_default());
        return Ok(());
    }
    println!("\nAgents Parler knows how to wire on this machine:\n");
    for d in &reg {
        let status = if is_configured(d) {
            "connected"
        } else if is_installed(d) {
            "installed"
        } else {
            "not found"
        };
        println!("  {:<15} {:<11} {}", d.name, status, match &d.wiring {
            Wiring::ClaudeCli => "claude mcp (user scope)".to_string(),
            Wiring::Json(p) | Wiring::Toml(p) => display_path(p),
        });
    }
    println!("\nWire the detected ones with:  parler connect\nOr a specific one with:       parler connect <name>\n");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// `~/.parler` — the deterministic root for per-host identities (independent of any ambient
/// `PARLER_HOME`, since `connect` is a machine-setup command, not a session).
fn parler_root() -> PathBuf {
    user_home().join(".parler")
}

/// Decide the join secret for a run, and whether it was freshly minted (case 3 below), factored out
/// of [`run`] so the reuse rule (issue #101) is unit-testable without touching the filesystem:
///   1. an explicit `--join-secret` / `PARLER_JOIN_SECRET` always wins — passthrough, not minted;
///   2. non-`--team` hubs have no secret;
///   3. `--team`: reuse the `existing` wired secret unless `rotate` is set; else mint a fresh one
///      (returned with `minted = true` so the caller prints the hub-restart instruction).
fn pick_team_secret(
    explicit: Option<String>,
    is_team: bool,
    rotate: bool,
    existing: Option<String>,
) -> (Option<String>, bool) {
    if let Some(s) = explicit {
        return (Some(s), false);
    }
    if !is_team {
        return (None, false);
    }
    match (rotate, existing) {
        (false, Some(s)) => (Some(s), false),                     // reuse the running hub's secret
        _ => (Some(parler_hub::random_secret()), true),           // first --team, or --rotate-secret
    }
}

/// The join secret already wired for `hub_url` across this machine's hosts, if any — the source of
/// truth for reusing a `--team` secret on a re-run instead of minting a fresh one that would strand
/// the running hub (issue #101). Scans every known host's configured env and returns the first
/// non-empty secret whose `PARLER_HUB` matches the hub we're about to wire. `None` when nothing is
/// wired to this hub yet (the genuine first `--team`, where minting a new secret is correct).
fn existing_team_secret(hub_url: &str) -> Option<String> {
    registry().iter().find_map(|def| match configured_env(def) {
        Some((hub, Some(secret))) if hub == hub_url && !secret.is_empty() => Some(secret),
        _ => None,
    })
}

/// The hub URL saved in the bare `~/.parler/config.json` — the identity a manual `parler mcp` mints
/// when no `PARLER_HOME` is set. `None` if there's no such identity (the common first-run case) or it
/// can't be read. Used to decide whether the primary host can safely reuse that identity.
fn bare_identity_hub() -> Option<String> {
    let txt = std::fs::read_to_string(parler_root().join("config.json")).ok()?;
    let v: Value = serde_json::from_str(&txt).ok()?;
    v.get("hub_url").and_then(Value::as_str).map(String::from)
}

fn user_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The absolute path of the running binary — so the config points at *this* `parler` even when it
/// isn't on PATH (e.g. bundled inside the desktop app).
fn binary_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "parler".to_string())
}

/// Abbreviate `$HOME/...` to `~/...` for display.
fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy().into_owned();
    let home = user_home().to_string_lossy().into_owned();
    if !home.is_empty() && s.starts_with(&home) {
        return format!("~{}", &s[home.len()..]);
    }
    s
}

fn local_hub_cmd(port: u16) -> String {
    if port == 7070 {
        "parler hub --local".to_string()
    } else {
        format!("parler hub --local --addr 127.0.0.1:{port}")
    }
}

fn team_hub_cmd(port: u16, secret: &str) -> String {
    format!("parler hub --addr 0.0.0.0:{port} --db {}/hub.sqlite --join-secret {secret}", display_path(&parler_root()))
}

/// Best-effort primary LAN IP: open a UDP socket "toward" a public address and read the local end.
/// No packets are sent; it just makes the OS pick the egress interface.
fn detect_lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "parler-connect-test-{}-{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn env() -> Vec<(String, String)> {
        vec![
            ("PARLER_HOME".into(), "/home/u/.parler/agents/cursor".into()),
            ("PARLER_HUB".into(), "wss://parler-hub.fly.dev".into()),
            ("PARLER_NAME".into(), "cursor".into()),
        ]
    }

    #[test]
    fn json_write_is_idempotent_and_preserves_other_servers() {
        let path = tmp("cursor").join("mcp.json");
        // Seed with an unrelated server the user already had.
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"mcpServers":{"other":{"command":"foo"}}}"#).unwrap();

        write_json(&path, &env(), "/usr/local/bin/parler").unwrap();
        write_json(&path, &env(), "/usr/local/bin/parler").unwrap(); // twice → still one entry

        let v = read_json(&path).unwrap();
        let servers = v.get("mcpServers").unwrap().as_object().unwrap();
        assert!(servers.contains_key("other"), "must not clobber the user's other server");
        let parler = servers.get("parler").unwrap();
        assert_eq!(parler["command"], "/usr/local/bin/parler");
        assert_eq!(parler["args"][0], "mcp");
        assert_eq!(parler["env"]["PARLER_HUB"], "wss://parler-hub.fly.dev");
        // Exactly one parler entry (no duplication across runs).
        assert_eq!(servers.keys().filter(|k| *k == "parler").count(), 1);

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn toml_write_preserves_comments_and_other_tables() {
        let path = tmp("codex").join("config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "# my codex config\nmodel = \"o3\"\n\n[mcp_servers.other]\ncommand = \"foo\"\n",
        )
        .unwrap();

        write_toml(&path, &env(), "/usr/local/bin/parler").unwrap();
        write_toml(&path, &env(), "/usr/local/bin/parler").unwrap();

        let txt = std::fs::read_to_string(&path).unwrap();
        assert!(txt.contains("# my codex config"), "must preserve comments");
        assert!(txt.contains("model = \"o3\""), "must preserve top-level keys");
        assert!(txt.contains("[mcp_servers.other]"), "must preserve the user's other server");
        // Our entry parses back correctly and appears once.
        let doc: toml_edit::DocumentMut = txt.parse().unwrap();
        assert_eq!(doc["mcp_servers"]["parler"]["command"].as_str(), Some("/usr/local/bin/parler"));
        assert_eq!(doc["mcp_servers"]["parler"]["env"]["PARLER_HUB"].as_str(), Some("wss://parler-hub.fly.dev"));
        assert_eq!(txt.matches("[mcp_servers.parler]").count(), 1);

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn toml_write_works_on_a_fresh_file() {
        // Regression: on a doc with no `[mcp_servers.*]` yet, plain index-assignment used to leave
        // an empty inline `mcp_servers = {}` and silently drop our entry (fresh Codex install).
        let path = tmp("codex-fresh").join("config.toml");
        write_toml(&path, &env(), "/usr/local/bin/parler").unwrap();
        let txt = std::fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = txt.parse().unwrap();
        assert_eq!(doc["mcp_servers"]["parler"]["command"].as_str(), Some("/usr/local/bin/parler"));
        assert_eq!(doc["mcp_servers"]["parler"]["env"]["PARLER_HUB"].as_str(), Some("wss://parler-hub.fly.dev"));
        assert!(txt.contains("[mcp_servers.parler]"), "must render as a real table header");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn remove_json_drops_only_our_entry() {
        let path = tmp("rm-json").join("mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"mcpServers":{"other":{"command":"foo"}}}"#).unwrap();
        write_json(&path, &env(), "/usr/local/bin/parler").unwrap();

        assert!(remove_json(&path).unwrap(), "first remove reports it removed something");
        assert!(!remove_json(&path).unwrap(), "second remove is a no-op");

        let v = read_json(&path).unwrap();
        let servers = v.get("mcpServers").unwrap().as_object().unwrap();
        assert!(servers.contains_key("other"), "must keep the user's other server");
        assert!(!servers.contains_key("parler"), "our entry is gone");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn remove_toml_drops_only_our_table() {
        let path = tmp("rm-toml").join("config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# keep me\nmodel = \"o3\"\n\n[mcp_servers.other]\ncommand = \"foo\"\n").unwrap();
        write_toml(&path, &env(), "/usr/local/bin/parler").unwrap();

        assert!(remove_toml(&path).unwrap());
        assert!(!remove_toml(&path).unwrap());

        let txt = std::fs::read_to_string(&path).unwrap();
        assert!(txt.contains("# keep me"), "must preserve comments");
        assert!(txt.contains("[mcp_servers.other]"), "must keep the user's other server");
        assert!(!txt.contains("[mcp_servers.parler]"), "our table is gone");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn remove_json_on_missing_file_is_a_noop() {
        let path = tmp("rm-missing").join("mcp.json");
        assert!(!remove_json(&path).unwrap());
    }

    #[test]
    fn parse_env_from_text_reads_the_wired_values() {
        let text = "PARLER_HOME=/x\nPARLER_HUB=ws://127.0.0.1:7071\nPARLER_JOIN_SECRET=s3cr3t\nPARLER_NAME=cursor";
        assert_eq!(parse_env_from_text(text, "PARLER_HUB"), Some("ws://127.0.0.1:7071".to_string()));
        assert_eq!(parse_env_from_text(text, "PARLER_JOIN_SECRET"), Some("s3cr3t".to_string()));
        // Tolerate the `KEY: value` shape claude's human output uses.
        assert_eq!(
            parse_env_from_text("  PARLER_HUB: wss://parler-hub.fly.dev  ", "PARLER_HUB"),
            Some("wss://parler-hub.fly.dev".to_string())
        );
        assert_eq!(parse_env_from_text("nothing here", "PARLER_HUB"), None);
        assert_eq!(parse_env_from_text(text, "PARLER_ROLE"), None);
    }

    #[test]
    fn configured_env_reads_hub_and_secret_back() {
        // JSON host (cursor-style).
        let jpath = tmp("cfg-env-json").join("mcp.json");
        let mut env_with_secret = env();
        env_with_secret.push(("PARLER_JOIN_SECRET".into(), "shh".into()));
        write_json(&jpath, &env_with_secret, "/bin/parler").unwrap();
        let jdef = HostDef {
            id: "cursor",
            name: "Cursor",
            wiring: Wiring::Json(jpath.clone()),
            hints: vec![],
        };
        assert_eq!(
            configured_env(&jdef),
            Some(("wss://parler-hub.fly.dev".to_string(), Some("shh".to_string())))
        );

        // TOML host (codex-style), no secret.
        let tpath = tmp("cfg-env-toml").join("config.toml");
        write_toml(&tpath, &env(), "/bin/parler").unwrap();
        let tdef = HostDef {
            id: "codex",
            name: "Codex",
            wiring: Wiring::Toml(tpath.clone()),
            hints: vec![],
        };
        assert_eq!(configured_env(&tdef), Some(("wss://parler-hub.fly.dev".to_string(), None)));

        // Unconfigured → None.
        let missing = HostDef {
            id: "cursor",
            name: "Cursor",
            wiring: Wiring::Json(tmp("cfg-env-none").join("mcp.json")),
            hints: vec![],
        };
        assert_eq!(configured_env(&missing), None);

        std::fs::remove_dir_all(jpath.parent().unwrap()).ok();
        std::fs::remove_dir_all(tpath.parent().unwrap()).ok();
    }

    #[test]
    fn every_known_host_has_a_specific_restart_hint() {
        for d in registry() {
            assert_ne!(restart_hint(d.id), restart_hint("something-unknown"),
                "{} should have a per-host load hint", d.id);
        }
    }

    #[test]
    fn json_refuses_to_clobber_malformed_config() {
        let path = tmp("broken").join("mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ this is not json").unwrap();
        // Must error, and must NOT overwrite the file.
        assert!(write_json(&path, &env(), "/bin/parler").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ this is not json");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn hub_urls_resolve_as_expected() {
        assert_eq!(Hub::Shared.url(), "wss://parler-hub.fly.dev");
        assert_eq!(Hub::Local { port: 7070 }.url(), "ws://127.0.0.1:7070");
        assert_eq!(Hub::Team { port: 9000 }.url(), "ws://127.0.0.1:9000");
        assert_eq!(Hub::Explicit("ws://x:1".into()).url(), "ws://x:1");
    }

    #[test]
    fn aliases_map_to_canonical_ids() {
        assert_eq!(canonical_id("Claude"), Some("claude-code"));
        assert_eq!(canonical_id("claude_code"), Some("claude-code"));
        assert_eq!(canonical_id("CODEX"), Some("codex"));
        assert_eq!(canonical_id("gemini-cli"), Some("gemini"));
        assert_eq!(canonical_id("hermes"), None);
    }

    #[test]
    fn registry_ids_are_unique_and_paths_absolute() {
        let reg = registry();
        let mut ids: Vec<&str> = reg.iter().map(|d| d.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), reg.len(), "host ids must be unique");
        for d in &reg {
            if let Wiring::Json(p) | Wiring::Toml(p) = &d.wiring {
                assert!(p.is_absolute(), "{} config path must be absolute", d.id);
            }
        }
    }

    #[test]
    fn generic_snippet_carries_the_env() {
        let s = generic_snippet(&env(), "/bin/parler");
        assert!(s.contains("PARLER_HUB"));
        assert!(s.contains("/bin/parler mcp"));
    }

    #[test]
    fn env_includes_join_secret_only_when_present() {
        let opts = Options {
            hosts: vec![],
            hub: Hub::Shared,
            name: None,
            join_secret: None,
            print: false,
            list: false,
            remove: false,
            json: false,
            hub_pinned: false,
            rotate_secret: false,
        };
        let with = env_for("codex", &opts, "wss://h", Some("s3cr3t"), false);
        assert!(with.iter().any(|(k, v)| k == "PARLER_JOIN_SECRET" && v == "s3cr3t"));
        // Default name falls back to the agent id; no secret key when none is supplied.
        let without = env_for("codex", &opts, "wss://h", None, false);
        assert!(!without.iter().any(|(k, _)| k == "PARLER_JOIN_SECRET"));
        assert!(without.iter().any(|(k, v)| k == "PARLER_NAME" && v == "codex"));
    }

    #[test]
    fn env_for_adopts_bare_home_for_primary_but_isolates_the_rest() {
        let opts = Options {
            hosts: vec![],
            hub: Hub::Shared,
            name: None,
            join_secret: None,
            print: false,
            list: false,
            remove: false,
            json: false,
            hub_pinned: false,
            rotate_secret: false,
        };
        // A primary that adopts the bare identity points PARLER_HOME at ~/.parler itself, so
        // `parler mcp` loads the identity the user already minted there.
        let primary = env_for("claude-code", &opts, "wss://h", None, true);
        let home = primary.iter().find(|(k, _)| k == "PARLER_HOME").map(|(_, v)| v.clone()).unwrap();
        assert!(home.ends_with("/.parler"), "primary adopts the bare home, got {home}");
        assert!(!home.ends_with("/agents/claude-code"), "primary must not be under agents/, got {home}");
        // Every other host still gets its own per-agent identity under agents/<id>.
        let normal = env_for("claude-code", &opts, "wss://h", None, false);
        let home2 = normal.iter().find(|(k, _)| k == "PARLER_HOME").map(|(_, v)| v.clone()).unwrap();
        assert!(home2.ends_with("/.parler/agents/claude-code"), "non-primary gets a per-agent home, got {home2}");
    }

    // ---- #101: `--team` re-run reuses the secret; --rotate-secret mints a new one ----------------

    #[test]
    fn team_rerun_reuses_the_existing_secret() {
        // A second `--team` run with a secret already wired for this hub reuses it (minted = false),
        // so the running hub — still enforcing that secret — isn't stranded.
        let (secret, minted) =
            pick_team_secret(None, true, false, Some("EXISTING-SECRET".into()));
        assert_eq!(secret.as_deref(), Some("EXISTING-SECRET"));
        assert!(!minted, "reusing the wired secret must not count as a mint (no restart prompt)");
    }

    #[test]
    fn first_team_run_mints_and_rotate_forces_a_new_secret() {
        // First `--team` (nothing wired yet) mints a fresh secret…
        let (first, minted) = pick_team_secret(None, true, false, None);
        assert!(first.is_some() && minted, "first --team mints a secret");
        // …and `--rotate-secret` mints even when one already exists, and flags it minted so the
        // caller prints the hub-restart instruction.
        let (rot, minted) = pick_team_secret(None, true, true, Some("OLD".into()));
        assert!(minted, "--rotate-secret always mints");
        assert_ne!(rot.as_deref(), Some("OLD"), "rotation replaces the old secret");
    }

    #[test]
    fn explicit_secret_always_wins_and_non_team_has_none() {
        // An explicit `--join-secret` / PARLER_JOIN_SECRET passes through untouched (never minted),
        // even for a team hub; a non-team hub carries no secret.
        let (s, minted) = pick_team_secret(Some("MINE".into()), true, false, Some("wired".into()));
        assert_eq!(s.as_deref(), Some("MINE"));
        assert!(!minted);
        assert_eq!(pick_team_secret(None, false, false, None), (None, false));
    }

    #[test]
    fn existing_team_secret_reads_a_matching_wired_hub_and_keeps_perms_tight() {
        // `existing_team_secret` finds the secret wired for the target hub by scanning host configs.
        // Drive it through a real JSON host write so we also prove the config we author is 0600.
        let path = tmp("team-reuse").join("mcp.json");
        let mut with_secret = env();
        // Wire this host to a specific hub with a known secret.
        with_secret.retain(|(k, _)| k != "PARLER_HUB");
        with_secret.push(("PARLER_HUB".into(), "ws://127.0.0.1:7070".into()));
        with_secret.push(("PARLER_JOIN_SECRET".into(), "REUSE-ME".into()));
        write_json(&path, &with_secret, "/bin/parler").unwrap();
        let def =
            HostDef { id: "cursor", name: "Cursor", wiring: Wiring::Json(path.clone()), hints: vec![] };
        // Match on the same hub → the wired secret; a different hub → None (mint fresh).
        assert_eq!(
            configured_env(&def),
            Some(("ws://127.0.0.1:7070".to_string(), Some("REUSE-ME".to_string())))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "a config carrying a join secret must be owner-only");
        }
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    // ---- #100: no env-prefixed `claude mcp add` in code/docs ------------------------------------
    // (The `--team` printed one-liner's round-trip through the real arg/env parser is tested in
    //  `lib.rs`, where `ConnectArgs` is in scope.)

    #[test]
    fn no_env_prefixed_claude_mcp_add_remains_in_code_or_docs() {
        // Every printed/documented `claude mcp add` must use the `-e PARLER_X=…` flag form; a shell
        // env-prefix (`PARLER_X=… claude mcp add`) is silently dropped before `parler mcp` runs
        // (issue #100). Grep the tracked repo for the broken shape and fail if any real command line
        // still has it. Scratch logs (`tasks/`) and the research write-up that *documents* the defect
        // are excluded; comment lines (Rust `//`, prose `#`, `*`) that merely describe it are too.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let files = collect_scannable_files(&root);
        assert!(files.len() > 20, "sanity: found {} scannable files under repo root", files.len());
        let mut offenders = Vec::new();
        for f in &files {
            let rel = f.strip_prefix(&root).unwrap_or(f).to_string_lossy().replace('\\', "/");
            if rel.starts_with("tasks/") || rel.starts_with("docs/research/") {
                continue; // scratch log + the audit that names the anti-pattern
            }
            let Ok(text) = std::fs::read_to_string(f) else { continue };
            for line in text.lines() {
                let t = line.trim_start();
                // Skip comment / prose lines that merely mention the pattern.
                if t.starts_with("//") || t.starts_with('#') || t.starts_with('*') || t.starts_with("> ") {
                    continue;
                }
                if is_env_prefixed_claude_add(line) {
                    offenders.push(format!("{rel}: {}", line.trim()));
                }
            }
        }
        assert!(offenders.is_empty(), "env-prefixed `claude mcp add` lines still present:\n{}", offenders.join("\n"));
    }

    /// True when `line` has the broken `PARLER_X=<v> … claude mcp add` shell-env-prefix shape (an
    /// assignment appearing *before* `claude mcp add` on the same line).
    fn is_env_prefixed_claude_add(line: &str) -> bool {
        let Some(add_at) = line.find("claude mcp add") else { return false };
        let before = &line[..add_at];
        // An `-e PARLER_X=` immediately before `claude mcp add` is a flag on a *different* command,
        // not a prefix; the broken form is a bare `PARLER_X=<val> ` (no `-e ` in front) leading the
        // line. Look for `PARLER_<caps>=` in the prefix that isn't part of an `-e ` flag.
        let mut idx = 0;
        while let Some(pos) = before[idx..].find("PARLER_") {
            let at = idx + pos;
            let is_e_flag = before[..at].trim_end().ends_with("-e") || before[..at].trim_end().ends_with("--env");
            // The token must look like an assignment `PARLER_XXX=`.
            let rest = &before[at..];
            let assigns = rest
                .split_once('=')
                .map(|(k, _)| k.trim_end().chars().all(|c| c.is_ascii_uppercase() || c == '_' || c == 'R'))
                .unwrap_or(false);
            if assigns && !is_e_flag {
                return true;
            }
            idx = at + "PARLER_".len();
        }
        false
    }

    /// Gather scannable source/doc files under `root` (skips build output + vendored deps).
    fn collect_scannable_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                let name = e.file_name();
                let name = name.to_string_lossy();
                if p.is_dir() {
                    if matches!(name.as_ref(), "target" | "node_modules" | ".git" | ".next") {
                        continue;
                    }
                    walk(&p, out);
                } else if matches!(
                    p.extension().and_then(|x| x.to_str()),
                    Some("rs" | "md" | "ts" | "tsx" | "js" | "sh" | "html" | "toml")
                ) {
                    out.push(p);
                }
            }
        }
        let mut out = Vec::new();
        walk(root, &mut out);
        out
    }
}
