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
}

/// How a given MCP host stores its server config — the knowledge that used to be scattered across
/// docs and the Electron app, centralized so one code path serves the CLI *and* the desktop.
#[derive(Debug, Clone)]
enum Wiring {
    /// Claude Code — driven through its own `claude mcp add` CLI (the supported API).
    ClaudeCli,
    /// A JSON file with a top-level `mcpServers` object (Cursor, Windsurf, Gemini, Claude Desktop).
    Json(PathBuf),
    /// A TOML file with `[mcp_servers.<name>]` tables (Codex).
    Toml(PathBuf),
}

struct HostDef {
    id: &'static str,
    name: &'static str,
    wiring: Wiring,
    /// If any of these paths exists, the host is considered installed.
    hints: Vec<PathBuf>,
}

/// The MCP server name we register under, in every host.
const SERVER_NAME: &str = "parler";

// ---------------------------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------------------------

fn registry() -> Vec<HostDef> {
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

fn is_installed(def: &HostDef) -> bool {
    match &def.wiring {
        Wiring::ClaudeCli => resolve_claude().is_some() || def.hints.iter().any(|p| p.exists()),
        _ => def.hints.iter().any(|p| p.exists()),
    }
}

/// Whether this host already has a `parler` server configured (so `--list` can say "connected").
fn is_configured(def: &HostDef) -> bool {
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

/// The `PARLER_HUB` a host is currently wired to (so `--list --json` can tell the desktop app whether
/// an agent points at the local or the shared hub). Best-effort: `None` if unconfigured/unreadable.
fn configured_hub(def: &HostDef) -> Option<String> {
    match &def.wiring {
        Wiring::ClaudeCli => {
            let claude = resolve_claude()?;
            let out = Command::new(claude).args(["mcp", "get", SERVER_NAME]).output().ok()?;
            if !out.status.success() {
                return None;
            }
            parse_hub_from_text(&String::from_utf8_lossy(&out.stdout))
        }
        Wiring::Json(path) => read_json(path)
            .ok()?
            .get("mcpServers")?
            .get(SERVER_NAME)?
            .get("env")?
            .get("PARLER_HUB")?
            .as_str()
            .map(String::from),
        Wiring::Toml(path) => {
            let doc: toml_edit::DocumentMut = std::fs::read_to_string(path).ok()?.parse().ok()?;
            doc.get("mcp_servers")?
                .get(SERVER_NAME)?
                .get("env")?
                .get("PARLER_HUB")?
                .as_str()
                .map(String::from)
        }
    }
}

/// Pull the first `PARLER_HUB=<value>` (or `: <value>`) token out of free-form text — used to read the
/// hub back from `claude mcp get`'s human output without pulling in a regex dependency.
fn parse_hub_from_text(s: &str) -> Option<String> {
    let idx = s.find("PARLER_HUB")?;
    let rest = s[idx + "PARLER_HUB".len()..].trim_start_matches([':', '=', ' ', '"']);
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
    use toml_edit::DocumentMut;
    let mut doc: DocumentMut = if path.exists() {
        std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?
            .parse()
            .with_context(|| format!("{} isn't valid TOML — fix or move it, then re-run (or use --print)", path.display()))?
    } else {
        DocumentMut::new()
    };
    // Replace our table wholesale (idempotent) while leaving the user's other servers untouched.
    doc["mcp_servers"][SERVER_NAME] = parler_toml_item(env, binpath);
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
}

/// `parler connect` entry point.
pub fn run(opts: Options) -> Result<()> {
    let binpath = binary_path();
    let hub_url = opts.hub.url();
    // An explicit secret wins; otherwise `--team` mints one. Any other mode has no secret.
    let secret = opts
        .join_secret
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| matches!(opts.hub, Hub::Team { .. }).then(parler_hub::random_secret));

    if opts.list {
        return print_list(opts.json);
    }

    if opts.remove {
        return run_remove(&opts);
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
    for t in &targets {
        match t {
            Target::Known(def) => {
                let env = env_for(def.id, &opts, &hub_url, secret.as_deref());
                if opts.print {
                    snippets.push((def.name.to_string(), snippet_for(def, &env, &binpath)));
                    reports.push(Report {
                        id: def.id.into(),
                        name: def.name.into(),
                        status: "printed",
                        detail: "snippet printed".into(),
                        config: None,
                    });
                } else {
                    match wire(def, &env, &binpath) {
                        Ok(where_) => reports.push(Report {
                            id: def.id.into(),
                            name: def.name.into(),
                            status: "wired",
                            detail: where_.clone(),
                            config: Some(where_),
                        }),
                        Err(e) => reports.push(Report {
                            id: def.id.into(),
                            name: def.name.into(),
                            status: "error",
                            detail: format!("{e}"),
                            config: None,
                        }),
                    }
                }
            }
            Target::Unknown(name) => {
                let env = env_for(name, &opts, &hub_url, secret.as_deref());
                snippets.push((name.clone(), generic_snippet(&env, &binpath)));
                reports.push(Report {
                    id: name.clone(),
                    name: name.clone(),
                    status: "printed",
                    detail: "snippet printed".into(),
                    config: None,
                });
            }
        }
    }

    if opts.json {
        emit_json(&opts, &hub_url, secret.as_deref(), &binpath, &reports, &snippets);
    } else {
        emit_human(&opts, &hub_url, secret.as_deref(), &reports, &snippets);
    }
    Ok(())
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
                }),
                Ok(false) => reports.push(Report {
                    id: def.id.into(),
                    name: def.name.into(),
                    status: "not-configured",
                    detail: "wasn't connected".into(),
                    config: None,
                }),
                Err(e) => reports.push(Report {
                    id: def.id.into(),
                    name: def.name.into(),
                    status: "error",
                    detail: format!("{e}"),
                    config: None,
                }),
            },
            Target::Unknown(name) => reports.push(Report {
                id: name.clone(),
                name: name.clone(),
                status: "not-configured",
                detail: "unknown host — nothing to remove".into(),
                config: None,
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
fn env_for(id: &str, opts: &Options, hub_url: &str, secret: Option<&str>) -> Vec<(String, String)> {
    let home = parler_root().join("agents").join(id);
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

fn emit_human(opts: &Options, hub_url: &str, secret: Option<&str>, reports: &[Report], snippets: &[(String, String)]) {
    let where_ = match &opts.hub {
        Hub::Shared => "the shared hub".to_string(),
        Hub::Local { .. } => "a hub on THIS machine (nothing leaves the box)".to_string(),
        Hub::Team { .. } => "your team hub".to_string(),
        Hub::Explicit(_) => "the hub you named".to_string(),
    };
    println!("\nParler · connecting your agents to {where_}");
    println!("         {hub_url}\n");

    let installed_ids: Vec<&'static str> = registry().iter().filter(|d| is_installed(d)).map(|d| d.id).collect();
    for r in reports {
        let mark = match r.status {
            "wired" => "  ✓",
            "error" => "  ✗",
            _ => "  ·",
        };
        println!("{mark} {:<15} {}", r.name, r.detail);
    }
    // In an auto run, name the known hosts we skipped because they weren't detected.
    if opts.hosts.is_empty() && !opts.print {
        for def in registry() {
            if !installed_ids.contains(&def.id) {
                println!("  · {:<15} not detected — skipped   (run: parler connect {})", def.name, def.id);
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
        println!("\nRestart those apps to load Parler. Each agent gets its own identity under {}.", display_path(&parler_root().join("agents")));
    }

    // The hub-run + teammate instructions, only where they apply.
    match &opts.hub {
        Hub::Local { port } => {
            println!("\nStart the local hub in its own terminal (keep it running), then restart your agents:");
            println!("  {}", local_hub_cmd(*port));
        }
        Hub::Team { port } => {
            if let Some(s) = secret {
                println!("\nJoin secret (share out-of-band — not on a shared screen):\n  {s}");
                println!("\nRun your team hub (keep it running):\n  {}", team_hub_cmd(*port, s));
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
        .map(|r| json!({ "id": r.id, "name": r.name, "status": r.status, "detail": r.detail, "config": r.config }))
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
                let hub = connected.then(|| configured_hub(d)).flatten();
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
    fn parse_hub_from_text_reads_the_wired_hub() {
        assert_eq!(
            parse_hub_from_text("PARLER_HOME=/x\nPARLER_HUB=ws://127.0.0.1:7071\nPARLER_NAME=cursor"),
            Some("ws://127.0.0.1:7071".to_string())
        );
        // Tolerate the `KEY: value` shape claude's human output uses.
        assert_eq!(
            parse_hub_from_text("  PARLER_HUB: wss://parler-hub.fly.dev  "),
            Some("wss://parler-hub.fly.dev".to_string())
        );
        assert_eq!(parse_hub_from_text("nothing here"), None);
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
        };
        let with = env_for("codex", &opts, "wss://h", Some("s3cr3t"));
        assert!(with.iter().any(|(k, v)| k == "PARLER_JOIN_SECRET" && v == "s3cr3t"));
        // Default name falls back to the agent id; no secret key when none is supplied.
        let without = env_for("codex", &opts, "wss://h", None);
        assert!(!without.iter().any(|(k, _)| k == "PARLER_JOIN_SECRET"));
        assert!(without.iter().any(|(k, v)| k == "PARLER_NAME" && v == "codex"));
    }
}
