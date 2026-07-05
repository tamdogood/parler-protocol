//! parler-cli — the `parler` command-line surface.
//!
//! Every networked subcommand is a thin wrapper over [`parler_connector::MeshAgent`]: load the
//! local identity, connect to the hub, do one op, print. `parler hub` runs the bus in-process and
//! `parler mcp` exposes the same ops as MCP tools (see [`mcp`]).

pub mod connect;
pub mod mcp;

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};
use parler_connector::{verify_message, BundleMeta, Config, MeshAgent, SigStatus};
use parler_protocol::{
    is_message_sig_part, AgentSkill, BundleRef, DirectoryEntry, DiscoverScope, HandoffRef, Part,
    RoomKind, StoredMessage, Target, Visibility,
};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "parler",
    version,
    about = "Parler Protocol — the chat protocol for AI agents: 1:1 / many:1 / 1:many messaging + a shared memory store"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the hub (the message bus + memory store).
    Hub(HubArgs),
    /// Wire every AI agent on this machine to Parler Protocol in one step (Claude Code, Codex, Cursor, …).
    Connect(ConnectArgs),
    /// Create this agent's identity and point it at a hub (advanced; `connect`/`mcp` do this for you).
    Init(InitArgs),
    /// Mint an invite code/link to hand to another agent (default: a 1:1 DM).
    Invite(InviteArgs),
    /// Redeem a pasted invite code/link.
    Join {
        /// The code (or full link) the other agent gave you.
        code: String,
    },
    /// Join a service queue as a worker (many-to-one), then `recv` it for tasks.
    Serve {
        service: String,
    },
    /// Open or join a shared live session — hand a key to another agent mid-conversation.
    #[command(subcommand)]
    Session(SessionCmd),
    /// Publish this agent's discovery card to the hub directory (default: private).
    Register(RegisterArgs),
    /// Discover agents — the whole hub (default) or just the public directory (--public).
    Discover(DiscoverArgs),
    /// Show a single agent's directory card by id.
    Card {
        id: String,
    },
    /// Mint a directory token to paste into the website to view this hub's private directory.
    Token(TokenArgs),
    /// Send a message (one of --room / --to / --service).
    Send(SendArgs),
    /// Hand off the turn: post a structured "you're up next" so a watching agent continues.
    Handoff(HandoffArgs),
    /// Hand off code: bundle a git ref and push it to a room/peer/service.
    Push(PushArgs),
    /// Download a pushed bundle's bytes by its blob id.
    Fetch(FetchArgs),
    /// Apply a pushed bundle into the current git repo (imports into refs/parler/*; never merges).
    Apply(ApplyArgs),
    /// Pull new messages for a room (advances your cursor unless --since/--all).
    Recv(RecvArgs),
    /// Write a fact to the shared memory store.
    Remember(RememberArgs),
    /// Recall facts by full-text query (returns only relevant rows — low token cost).
    Recall(RecallArgs),
    /// List the rooms you belong to, with unread counts.
    Rooms,
    /// Show who is in a room.
    Roster {
        #[arg(long)]
        room: String,
    },
    /// Advertise your presence status.
    Presence {
        /// One of: idle | working | waiting | offline (free-form).
        status: String,
        #[arg(long)]
        activity: Option<String>,
    },
    /// Print this agent's identity and hub.
    Whoami,
    /// Run the MCP server (stdio) exposing the parler_* tools to an MCP host.
    Mcp,
    /// Check configuration, database, connections, and dependencies.
    Doctor,
    /// Handle a Claude Code / editor lifecycle hook.
    Hook {
        /// The hook type (session-start, pre-tool-use, post-tool-use, post-tool-use-failure, session-end).
        kind: String,
    },
    /// Consolidate the active session backlog into key semantic facts.
    Consolidate,
}

#[derive(Args)]
struct HubArgs {
    #[arg(long, env = "PARLER_HUB_ADDR", default_value = "127.0.0.1:7070")]
    addr: String,
    /// SQLite file for durable storage. Omit for in-memory (lost on exit).
    #[arg(long, env = "PARLER_HUB_DB")]
    db: Option<String>,
    /// Public base URL advertised in invite links. Defaults to `parler://<addr>`.
    #[arg(long, env = "PARLER_HUB_URL")]
    url: Option<String>,
    /// Display name for this hub (the workspace name shown in the directory/site).
    #[arg(long, env = "PARLER_HUB_NAME", default_value = "Parler Protocol Hub")]
    name: String,
    /// Run a public hub (world-readable directory). Omit for a private, token-gated hub.
    #[arg(long, env = "PARLER_HUB_PUBLIC")]
    public: bool,
    /// Require this shared secret on connect. Strongly recommended for a private hub exposed on a
    /// public URL — otherwise anyone who can reach it can join. Agents present it via
    /// `PARLER_JOIN_SECRET`.
    #[arg(long, env = "PARLER_HUB_JOIN_SECRET")]
    join_secret: Option<String>,
    /// Disconnect an authenticated agent after this many seconds of silence (it can reconnect and
    /// resume from its durable cursor). `0` disables the timeout. Default: 1800 (30 min).
    #[arg(long, env = "PARLER_HUB_IDLE_TIMEOUT_SECS", default_value_t = 1800)]
    idle_timeout_secs: u64,
    /// Convenience for a persistent hub on THIS machine: if `--db` is unset, store it at
    /// `~/.parler/hub.sqlite`. The default `--addr` already binds loopback, so nothing leaves the box
    /// and no join secret is needed. Pairs with `parler connect --local`.
    #[arg(long)]
    local: bool,
}

#[derive(Args)]
struct ConnectArgs {
    /// Specific agents to wire (e.g. `codex`, `cursor`, `hermes`). Omit to auto-detect and wire every
    /// agent found on this machine.
    hosts: Vec<String>,
    /// Keep everything on THIS machine: point agents at a local hub (nothing leaves the box).
    #[arg(long, conflicts_with_all = ["team", "hub", "shared"])]
    local: bool,
    /// Like `--local`, but reachable by teammates on your network; generates a join secret.
    #[arg(long, conflicts_with_all = ["local", "hub", "shared"])]
    team: bool,
    /// Move agents to the shared hub explicitly. (A bare `parler connect` keeps an already-wired
    /// agent on its current hub; this flag is how you move it back.)
    #[arg(long, conflicts_with_all = ["local", "team", "hub"])]
    shared: bool,
    /// Advanced: dial this explicit hub URL instead of the shared/local one. A bare `parler connect`
    /// (no `--local`/`--team`/`--shared`/`--hub`) also honors the `PARLER_HUB` env var, so the
    /// teammate one-liner `--team` prints (`PARLER_HUB=… PARLER_JOIN_SECRET=… parler connect`) works
    /// verbatim. (Read in `cmd_connect`, not via clap `env=`, so an *exported* `PARLER_HUB` never
    /// conflicts with an explicit `--local`/`--team`.)
    #[arg(long)]
    hub: Option<String>,
    /// Port for the `--local` / `--team` hub (default 7070).
    #[arg(long, default_value_t = 7070)]
    port: u16,
    /// Display-name base for this machine's agents (default: the agent id, e.g. `codex`).
    #[arg(long)]
    name: Option<String>,
    /// Join secret required by a secret-gated hub (pair with `--hub`). `--team` mints one for you.
    /// A bare `parler connect` also honors `PARLER_JOIN_SECRET` from env (read in `cmd_connect`
    /// alongside `PARLER_HUB`) so the printed teammate one-liner works verbatim.
    #[arg(long)]
    join_secret: Option<String>,
    /// Mint a fresh `--team` join secret instead of reusing this hub's existing one. Re-running
    /// `parler connect --team` reuses the secret by default so it doesn't strand the running hub;
    /// rotate deliberately with this flag, then restart the hub with the printed line.
    #[arg(long)]
    rotate_secret: bool,
    /// Don't write anything — just print the config snippet to paste yourself.
    #[arg(long)]
    print: bool,
    /// List detected agents and their current Parler Protocol status; write nothing.
    #[arg(long)]
    list: bool,
    /// Remove Parler Protocol from the named agents (or every configured one when none are named).
    #[arg(long)]
    remove: bool,
    /// Emit machine-readable JSON (used by the Parler Protocol desktop app).
    #[arg(long)]
    json: bool,
    /// After wiring, wait and report each agent as it dials the hub — restart your agents and watch
    /// them come online. (Human output only; ignored with --json.)
    #[arg(long)]
    verify: bool,
    /// How long --verify waits before giving up, in seconds.
    #[arg(long, default_value_t = 180)]
    verify_timeout_secs: u64,
}

#[derive(Subcommand)]
enum SessionCmd {
    /// Open a shared session and print a KEY to hand to other agents.
    Open {
        /// A recap of the conversation/state to seed the session with (posted as its first message).
        #[arg(long)]
        context: Option<String>,
        /// An optional short name for the session.
        #[arg(long)]
        topic: Option<String>,
        /// Don't require approval: anyone with the key joins immediately. By default joiners must be
        /// approved by you before they can read the conversation.
        #[arg(long)]
        no_approval: bool,
        /// How long the key stays valid, in seconds (default 86400).
        #[arg(long)]
        ttl: Option<u64>,
        /// How many agents may join with the key (default 50).
        #[arg(long)]
        max_uses: Option<u32>,
    },
    /// Join a shared session with a key and print the context so far (or a pending-approval notice).
    Join {
        /// The session key (or full link) you were given.
        key: String,
    },
    /// List the agents waiting for your approval to join a session you opened.
    Requests {
        /// The session room (defaults to your active session).
        #[arg(long)]
        room: Option<String>,
        /// Emit machine-readable JSON (used by the Parler Protocol desktop app): `{room, requests:[…]}`.
        #[arg(long)]
        json: bool,
    },
    /// Approve a pending joiner into a session you opened — they can then read it and participate.
    Approve {
        /// The session room (defaults to your active session).
        #[arg(long)]
        room: Option<String>,
        /// The id of the joiner to admit.
        agent: String,
    },
    /// Reject a pending joiner's request — they are turned away and cannot re-request.
    Deny {
        /// The session room (defaults to your active session).
        #[arg(long)]
        room: Option<String>,
        /// The id of the joiner to reject.
        agent: String,
    },
    /// Mint a read-only WATCH code for a session you opened — paste it into the website's session
    /// viewer to watch the conversation and how many agents are in the room (no joining). Owner-only.
    Watch {
        /// The session room (defaults to your active session).
        #[arg(long)]
        room: Option<String>,
        /// How long the watch code stays valid, in seconds (default 3600).
        #[arg(long)]
        ttl: Option<u64>,
    },
}

#[derive(Args)]
struct InitArgs {
    /// Hub address/URL (host:port, ws://, or parler://).
    #[arg(long, default_value = "parler://127.0.0.1:7070")]
    hub: String,
    /// Display name (defaults to $USER).
    #[arg(long)]
    name: Option<String>,
    /// The role this agent plays (planner, reviewer, …).
    #[arg(long)]
    role: Option<String>,
    /// Overwrite an existing identity.
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct InviteArgs {
    /// Create a group channel room (one-to-many) with this name.
    #[arg(long)]
    group: Option<String>,
    /// Create a service worker queue (many-to-one) with this name.
    #[arg(long)]
    service: Option<String>,
    /// Invite lifetime in seconds (default 86400).
    #[arg(long)]
    ttl: Option<u64>,
    /// How many agents may redeem it (channel/service only; a DM is always single-use).
    #[arg(long)]
    max_uses: Option<u32>,
    /// Require your approval before a redeemer joins (group rooms only). Without it, anyone with the
    /// code joins immediately.
    #[arg(long)]
    require_approval: bool,
}

#[derive(Args)]
struct SendArgs {
    /// Send to a channel room (one-to-many).
    #[arg(long)]
    room: Option<String>,
    /// Send a DM to a peer agent id (one-to-one).
    #[arg(long)]
    to: Option<String>,
    /// Send to a service queue (many-to-one).
    #[arg(long)]
    service: Option<String>,
    /// The message text.
    #[arg(required = true, trailing_var_arg = true)]
    text: Vec<String>,
}

#[derive(Args)]
struct HandoffArgs {
    /// Hand off into a channel room (one-to-many).
    #[arg(long)]
    room: Option<String>,
    /// Hand off as a DM to a peer agent id (one-to-one).
    #[arg(long)]
    to: Option<String>,
    /// Hand off to a service queue (many-to-one).
    #[arg(long)]
    service: Option<String>,
    /// What the next agent should do — the instruction to act on.
    #[arg(long)]
    next: String,
    /// A recap of what you just finished / the current state (gives the next agent context).
    #[arg(long)]
    summary: Option<String>,
    /// Address the handoff to a specific agent by name or role (default: anyone in the room).
    #[arg(long = "for", value_name = "WHO")]
    for_who: Option<String>,
    /// Attach a code bundle by blob id (from a prior `parler push`).
    #[arg(long)]
    bundle: Option<String>,
}

#[derive(Args)]
struct PushArgs {
    /// Push to a channel room (one-to-many).
    #[arg(long)]
    room: Option<String>,
    /// Push a DM to a peer agent id (one-to-one).
    #[arg(long)]
    to: Option<String>,
    /// Push to a service queue (many-to-one).
    #[arg(long)]
    service: Option<String>,
    /// Only bundle commits after this base ref (a thin patch series, e.g. origin/main).
    #[arg(long)]
    base: Option<String>,
    /// One-line summary (defaults to the tip commit subject).
    #[arg(long)]
    summary: Option<String>,
    /// An optional note posted alongside the bundle.
    #[arg(long)]
    note: Option<String>,
    /// The git ref/tip to bundle (default: HEAD).
    #[arg(default_value = "HEAD")]
    gitref: String,
}

#[derive(Args)]
struct FetchArgs {
    /// The blob id (from a `com.parler.bundle` message).
    blob: String,
    /// Output file (default: <blob-prefix>.bundle).
    #[arg(long, short = 'o')]
    out: Option<String>,
}

#[derive(Args)]
struct ApplyArgs {
    /// The blob id (from a `com.parler.bundle` message).
    blob: String,
}

#[derive(Args)]
struct RecvArgs {
    #[arg(long)]
    room: Option<String>,
    /// Pull messages with seq greater than this (does not advance your cursor).
    #[arg(long)]
    since: Option<i64>,
    /// Re-read the full history (equivalent to --since 0).
    #[arg(long)]
    all: bool,
    #[arg(long)]
    limit: Option<u32>,
    /// Stay connected and print messages the moment they arrive (sub-second push). Falls back to
    /// polling if the hub doesn't support push. Ctrl-C to stop.
    #[arg(long)]
    watch: bool,
}

#[derive(Args)]
struct RememberArgs {
    /// A stable key — re-remembering the same key overwrites (idempotent).
    #[arg(long)]
    key: Option<String>,
    /// Scope the fact to a room (default: your private memory).
    #[arg(long)]
    room: Option<String>,
    #[arg(required = true, trailing_var_arg = true)]
    text: Vec<String>,
}

#[derive(Args)]
struct RecallArgs {
    /// Limit the search to a room (default: all your reachable memory).
    #[arg(long)]
    room: Option<String>,
    #[arg(long)]
    limit: Option<u32>,
    #[arg(required = true, trailing_var_arg = true)]
    query: Vec<String>,
}

#[derive(Args)]
struct RegisterArgs {
    /// Make this agent discoverable by anyone (public directory). Default: private (same-hub only).
    #[arg(long)]
    public: bool,
    /// A capability tag (repeatable): --tag planning --tag ops.
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// A skill id (repeatable): --skill code-review.
    #[arg(long = "skill")]
    skills: Vec<String>,
    /// A short description of what this agent does.
    #[arg(long)]
    describe: Option<String>,
}

#[derive(Args)]
struct DiscoverArgs {
    /// Search only the public directory (default: the whole hub).
    #[arg(long)]
    public: bool,
    /// Filter by a capability tag.
    #[arg(long)]
    tag: Option<String>,
    /// Filter by a skill.
    #[arg(long)]
    skill: Option<String>,
    /// Filter by presence status (idle/working/waiting/offline).
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    limit: Option<u32>,
    /// Free-text query over name / tags / skills.
    #[arg(trailing_var_arg = true)]
    query: Vec<String>,
}

#[derive(Args)]
struct TokenArgs {
    /// Token lifetime in seconds (default 3600).
    #[arg(long)]
    ttl: Option<u64>,
}

/// Entry point for the `parler` binary.
pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Hub(a) => cmd_hub(a).await,
        Cmd::Connect(a) => cmd_connect(a).await,
        Cmd::Init(a) => cmd_init(a),
        Cmd::Invite(a) => cmd_invite(a).await,
        Cmd::Join { code } => cmd_join(code).await,
        Cmd::Serve { service } => cmd_serve(service).await,
        Cmd::Session(c) => cmd_session(c).await,
        Cmd::Register(a) => cmd_register(a).await,
        Cmd::Discover(a) => cmd_discover(a).await,
        Cmd::Card { id } => cmd_card(id).await,
        Cmd::Token(a) => cmd_token(a).await,
        Cmd::Send(a) => cmd_send(a).await,
        Cmd::Handoff(a) => cmd_handoff(a).await,
        Cmd::Push(a) => cmd_push(a).await,
        Cmd::Fetch(a) => cmd_fetch(a).await,
        Cmd::Apply(a) => cmd_apply(a).await,
        Cmd::Recv(a) => cmd_recv(a).await,
        Cmd::Remember(a) => cmd_remember(a).await,
        Cmd::Recall(a) => cmd_recall(a).await,
        Cmd::Rooms => cmd_rooms().await,
        Cmd::Roster { room } => cmd_roster(room).await,
        Cmd::Presence { status, activity } => cmd_presence(status, activity).await,
        Cmd::Whoami => cmd_whoami(),
        Cmd::Mcp => mcp::serve_stdio().await,
        Cmd::Doctor => cmd_doctor().await,
        Cmd::Hook { kind } => cmd_hook(kind).await,
        Cmd::Consolidate => cmd_consolidate().await,
    }
}

async fn connect() -> Result<MeshAgent> {
    // One env/config precedence rule for the whole CLI: hub/name/role resolve through the same
    // `explicit env > saved config > default` helper the MCP server uses, so `parler` and
    // `parler mcp` on the same machine can never dial different hubs (issue #99).
    let cfg = match Config::load() {
        Ok(c) => mcp::apply_env_overrides(c),
        Err(e) => {
            let fallback_path = std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| parler_connector::home_dir())
                .join(".parler-codex")
                .join("config.json");
            if fallback_path.exists() {
                std::env::set_var("PARLER_HOME", fallback_path.parent().unwrap());
                mcp::apply_env_overrides(Config::load().map_err(|_| e)?)
            } else if !Config::exists() {
                // Zero-setup, same as `parler mcp`: mint an identity on first use (already env-aware)
                // instead of telling the user to go run `parler init` first.
                let cfg = mcp::load_or_bootstrap_config()?;
                eprintln!(
                    "✱ first run — created your identity at {} (hub: {})",
                    parler_connector::home_dir().display(),
                    cfg.hub_url
                );
                cfg
            } else {
                return Err(e);
            }
        }
    };
    MeshAgent::connect(&cfg).await.map_err(|e| {
        anyhow::anyhow!("{e} (run `parler doctor` to troubleshoot)")
    })
}

async fn cmd_hub(a: HubArgs) -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .try_init();
    // `--local` makes a bare `parler hub` persistent: default the db under ~/.parler so a loopback
    // hub survives restarts (agents resume from their durable cursors) without extra flags.
    let db = if a.local && a.db.is_none() {
        Some(parler_connector::home_dir().join("hub.sqlite").to_string_lossy().into_owned())
    } else {
        a.db.clone()
    };
    let store = parler_hub::Store::open(db.as_deref().map(std::path::Path::new))?;
    let public_url = a.url.unwrap_or_else(|| format!("parler://{}", a.addr));
    let mode = if a.public { parler_hub::HubMode::Public } else { parler_hub::HubMode::Private };
    let mut state = parler_hub::HubState::new(store, public_url, a.name, mode);
    if let Some(db) = &db {
        state.blob_dir = std::path::PathBuf::from(format!("{db}.blobs"));
    }
    state.join_secret = a.join_secret.filter(|s| !s.is_empty());
    state.idle_timeout =
        (a.idle_timeout_secs != 0).then(|| std::time::Duration::from_secs(a.idle_timeout_secs));
    let state = Arc::new(state);
    let listener = tokio::net::TcpListener::bind(&a.addr).await?;
    let actual = listener.local_addr()?;
    println!(
        "parler-hub up · ws://{actual}/ws · {} hub '{}' · db: {}",
        state.mode.as_str(),
        state.name,
        db.as_deref().unwrap_or(":memory:")
    );
    parler_hub::serve(listener, state).await
}

/// The hub-selection inputs to [`resolve_connect_hub`] — the `--hub`/`--shared`/`--team`/`--local`
/// flags plus the `PARLER_HUB`/`PARLER_JOIN_SECRET` env values (read by the caller). Grouped into a
/// struct so the resolver stays a single, testable function.
struct HubInputs {
    hub_flag: Option<String>,
    shared: bool,
    team: bool,
    local: bool,
    port: u16,
    join_secret_flag: Option<String>,
    env_hub: Option<String>,
    env_secret: Option<String>,
}

/// Resolve the hub topology + join secret + pinned flag for a `parler connect` run, applying the
/// env-var honoring the teammate one-liner relies on. Pure over its inputs (env is read by the
/// caller) so the precedence is unit-testable (issue #100):
///   * an explicit `--hub`/`--local`/`--team`/`--shared` always wins over env;
///   * only a **bare** `parler connect` (no hub flag) adopts `PARLER_HUB` from env — so an exported
///     `PARLER_HUB` never conflicts with an intentional `--local`/`--team`;
///   * `--join-secret` wins over `PARLER_JOIN_SECRET`.
///
/// Returns `(hub, join_secret, hub_pinned)`.
fn resolve_connect_hub(i: HubInputs) -> (connect::Hub, Option<String>, bool) {
    let no_hub_flag = !i.shared && !i.team && !i.local && i.hub_flag.is_none();
    let hub_arg = i.hub_flag.or(if no_hub_flag { i.env_hub } else { None });
    let join_secret = i.join_secret_flag.or(i.env_secret);
    // An env-provided PARLER_HUB counts as pinning (the teammate is deliberately moving to the team
    // hub); a truly bare run with no env stays unpinned and keeps already-wired agents in place.
    let hub_pinned = i.shared || i.team || i.local || hub_arg.is_some();
    let hub = if let Some(u) = hub_arg {
        connect::Hub::Explicit(u)
    } else if i.team {
        connect::Hub::Team { port: i.port }
    } else if i.local {
        connect::Hub::Local { port: i.port }
    } else {
        connect::Hub::Shared
    };
    (hub, join_secret, hub_pinned)
}

async fn cmd_connect(a: ConnectArgs) -> Result<()> {
    // The teammate one-liner `--team` prints is `PARLER_HUB=… PARLER_JOIN_SECRET=… parler connect`
    // (no flags). Honor those env vars **only when no hub-mode flag is given** (issue #100), read
    // here rather than via clap `env=` so an *exported* `PARLER_HUB` never conflicts with an explicit
    // `--local`/`--team`.
    let env_hub = std::env::var("PARLER_HUB").ok().filter(|s| !s.is_empty());
    let env_secret = std::env::var("PARLER_JOIN_SECRET").ok().filter(|s| !s.is_empty());
    let (hub, join_secret, hub_pinned) = resolve_connect_hub(HubInputs {
        hub_flag: a.hub.clone(),
        shared: a.shared,
        team: a.team,
        local: a.local,
        port: a.port,
        join_secret_flag: a.join_secret.clone(),
        env_hub,
        env_secret,
    });
    // Remember whether this is a `--local` run (and its port) — after wiring, we offer to start the
    // loopback hub so the user doesn't have to babysit a foreground terminal (issue #102).
    let started_local = (a.local, a.port);
    let interactive = !a.json && !a.print && !a.list && !a.remove;
    let wired = connect::run(connect::Options {
        hosts: a.hosts,
        hub,
        name: a.name,
        join_secret,
        print: a.print,
        list: a.list,
        remove: a.remove,
        json: a.json,
        hub_pinned,
        rotate_secret: a.rotate_secret,
    })?;
    if a.json || wired.is_empty() {
        return Ok(());
    }
    // For `--local`, offer to bring the loopback hub up detached (db under ~/.parler) so the user
    // never has to keep a terminal open — the minimum bar the flow audit set (issue #102).
    if interactive {
        if let (true, port) = started_local {
            maybe_start_local_hub(port);
        }
    }
    if a.verify {
        verify_dial_in(wired, Duration::from_secs(a.verify_timeout_secs)).await?;
    } else {
        // A bare `connect` still confirms the hub is actually reachable *now*, so a wrong URL / a
        // hub that isn't running / an unwritable identity dir surfaces here instead of as silent
        // failure after the user restarts their agents. It does not wait for the agents to dial in —
        // that's what `--verify` is for.
        probe_hubs(&wired).await;
    }
    Ok(())
}

/// The tail of a bare `parler connect`: dial each hub the agents were wired to **once** (short
/// timeout) to prove reachability, then return. Not a substitute for `--verify` (which waits for the
/// agents themselves to come online) — just a fast "is this hub actually up?" so failures aren't silent.
async fn probe_hubs(wired: &[connect::WiredAgent]) {
    use std::collections::BTreeSet;
    let hubs: BTreeSet<(String, Option<String>)> =
        wired.iter().map(|w| (w.hub.clone(), w.secret.clone())).collect();
    for (hub, secret) in hubs {
        // A gated hub needs the same join secret the agents were handed.
        match &secret {
            Some(s) => std::env::set_var("PARLER_JOIN_SECRET", s),
            None => std::env::remove_var("PARLER_JOIN_SECRET"),
        }
        let cfg = match mcp::load_or_bootstrap_config() {
            Ok(mut c) => {
                c.hub_url = hub.clone();
                c
            }
            Err(e) => {
                println!("  ⚠ couldn't prepare a local identity to test {hub}: {e}");
                continue;
            }
        };
        match tokio::time::timeout(Duration::from_secs(3), MeshAgent::connect(&cfg)).await {
            Ok(Ok(_)) => println!("  ✓ hub reachable — {hub}"),
            Ok(Err(e)) => report_unreachable(&hub, &e.to_string()),
            Err(_) => report_unreachable(&hub, "timed out after 3s"),
        }
    }
}

fn report_unreachable(hub: &str, err: &str) {
    println!("  ⚠ hub not reachable yet — {hub}: {err} (run `parler doctor` to troubleshoot)");
    if hub.contains("127.0.0.1") || hub.contains("localhost") {
        println!("     start it and keep it running:  parler hub --local");
    } else {
        println!("     the wiring is saved — your agents will connect once the hub is reachable.");
    }
}

/// After a `--local` wire, bring the loopback hub up **detached** if it isn't already listening, so
/// the user never has to keep a foreground terminal alive (issue #102). Best-effort and quiet on the
/// happy path: if the hub is already up we say nothing, and if spawning fails we fall back to
/// printing the manual start line rather than erroring. The child stores its db under `~/.parler`
/// (via `parler hub --local`) and outlives this process; we print how to stop it.
fn maybe_start_local_hub(port: u16) {
    // Already listening? Then a previous run (or the desktop app) started it — leave it alone.
    if std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().expect("loopback addr"),
        Duration::from_millis(300),
    )
    .is_ok()
    {
        println!("\nLocal hub already running on 127.0.0.1:{port} — nothing to start.");
        return;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("parler"));
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("hub").arg("--local");
    if port != 7070 {
        cmd.arg("--addr").arg(format!("127.0.0.1:{port}"));
    }
    // Detach: no inherited stdio, so the child neither blocks this terminal nor writes over its
    // output. It outlives `parler connect` (which exits right after the reachability probe below).
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match cmd.spawn() {
        Ok(child) => {
            println!("\n✓ started your local hub in the background (pid {}) — db under {}.",
                child.id(),
                parler_connector::home_dir().display());
            println!("  stop it later with:  kill {}", child.id());
            println!("  or run it yourself in a terminal instead:  parler hub --local");
        }
        Err(e) => {
            println!("\n⚠ couldn't auto-start the local hub ({e}).");
            println!("  start it yourself and keep it running:  parler hub --local");
        }
    }
}

/// The `--verify` tail of `parler connect`: dial each hub the agents were wired to and report every
/// agent the moment it first authenticates (its `Hello` upserts it into the hub directory). Closes
/// the loop that used to end at "restart them" with silence.
async fn verify_dial_in(wired: Vec<connect::WiredAgent>, timeout: Duration) -> Result<()> {
    use std::collections::BTreeMap;
    // One dial per (hub, secret); a gated hub needs the same join secret the agents were given.
    let mut by_hub: BTreeMap<(String, Option<String>), Vec<String>> = BTreeMap::new();
    for w in wired {
        by_hub.entry((w.hub, w.secret)).or_default().push(w.name);
    }
    println!("Waiting for your agents to dial in — restart them now (Ctrl-C to stop waiting).");
    let started = std::time::Instant::now();
    for ((hub, secret), mut names) in by_hub {
        match &secret {
            Some(s) => std::env::set_var("PARLER_JOIN_SECRET", s),
            None => std::env::remove_var("PARLER_JOIN_SECRET"),
        }
        let mut cfg = mcp::load_or_bootstrap_config()?;
        cfg.hub_url = hub.clone();
        let mut ag = match MeshAgent::connect(&cfg).await {
            Ok(ag) => ag,
            Err(e) => {
                println!("  ✗ can't reach {hub}: {e} (run `parler doctor` to troubleshoot)");
                if hub.contains("127.0.0.1") || hub.contains("localhost") {
                    println!("    (is your local hub running? start it with: parler hub --local)");
                }
                continue;
            }
        };
        while !names.is_empty() && started.elapsed() < timeout {
            let seen = ag
                .discover(DiscoverScope::Hub, None, None, None, None, Some(500))
                .await
                .unwrap_or_default();
            names.retain(|n| {
                let online = seen.iter().any(|e| e.card.name.eq_ignore_ascii_case(n));
                if online {
                    println!("  ✓ {n} dialed in ({}s)", started.elapsed().as_secs());
                }
                !online
            });
            if names.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        for n in &names {
            println!("  ⏳ {n} hasn't dialed in after {}s — restart it, run `parler doctor` to troubleshoot, or check later: parler connect --list", timeout.as_secs());
        }
    }
    Ok(())
}

fn cmd_init(a: InitArgs) -> Result<()> {
    if Config::exists() && !a.force {
        bail!("already initialized — pass --force to overwrite the existing identity");
    }
    let name = a
        .name
        .unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "agent".into()));
    let cfg = Config::create(a.hub, name, a.role)?;
    cfg.save()?;
    println!("✓ identity created");
    println!("  id:   {}", cfg.identity.id);
    println!(
        "  name: {}{}",
        cfg.name,
        cfg.role.as_deref().map(|r| format!(" ({r})")).unwrap_or_default()
    );
    println!("  hub:  {}", cfg.hub_url);
    println!("  saved to {}/config.json", parler_connector::home_dir().display());
    Ok(())
}

async fn cmd_invite(a: InviteArgs) -> Result<()> {
    if a.group.is_some() && a.service.is_some() {
        bail!("--group and --service are mutually exclusive");
    }
    let (kind, room) = if let Some(g) = a.group {
        (RoomKind::Channel, Some(g))
    } else if let Some(s) = a.service {
        (RoomKind::Service, Some(s))
    } else {
        (RoomKind::Dm, None)
    };
    let mut ag = connect().await?;
    let inv = ag
        .invite_with_approval(kind, room, a.ttl, a.max_uses, a.require_approval)
        .await?;
    println!("✓ invite ready — {} room '{}'", inv.kind.as_str(), inv.room);
    println!();
    println!("    code: {}", inv.code);
    println!("    link: {}", inv.url);
    println!();
    if a.require_approval && inv.kind == RoomKind::Channel {
        println!("Redeemers must be approved by you first:");
        println!("  parler session requests --room {0}    parler session approve --room {0} <id>", inv.room);
        println!();
    }
    println!("Hand it to another agent and have it run:  parler join {}", inv.code);
    Ok(())
}

async fn cmd_join(code: String) -> Result<()> {
    let mut ag = connect().await?;
    let (room, kind) = ag.join(&code).await?;
    println!("✓ joined {} room '{}'", kind.as_str(), room);
    println!("  receive with:  parler recv --room {room}");
    Ok(())
}

async fn cmd_serve(service: String) -> Result<()> {
    let mut ag = connect().await?;
    let room = ag.serve(&service).await?;
    println!("✓ serving '{service}' (room '{room}')");
    println!("  receive tasks with:  parler recv --room {room}");
    Ok(())
}

async fn cmd_session(c: SessionCmd) -> Result<()> {
    let mut ag = connect().await?;
    match c {
        SessionCmd::Open { context, topic, no_approval, ttl, max_uses } => {
            let require_approval = !no_approval;
            let inv = ag
                .invite_with_approval(RoomKind::Channel, topic, ttl, max_uses, require_approval)
                .await?;
            // Seed the room with the context snapshot so a late joiner catches up by reading history.
            if let Some(ctx) = context.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                let seed = format!("📋 session context (from {}):\n{ctx}", ag.name);
                ag.send_text(Target::Room { room: inv.room.clone() }, &seed).await?;
            }
            save_active_session(&inv.room)?;
            println!("✓ session open — room '{}'", inv.room);
            println!();
            println!("    KEY:  {}", inv.code);
            println!("    link: {}", inv.url);
            println!();
            if require_approval {
                println!("Joiners need your approval before they can read the conversation.");
                println!("  see who's waiting:  parler session requests --room {}", inv.room);
                println!("  approve / reject:   parler session approve --room {0} <id>  |  parler session deny --room {0} <id>", inv.room);
                println!();
            }
            println!("Hand the key to another agent:  parler session join {}", inv.code);
            println!("…or launch its MCP server with env  PARLER_SESSION_KEY={}", inv.code);
            println!();
            println!("Watch it live in your browser:  parler session watch --room {}", inv.room);
        }
        SessionCmd::Join { key } => {
            // An approval-gated session holds us as a pending request until the host admits us.
            let room = match ag.redeem(&key).await? {
                parler_connector::JoinOutcome::Joined { room, .. } => room,
                parler_connector::JoinOutcome::Pending { room } => {
                    println!("⏳ join request sent — waiting for the host to approve you into '{room}'.");
                    println!("You can't see the conversation until then. Re-run this to check:");
                    println!("  parler session join {key}");
                    return Ok(());
                }
            };
            // since=None advances the fresh cursor to the live edge after backfilling the context.
            let (msgs, _cursor) = ag.pull(&room, None, None).await?;
            save_active_session(&room)?;
            println!("✓ joined session — room '{room}'");
            if msgs.is_empty() {
                println!("(no prior context yet)");
            } else {
                println!("--- context so far ---");
                for m in &msgs {
                    println!("{}", render_message(m));
                }
                println!("--- end context ---");
            }
            println!("send with:  parler send --room {room} \"…\"    receive with:  parler recv --room {room}");
        }
        SessionCmd::Requests { room, json } => {
            let room = session_room(room)?;
            let reqs = ag.join_requests(&room).await?;
            if json {
                // Stable machine-readable shape for the desktop app. `JoinRequest` already
                // serializes as `{agent, name, role?, requestedAt}`.
                println!(
                    "{}",
                    serde_json::json!({ "room": room, "requests": reqs })
                );
                return Ok(());
            }
            if reqs.is_empty() {
                println!("(no agents waiting to join '{room}')");
                return Ok(());
            }
            println!("{} agent(s) waiting to join '{room}':", reqs.len());
            for r in &reqs {
                let role = r.role.as_deref().map(|x| format!(" ({x})")).unwrap_or_default();
                println!("  • {}{role}  {}", r.name, r.agent);
            }
            println!("approve:  parler session approve --room {room} <id>     deny:  parler session deny --room {room} <id>");
        }
        SessionCmd::Approve { room, agent } => {
            let room = session_room(room)?;
            ag.resolve_join(&room, &agent, true).await?;
            println!("✓ approved {agent} into '{room}' — they can now read the conversation and participate.");
        }
        SessionCmd::Deny { room, agent } => {
            let room = session_room(room)?;
            ag.resolve_join(&room, &agent, false).await?;
            println!("✓ denied {agent}'s request to join '{room}'.");
        }
        SessionCmd::Watch { room, ttl } => {
            let room = session_room(room)?;
            let (token, expires_at) = ag.mint_watch_token(&room, ttl).await?;
            println!("✓ read-only watch code for '{room}' (expires at {expires_at}):");
            println!();
            println!("    {token}");
            println!();
            println!("Paste it into the Parler Protocol website's session viewer (the /session page) to watch the");
            println!("conversation and how many agents are in the room — read-only, no joining. Anyone with");
            println!("this code can read the session, so share it like a password.");
        }
    }
    Ok(())
}

/// Resolve an optional `--room` for the session subcommands: explicit wins, else the active
/// session (parity with the MCP tools, which have defaulted this way from day one).
fn session_room(room: Option<String>) -> Result<String> {
    room.or_else(load_active_session)
        .ok_or_else(|| anyhow::anyhow!("specify --room, or open/join a session first"))
}

async fn cmd_register(a: RegisterArgs) -> Result<()> {
    let visibility = if a.public { Visibility::Public } else { Visibility::Private };
    let skills = a
        .skills
        .into_iter()
        .map(|s| AgentSkill { id: s.clone(), name: s, description: None })
        .collect();
    let mut ag = connect().await?;
    let (visibility, verified) = ag.register(visibility, a.tags, skills, a.describe).await?;
    let sig = if verified { "signature verified ✓" } else { "unsigned" };
    println!("✓ registered in the directory as {} ({sig})", visibility.as_str());
    println!("  discover with:  parler discover{}", if visibility == Visibility::Public { " --public" } else { "" });
    Ok(())
}

async fn cmd_discover(a: DiscoverArgs) -> Result<()> {
    let scope = if a.public { DiscoverScope::Public } else { DiscoverScope::Hub };
    let query = (!a.query.is_empty()).then(|| a.query.join(" "));
    let mut ag = connect().await?;
    let agents = ag.discover(scope, query, a.tag, a.skill, a.status, a.limit).await?;
    if agents.is_empty() {
        println!("(no agents found)");
        return Ok(());
    }
    let scope_label = if a.public { "public directory" } else { "hub" };
    println!("{} agent(s) in the {scope_label}:", agents.len());
    for e in &agents {
        println!("{}", render_entry(e));
    }
    Ok(())
}

async fn cmd_card(id: String) -> Result<()> {
    let mut ag = connect().await?;
    match ag.lookup(&id).await? {
        Some(e) => print!("{}", render_entry_full(&e)),
        None => println!("(no directory card for '{id}')"),
    }
    Ok(())
}

async fn cmd_token(a: TokenArgs) -> Result<()> {
    let mut ag = connect().await?;
    let (token, expires_at) = ag.mint_directory_token(a.ttl).await?;
    println!("✓ directory token (expires at {expires_at}):");
    println!();
    println!("    {token}");
    println!();
    println!("Paste it into the website's \"hub view\" to browse this hub's private directory.");
    Ok(())
}

/// Resolve the `--room`/`--to`/`--service` trio into exactly one [`Target`].
fn target_from(room: Option<String>, to: Option<String>, service: Option<String>) -> Result<Target> {
    match (room, to, service) {
        (Some(r), None, None) => Ok(Target::Room { room: r }),
        (None, Some(t), None) => Ok(Target::Dm { agent: t }),
        (None, None, Some(s)) => Ok(Target::Service { service: s }),
        (None, None, None) => {
            if let Some(active_room) = load_active_session() {
                Ok(Target::Room { room: active_room })
            } else {
                bail!("specify a destination: --room, --to, or --service (or open/join a session first)")
            }
        }
        _ => bail!("specify exactly one of --room, --to, --service"),
    }
}

/// True when `s` parses as an nkey public key (an agent id); anything else is treated as a name.
fn looks_like_agent_id(s: &str) -> bool {
    nkeys::KeyPair::from_public_key(s).is_ok()
}

/// Let `--to` take a directory *name* as well as a full id: a non-id value is resolved against the
/// hub directory and must match exactly one agent (case-insensitive on the card name). Kills the
/// "copy a 56-char key between terminals" step for the common case.
pub(crate) async fn resolve_target(ag: &mut MeshAgent, target: Target) -> Result<Target> {
    let Target::Dm { agent } = &target else { return Ok(target) };
    if looks_like_agent_id(agent) {
        return Ok(target);
    }
    // Try the hub's free-text query first; fall back to a plain listing in case the query
    // tokenization misses an exact name.
    let mut found = ag
        .discover(DiscoverScope::Hub, Some(agent.clone()), None, None, None, Some(50))
        .await?;
    if !found.iter().any(|e| e.card.name.eq_ignore_ascii_case(agent)) {
        found = ag.discover(DiscoverScope::Hub, None, None, None, None, Some(500)).await?;
    }
    let hits: Vec<&DirectoryEntry> =
        found.iter().filter(|e| e.card.name.eq_ignore_ascii_case(agent)).collect();
    match hits.len() {
        1 => {
            eprintln!("→ '{agent}' is {}", hits[0].card.id);
            Ok(Target::Dm { agent: hits[0].card.id.clone() })
        }
        0 => bail!("no agent named '{agent}' on this hub — check `parler discover`, or pass the full agent id"),
        _ => {
            let list = hits
                .iter()
                .map(|e| format!("  {}  {}", e.card.name, e.card.id))
                .collect::<Vec<_>>()
                .join("\n");
            bail!("'{agent}' matches more than one agent on this hub — pass the id instead:\n{list}")
        }
    }
}

async fn cmd_send(a: SendArgs) -> Result<()> {
    let target = target_from(a.room, a.to, a.service)?;
    let text = a.text.join(" ");
    let mut ag = connect().await?;
    let target = resolve_target(&mut ag, target).await?;
    let (_id, seq, room) = ag.send_text(target, &text).await?;
    println!("✓ sent to '{room}' (seq {seq})");
    Ok(())
}

async fn cmd_handoff(a: HandoffArgs) -> Result<()> {
    let target = target_from(a.room, a.to, a.service)?;
    let handoff = HandoffRef {
        next: a.next,
        summary: a.summary,
        to: a.for_who.clone(),
        bundle: a.bundle,
    };
    let mut ag = connect().await?;
    let target = resolve_target(&mut ag, target).await?;
    // Mention the addressee so the hub's push layer wakes them as well as the typed addressing.
    let mentions = a.for_who.map(|w| vec![w]);
    let (_id, seq, room) = ag.send(target, vec![handoff.to_part()], mentions, None).await?;
    let whom = handoff.to.as_deref().unwrap_or("anyone");
    println!("✓ handed off to {whom} in '{room}' (seq {seq})");
    println!("  next: {}", handoff.next);
    Ok(())
}

async fn cmd_push(a: PushArgs) -> Result<()> {
    let target = target_from(a.room, a.to, a.service)?;
    // Build the git bundle locally (in the current repo).
    let (bytes, tip, summary) =
        build_git_bundle(None, &a.gitref, a.base.as_deref(), a.summary.clone())?;
    let meta = BundleMeta {
        vcs: "git".into(),
        tip: Some(tip.clone()),
        base: a.base.clone(),
        summary: (!summary.is_empty()).then(|| summary.clone()),
        media_type: Some("application/x-git-bundle".into()),
    };
    let mut ag = connect().await?;
    let target = resolve_target(&mut ag, target).await?;
    let r = ag.push(target, &bytes, meta, a.note).await?;
    println!("✓ pushed git bundle to '{}' (seq {}, {} bytes)", r.room, r.seq, bytes.len());
    println!("  tip:  {}  {summary}", short(&tip));
    println!("  blob: {}", r.blob_id);
    println!("  peer: parler apply {}   (or just download: parler fetch {})", r.blob_id, r.blob_id);
    Ok(())
}

async fn cmd_fetch(a: FetchArgs) -> Result<()> {
    let mut ag = connect().await?;
    let bytes = ag.fetch_blob(&a.blob).await?;
    let out = a.out.unwrap_or_else(|| format!("{}.bundle", short(&a.blob)));
    std::fs::write(&out, &bytes)?;
    println!("✓ wrote {} bytes to {out}", bytes.len());
    Ok(())
}

async fn cmd_apply(a: ApplyArgs) -> Result<()> {
    if git_in(None, &["rev-parse", "--git-dir"]).is_err() {
        bail!("not inside a git repository — run `parler apply` from the repo you want to import into");
    }
    let mut ag = connect().await?;
    let bytes = ag.fetch_blob(&a.blob).await?;
    let tmp = std::env::temp_dir().join(format!("parler-apply-{}.bundle", std::process::id()));
    std::fs::write(&tmp, &bytes)?;
    let refname = format!("refs/parler/{}", short(&a.blob));
    let result = (|| -> Result<String> {
        let tmp_s = path_str(&tmp)?;
        if let Err(e) = git_in(None, &["bundle", "verify", tmp_s]) {
            bail!("bundle verify failed (you may be missing the base commit it is thin against): {e}");
        }
        // Import the objects (anchored by FETCH_HEAD) without touching the working tree…
        git_in(None, &["fetch", tmp_s])?;
        // …then pin the bundle's tip under a stable, namespaced ref.
        let heads = git_in(None, &["bundle", "list-heads", tmp_s])?;
        let tip_sha = heads.split_whitespace().next().unwrap_or_default().to_string();
        if !tip_sha.is_empty() {
            git_in(None, &["update-ref", &refname, &tip_sha])?;
        }
        Ok(heads)
    })();
    let _ = std::fs::remove_file(&tmp);
    let heads = result?;
    println!("✓ imported into {refname} (working tree untouched)");
    for line in heads.lines() {
        println!("    {}", line.trim());
    }
    println!("  inspect: git log {refname}    merge when ready: git merge {refname}");
    Ok(())
}

async fn cmd_recv(a: RecvArgs) -> Result<()> {
    let room = a.room
        .or_else(load_active_session)
        .ok_or_else(|| anyhow::anyhow!("specify --room, or open/join a session first"))?;
    let since = if a.all { Some(0) } else { a.since };
    let mut ag = connect().await?;

    // First, the backlog past the cursor (or `since`).
    let (msgs, cursor) = ag.pull(&room, since, a.limit).await?;
    if msgs.is_empty() {
        println!("(no new messages in '{}')", room);
    } else {
        for m in &msgs {
            println!("{}", render_message(m));
        }
        println!("— cursor at {cursor} —");
    }
    if !a.watch {
        return Ok(());
    }

    // Watch mode: ask the hub to push (sub-second), then block for new messages. Each wake re-pulls
    // the room, which both reads + advances/dedups the durable cursor AND, by sending a frame, keeps
    // the connection under the hub's idle timeout. If the hub can't push, degrade to a poll loop —
    // the per-iteration pull is then the poll.
    let pushing = ag.subscribe().await.unwrap_or(false);
    eprintln!(
        "👁  watching '{}' — {} (Ctrl-C to stop)",
        room,
        if pushing { "live push" } else { "polling every 2s" }
    );
    loop {
        if pushing {
            // Wake on a push for any of our rooms, or fall through every 25s to re-pull + heartbeat.
            let _ = ag.next_delivery(Duration::from_secs(25)).await?;
        } else {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let (new, cur) = ag.pull(&room, None, a.limit).await?;
        if !new.is_empty() {
            for m in &new {
                println!("{}", render_message(m));
            }
            println!("— cursor at {cur} —");
        }
    }
}

async fn cmd_remember(a: RememberArgs) -> Result<()> {
    let text = a.text.join(" ");
    let mut ag = connect().await?;
    ag.remember(&text, a.key, a.room, None, None).await?;
    println!("✓ remembered");
    Ok(())
}

async fn cmd_recall(a: RecallArgs) -> Result<()> {
    let query = a.query.join(" ");
    let mut ag = connect().await?;
    let hits = ag.recall(&query, a.room, a.limit, None).await?;
    if hits.is_empty() {
        println!("(nothing recalled for '{query}')");
        return Ok(());
    }
    for h in &hits {
        let scope = h.room.as_deref().map(|r| format!("#{r}")).unwrap_or_else(|| "private".into());
        let key = h.key.as_deref().map(|k| format!("[{k}] ")).unwrap_or_default();
        println!("• {key}{} ({scope})", h.text);
    }
    Ok(())
}

async fn cmd_rooms() -> Result<()> {
    let mut ag = connect().await?;
    let rooms = ag.rooms().await?;
    if rooms.is_empty() {
        println!("(no rooms yet — `parler invite` or `parler join`)");
        return Ok(());
    }
    for r in &rooms {
        let unread = if r.unread > 0 { format!("  ({} unread)", r.unread) } else { String::new() };
        println!("#{}  [{}]  {} member(s){unread}", r.name, r.kind.as_str(), r.members);
    }
    Ok(())
}

async fn cmd_roster(room: String) -> Result<()> {
    let mut ag = connect().await?;
    let entries = ag.roster(&room).await?;
    println!("members of '{room}':");
    for e in &entries {
        let role = e.role.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
        let act = e.activity.as_deref().map(|a| format!(" — {a}")).unwrap_or_default();
        println!("  {} {}{role}  [{}]{act}", e.name, e.id, e.status);
    }
    Ok(())
}

async fn cmd_presence(status: String, activity: Option<String>) -> Result<()> {
    let mut ag = connect().await?;
    ag.presence(&status, activity).await?;
    println!("✓ presence: {status}");
    Ok(())
}

fn cmd_whoami() -> Result<()> {
    let cfg = Config::load()?;
    println!("id:   {}", cfg.identity.id);
    println!(
        "name: {}{}",
        cfg.name,
        cfg.role.as_deref().map(|r| format!(" ({r})")).unwrap_or_default()
    );
    println!("hub:  {}", cfg.hub_url);
    Ok(())
}

/// One-line directory entry: `● name (role)  Uid…  [public ✓]  working  #tag …`.
fn render_entry(e: &DirectoryEntry) -> String {
    let role = e.card.role.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
    let vis = if e.verified {
        format!("{} ✓", e.visibility.as_str())
    } else {
        e.visibility.as_str().to_string()
    };
    let tags = e
        .card
        .tags
        .as_deref()
        .map(|t| t.iter().map(|x| format!("#{x}")).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    format!(
        "● {}{role}  {}  [{}]  {}  {}",
        e.card.name, e.card.id, vis, e.status, tags
    )
}

/// Multi-line directory card for `parler card <id>`.
fn render_entry_full(e: &DirectoryEntry) -> String {
    let mut out = String::new();
    out.push_str(&format!("name:    {}\n", e.card.name));
    out.push_str(&format!("id:      {}\n", e.card.id));
    if let Some(role) = &e.card.role {
        out.push_str(&format!("role:    {role}\n"));
    }
    out.push_str(&format!("hub:     {}\n", e.hub));
    out.push_str(&format!(
        "visible: {} ({})\n",
        e.visibility.as_str(),
        if e.verified { "signature verified ✓" } else { "unverified" }
    ));
    out.push_str(&format!("status:  {}\n", e.status));
    if let Some(d) = &e.card.description {
        out.push_str(&format!("about:   {d}\n"));
    }
    if let Some(tags) = &e.card.tags {
        out.push_str(&format!("tags:    {}\n", tags.join(", ")));
    }
    if let Some(skills) = &e.card.skills {
        let s = skills.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join(", ");
        out.push_str(&format!("skills:  {s}\n"));
    }
    out
}

/// Render the text of a message's parts (text joined; a bundle handoff and other extensions noted).
pub fn render_parts(parts: &[Part]) -> String {
    let mut out = Vec::new();
    for p in parts {
        if is_message_sig_part(p) {
            continue; // the detached author signature is verified (see render_message), not shown
        }
        if let Some(b) = BundleRef::from_part(p) {
            let sum = b.summary.unwrap_or_else(|| "(bundle)".into());
            let tip = b.tip.map(|t| format!(" @{}", short(&t))).unwrap_or_default();
            // The blob id is shown in full so the `parler apply` command copy-pastes and works.
            out.push(format!("📦 {sum}{tip} ({} bytes) — parler apply {}", b.size, b.blob));
            continue;
        }
        if let Some(h) = HandoffRef::from_part(p) {
            let whom = h.to.as_deref().unwrap_or("anyone");
            let mut line = format!("🤝 handoff → {whom}: {}", h.next);
            if let Some(s) = &h.summary {
                line.push_str(&format!("  (done: {s})"));
            }
            if let Some(blob) = &h.bundle {
                line.push_str(&format!("  — parler apply {blob}"));
            }
            out.push(line);
            continue;
        }
        match p {
            Part::Text(t) => out.push(t.clone()),
            Part::Data(d) => out.push(format!("[data] {d}")),
            Part::Extension { kind, .. } => out.push(format!("[{kind}]")),
        }
    }
    out.join(" ")
}

// ---- git helpers (code handoff) ----

/// Run `git` (optionally inside `repo` via `-C`), returning trimmed stdout or an error with stderr.
pub(crate) fn git_in(repo: Option<&str>, args: &[&str]) -> Result<String> {
    let mut cmd = std::process::Command::new("git");
    if let Some(r) = repo {
        cmd.arg("-C").arg(r);
    }
    cmd.args(args);
    let out = cmd
        .output()
        .map_err(|e| anyhow::anyhow!("running git: {e} (is git installed and on PATH?)"))?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub(crate) fn path_str(p: &Path) -> Result<&str> {
    p.to_str().ok_or_else(|| anyhow::anyhow!("non-UTF8 path: {}", p.display()))
}

/// First 12 chars of a content id (for display / ref names).
pub(crate) fn short(id: &str) -> &str {
    &id[..id.len().min(12)]
}

/// Build a git bundle for `gitref` (thin against `base` if given) in the repo at `repo` (cwd when
/// `None`). Returns `(bytes, tip_hash, summary)`.
pub(crate) fn build_git_bundle(
    repo: Option<&str>,
    gitref: &str,
    base: Option<&str>,
    summary_override: Option<String>,
) -> Result<(Vec<u8>, String, String)> {
    let tip = git_in(repo, &["rev-parse", gitref])?;
    let summary = match summary_override {
        Some(s) => s,
        None => git_in(repo, &["log", "-1", "--format=%s", gitref]).unwrap_or_default(),
    };
    let range = match base {
        Some(b) => format!("{b}..{gitref}"),
        None => gitref.to_string(),
    };
    let tmp = std::env::temp_dir().join(format!("parler-push-{}.bundle", std::process::id()));
    let made = git_in(repo, &["bundle", "create", path_str(&tmp)?, &range]);
    let bytes = match made {
        Ok(_) => std::fs::read(&tmp),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(anyhow::anyhow!("git bundle create failed: {e}"));
        }
    };
    let _ = std::fs::remove_file(&tmp);
    Ok((bytes?, tip, summary))
}

/// One line: `[seq] name (role): text`, with an authenticity marker on anything not cleanly signed.
///
/// Verified messages render clean (silent success); a legacy/unsigned peer is flagged `⚠` and a
/// message whose signature fails to verify — i.e. a hub forged or altered it — is flagged
/// `✗ UNVERIFIED` so a reader (or the agent itself) never mistakes tampered context for authentic.
pub fn render_message(m: &StoredMessage) -> String {
    let who = m
        .from
        .role
        .as_deref()
        .map(|r| format!("{} ({r})", m.from.name))
        .unwrap_or_else(|| m.from.name.clone());
    let body = render_parts(&m.parts);
    match verify_message(&m.from.id, &m.parts, m.reply_to.as_deref()) {
        SigStatus::Valid => format!("[{}] {}: {}", m.seq, who, body),
        SigStatus::Unsigned => format!("[{}] ⚠ {}: {}", m.seq, who, body),
        SigStatus::Invalid => format!("[{}] ✗ UNVERIFIED {}: {}", m.seq, who, body),
    }
}

pub fn save_active_session(room: &str) -> Result<()> {
    let home = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| parler_connector::home_dir());
    let path = home.join(".parler").join("active_session");
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, room)?;
    Ok(())
}

pub fn load_active_session() -> Option<String> {
    let home = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| parler_connector::home_dir());
    let path = home.join(".parler").join("active_session");
    std::fs::read_to_string(&path).ok().map(|s| s.trim().to_string())
}

pub fn clear_active_session() -> Result<()> {
    let home = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| parler_connector::home_dir());
    let path = home.join(".parler").join("active_session");
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    Ok(())
}

async fn check_session_key(cfg: &Config, key: &str, connected: bool) -> Result<String, String> {
    if !connected {
        return Err("hub offline".to_string());
    }
    match MeshAgent::connect(cfg).await {
        Ok(mut test_agent) => {
            match test_agent.redeem(key).await {
                Ok(outcome) => {
                    match outcome {
                        parler_connector::JoinOutcome::Joined { room, .. } => {
                            Ok(format!("✅ VALID (joined room '{room}')"))
                        }
                        parler_connector::JoinOutcome::Pending { room } => {
                            Ok(format!("✅ VALID (pending approval for room '{room}')"))
                        }
                    }
                }
                Err(e) => {
                    Err(format!("❌ STALE/CLOSED\n     PARLER_SESSION_KEY '{key}' is stale or closed.\n     Error: {e}"))
                }
            }
        }
        Err(_) => {
            Err("could not connect to test key".to_string())
        }
    }
}

async fn cmd_doctor() -> Result<()> {
    println!("🩺 Running Parler Protocol System Diagnostics...");
    let mut clean = true;

    // 1. Config Check
    print!("  • Checking local configuration... ");
    let mut loaded_cfg = None;
    if !Config::exists() {
        println!("❌ CONFIG NOT FOUND");
        println!("     👉 Fix: parler init");
        clean = false;
    } else {
        match Config::load() {
            Ok(cfg) => {
                // Show the *resolved* hub/name/role — the same `explicit env > saved config`
                // precedence the CLI and MCP server actually dial with, so doctor reports where the
                // agent really goes (and any env override is announced by the helper's stderr line).
                let cfg = mcp::apply_env_overrides(cfg);
                println!("✅ LOADED");
                println!("     Hub URL:      {}", cfg.hub_url);
                println!("     Agent Name:   {}", cfg.name);
                println!("     Agent Role:   {}", cfg.role.as_deref().unwrap_or("none"));
                println!("     Agent ID:     {}", cfg.identity.id);
                loaded_cfg = Some(cfg);
            }
            Err(e) => {
                println!("❌ PARSE ERROR ({e})");
                println!("     👉 Fix: parler init --force");
                clean = false;
            }
        }
    }

    // 2. Keypair Check (only if config loaded successfully)
    if let Some(ref cfg) = loaded_cfg {
        print!("  • Verifying Ed25519 identity keypair... ");
        let keypair_ok = (|| -> Result<()> {
            let kp = nkeys::KeyPair::from_seed(&cfg.identity.seed)?;
            if kp.public_key() != cfg.identity.id {
                bail!("identity mismatch: seed public key != config id");
            }
            let test_sig = kp.sign(b"parler_doctor_probe")?;
            if !parler_auth::verify(&cfg.identity.id, b"parler_doctor_probe", &data_encoding::BASE64.encode(&test_sig)) {
                bail!("keypair verification failed");
            }
            Ok(())
        })();
        match keypair_ok {
            Ok(_) => println!("✅ INTEGRITY OK"),
            Err(e) => {
                println!("❌ CORRUPTED ({e})");
                println!("     👉 Fix: parler init --force");
                clean = false;
            }
        }
    }

    // 3. Hub reachability & Join secret check
    let mut connected = false;
    if let Some(ref cfg) = loaded_cfg {
        print!("  • Testing connectivity to hub... ");
        match MeshAgent::connect(cfg).await {
            Ok(_) => {
                println!("✅ CONNECTED & AUTHENTICATED");
                connected = true;
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("requires a join secret") || err_str.contains("authentication failed:") {
                    println!("❌ JOIN SECRET INVALID");
                    println!("     Could not authenticate to {}: {}", cfg.hub_url, e);
                    println!("     👉 Fix: export PARLER_JOIN_SECRET=<secret>");
                } else {
                    println!("❌ FAILED");
                    println!("     Could not connect to {}: {}", cfg.hub_url, e);
                    // Name the exact start command for the hub this agent points at, so an
                    // agent-started-before-hub is never a mystery (issue #102).
                    println!("     👉 Fix: {} (or check your network/URL).", mcp::start_hub_hint(&cfg.hub_url));
                }
                clean = false;
            }
        }
    }

    // 4. Stale/closed PARLER_SESSION_KEY detection
    if let Some(ref cfg) = loaded_cfg {
        if let Some(key) = std::env::var("PARLER_SESSION_KEY").ok().filter(|s| !s.is_empty()) {
            print!("  • Checking PARLER_SESSION_KEY... ");
            match check_session_key(cfg, &key, connected).await {
                Ok(msg) => println!("{msg}"),
                Err(ref e) => {
                    if e.contains("❌ STALE/CLOSED") {
                        println!("{e}");
                        println!("     👉 Fix: unset PARLER_SESSION_KEY (stale key; remove it from your environment or configuration)");
                        clean = false;
                    } else if e == "hub offline" {
                        println!("⚠️ SKIPPED (hub offline)");
                    } else {
                        println!("⚠️ SKIPPED ({e})");
                    }
                }
            }
        }
    }

    // 5. MCP entry present per host
    print!("  • Checking MCP entries present per host... ");
    let hosts = connect::registry();
    let mut installed_hosts = Vec::new();
    for host in &hosts {
        if connect::is_installed(host) {
            installed_hosts.push(host);
        }
    }
    if installed_hosts.is_empty() {
        println!("✅ NO INSTALLED HOSTS DETECTED");
    } else {
        let mut mcp_clean = true;
        let mut details = Vec::new();
        for host in installed_hosts {
            if connect::is_configured(host) {
                details.push(format!("     ✅ {} is configured", host.name));
            } else {
                details.push(format!("     ❌ {} is NOT configured", host.name));
                details.push(format!("        👉 Fix: parler connect {}", host.id));
                mcp_clean = false;
                clean = false;
            }
        }
        if mcp_clean {
            println!("✅ OK");
        } else {
            println!("❌ MISSING CONFIGURATION");
        }
        for detail in details {
            println!("{detail}");
        }
    }

    // 6. Database Check
    print!("  • Checking sqlite-vec extension... ");
    match parler_hub::Store::open(None) {
        Ok(_) => println!("✅ AVAILABLE"),
        Err(e) => {
            println!("❌ UNAVAILABLE ({e})");
            println!("     👉 Fix: Check sqlite-vec dependency or library load paths.");
            clean = false;
        }
    }

    // 7. Git Check
    print!("  • Checking git workspace... ");
    match git_in(None, &["--version"]) {
        Ok(v) => {
            println!("✅ AVAILABLE ({})", v.trim());
            match git_in(None, &["rev-parse", "--show-toplevel"]) {
                Ok(path) => println!("     Git Repo Root: {}", path),
                Err(_) => println!("     ⚠️ Current directory is not inside a git repository."),
            }
        }
        Err(e) => {
            println!("❌ NOT FOUND ({e})");
            println!("     👉 Fix: Install git and ensure it is in your PATH.");
            clean = false;
        }
    }

    // 8. Recent MCP activity — the breadcrumb `parler mcp` leaves each launch, so a user can see
    // whether an editor-launched agent actually connected (its stderr is invisible in a GUI host).
    print!("  • Recent MCP activity... ");
    match mcp::recent_log(5) {
        Some(entries) if !entries.is_empty() => {
            println!("✅");
            for (ago, msg) in entries {
                println!("     {ago:>4} ago  {msg}");
            }
        }
        _ => println!("— none yet (start an MCP-wired agent, then re-run `parler doctor`)"),
    }

    println!();
    if clean {
        println!("✨ All diagnostics passed! Your Parler Protocol mesh agent is healthy.");
    } else {
        println!("⚠️ Diagnostics failed. Review the errors above to fix your installation.");
    }

    Ok(())
}

async fn cmd_hook(kind: String) -> Result<()> {
    // 1. Check if there is an active session
    let Some(room) = load_active_session() else {
        // Exit silently if no active session
        return Ok(());
    };

    // 2. Read stdin for JSON payload from Claude Code
    let mut stdin_buffer = String::new();
    let mut stdin = tokio::io::stdin();
    use tokio::io::AsyncReadExt;
    let _ = stdin.read_to_string(&mut stdin_buffer).await;
    
    let data: serde_json::Value = serde_json::from_str(&stdin_buffer).unwrap_or(serde_json::Value::Null);

    let mut ag = connect().await?;

    let parts = match kind.as_str() {
        "session-start" | "SessionStart" => {
            let cwd = data.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
            vec![Part::Text(format!("🚀 Session started by agent {} in directory {cwd}", ag.name))]
        }
        "session-end" | "SessionEnd" => {
            vec![Part::Text("👋 Session ended.".to_string())]
        }
        "user-prompt-submit" | "UserPromptSubmit" | "prompt-submit" | "PromptSubmit" => {
            let mut prompt = data.get("prompt")
                .or_else(|| data.get("text"))
                .or_else(|| data.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if prompt.is_empty() {
                return Ok(());
            }
            if prompt.len() > 1000 {
                prompt = format!("{}\n[...truncated]", &prompt[..1000]);
            }
            vec![Part::Text(format!("💬 Prompt: {prompt}"))]
        }
        "post-tool-use" | "PostToolUse" | "post-tool-use-failure" | "PostToolUseFailure" => {
            let tool_name = data.get("tool_name").or_else(|| data.get("toolName")).and_then(|v| v.as_str()).unwrap_or("unknown");
            let tool_input = data.get("tool_input").or_else(|| data.get("toolArgs")).cloned().unwrap_or(serde_json::json!({}));
            
            // Extract output and truncate if too long
            let tool_result = data.get("tool_response")
                .or_else(|| data.get("tool_output"))
                .or_else(|| data.get("toolResult"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
                
            let mut output_str = if tool_result.is_string() {
                tool_result.as_str().unwrap().to_string()
            } else if tool_result.is_object() {
                tool_result.get("text_result_for_llm")
                    .or_else(|| tool_result.get("textResultForLlm"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| tool_result.to_string())
            } else {
                tool_result.to_string()
            };
            
            if output_str.len() > 2000 {
                output_str = format!("{}\n[...truncated]", &output_str[..2000]);
            }
            
            let status = if kind.contains("failure") { "failure" } else { "success" };
            
            let mut fields = serde_json::Map::new();
            fields.insert("type".to_string(), serde_json::json!("tool"));
            fields.insert("tool_name".to_string(), serde_json::json!(tool_name));
            fields.insert("tool_input".to_string(), tool_input);
            fields.insert("tool_output".to_string(), serde_json::json!(output_str));
            fields.insert("status".to_string(), serde_json::json!(status));
            
            vec![Part::Extension {
                kind: "com.parler.observation".to_string(),
                fields,
            }]
        }
        _ => return Ok(()),
    };

    if !parts.is_empty() {
        let _ = ag.send(Target::Room { room }, parts, None, None).await;
    }

    Ok(())
}

async fn cmd_consolidate() -> Result<()> {
    let Some(room) = load_active_session() else {
        bail!("No active session found. Open or join a session first.");
    };

    println!("🧠 Consolidating active session backlog for room '{room}'...");
    let mut ag = connect().await?;

    // 1. Pull the backlog
    let (msgs, _) = ag.pull(&room, Some(0), None).await?;
    if msgs.is_empty() {
        println!("(No messages in backlog to consolidate)");
        return Ok(());
    }
    let backlog = msgs.iter().map(render_message).collect::<Vec<_>>().join("\n");

    // 2. Determine LLM provider & API key
    let gemini_key = std::env::var("GEMINI_API_KEY").ok();
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").ok();
    let openai_key = std::env::var("OPENAI_API_KEY").ok();

    let prompt = format!(
        "Analyze the following conversation backlog from a collaborative session. \
         Extract 1 to 5 key decisions, architectural choices, modified file paths, or lessons learned. \
         Format the output strictly as a JSON array of strings, where each string is a concise fact. \
         Only return the JSON array, no markdown wrappers (like ```json), no extra explanation.\n\n\
         Backlog:\n{}",
        backlog
    );

    let mut facts: Vec<String> = Vec::new();

    if let Some(key) = gemini_key {
        println!("  • Requesting consolidation from Gemini...");
        let payload = serde_json::json!({
            "contents": [{
                "parts": [{"text": prompt}]
            }]
        });
        let payload_str = serde_json::to_string(&payload)?;
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent?key={}",
            key
        );
        let output = run_curl_post(&url, &payload_str)?;
        if let Ok(res_val) = serde_json::from_str::<serde_json::Value>(&output) {
            if let Some(text) = res_val.pointer("/candidates/0/content/parts/0/text").and_then(|v| v.as_str()) {
                let cleaned = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
                if let Ok(parsed_facts) = serde_json::from_str::<Vec<String>>(cleaned) {
                    facts = parsed_facts;
                }
            }
        }
    } else if let Some(key) = anthropic_key {
        println!("  • Requesting consolidation from Anthropic...");
        let payload = serde_json::json!({
            "model": "claude-haiku-4-5-20251001",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": prompt}]
        });
        let payload_str = serde_json::to_string(&payload)?;
        let output = run_curl_post_headers(
            "https://api.anthropic.com/v1/messages",
            &payload_str,
            &[
                ("x-api-key", &key),
                ("anthropic-version", "2023-06-01"),
                ("content-type", "application/json"),
            ],
        )?;
        if let Ok(res_val) = serde_json::from_str::<serde_json::Value>(&output) {
            if let Some(text) = res_val.pointer("/content/0/text").and_then(|v| v.as_str()) {
                let cleaned = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
                if let Ok(parsed_facts) = serde_json::from_str::<Vec<String>>(cleaned) {
                    facts = parsed_facts;
                }
            }
        }
    } else if let Some(key) = openai_key {
        println!("  • Requesting consolidation from OpenAI...");
        let payload = serde_json::json!({
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": prompt}]
        });
        let payload_str = serde_json::to_string(&payload)?;
        let output = run_curl_post_headers(
            "https://api.openai.com/v1/chat/completions",
            &payload_str,
            &[
                ("authorization", &format!("Bearer {key}")),
                ("content-type", "application/json"),
            ],
        )?;
        if let Ok(res_val) = serde_json::from_str::<serde_json::Value>(&output) {
            if let Some(text) = res_val.pointer("/choices/0/message/content").and_then(|v| v.as_str()) {
                let cleaned = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
                if let Ok(parsed_facts) = serde_json::from_str::<Vec<String>>(cleaned) {
                    facts = parsed_facts;
                }
            }
        }
    } else {
        println!("⚠️ No LLM API key (GEMINI_API_KEY, ANTHROPIC_API_KEY, or OPENAI_API_KEY) found.");
        println!("  Consolidation is agent-driven inside the MCP server via the 'parler_consolidate_session' prompt.");
        return Ok(());
    }

    if facts.is_empty() {
        println!("❌ Failed to distill facts from LLM response.");
        return Ok(());
    }

    println!("✓ Distilled {} facts from session history:", facts.len());
    for f in &facts {
        println!("  • {f}");
        ag.remember(f, None, Some(room.clone()), None, None).await?;
    }
    println!("✓ Saved as room-scoped facts.");

    Ok(())
}

fn run_curl_post(url: &str, json_payload: &str) -> Result<String> {
    run_curl_post_headers(url, json_payload, &[("content-type", "application/json")])
}

fn run_curl_post_headers(url: &str, json_payload: &str, headers: &[(&str, &str)]) -> Result<String> {
    let mut cmd = std::process::Command::new("curl");
    cmd.arg("-s").arg("-X").arg("POST").arg(url);
    for (k, v) in headers {
        cmd.arg("-H").arg(format!("{k}: {v}"));
    }
    cmd.arg("-d").arg(json_payload);
    let out = cmd.output().map_err(|e| anyhow::anyhow!("running curl: {e}"))?;
    if !out.status.success() {
        bail!("curl request failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn team_teammate_oneliner_resolves_to_the_team_hub_with_secret() {
        // Issue #100(a): the teammate line `--team` prints is
        //   `PARLER_HUB=… PARLER_JOIN_SECRET=… parler connect`  (a bare `parler connect`, no flags).
        // With those env vars set and no hub flag, resolution must land on the team hub + secret —
        // not the silent public-hub default the issue warns about — and mark the run pinned.
        let (hub, secret, pinned) = resolve_connect_hub(HubInputs {
            hub_flag: None, shared: false, team: false, local: false, port: 7070,
            join_secret_flag: None,
            env_hub: Some("ws://10.0.0.5:7070".into()),
            env_secret: Some("TEAMSECRET123".into()),
        });
        assert!(matches!(&hub, connect::Hub::Explicit(u) if u == "ws://10.0.0.5:7070"), "PARLER_HUB → explicit hub");
        assert_eq!(secret.as_deref(), Some("TEAMSECRET123"), "PARLER_JOIN_SECRET carried through");
        assert!(pinned, "an env-provided hub pins the run so the teammate actually moves");
    }

    #[test]
    fn exported_hub_env_never_overrides_an_explicit_local_flag() {
        // The clap `env=` version regressed here: an *exported* PARLER_HUB made `--local` a conflict
        // error. Reading env only for a bare run fixes it — `--local` wins, env is ignored.
        let (hub, _s, pinned) = resolve_connect_hub(HubInputs {
            hub_flag: None, shared: false, team: false, local: true, port: 7071,
            join_secret_flag: None, env_hub: Some("ws://exported:7070".into()), env_secret: None,
        });
        assert!(matches!(hub, connect::Hub::Local { port: 7071 }), "--local wins over exported PARLER_HUB");
        assert!(pinned);
        // An explicit --hub also wins over env.
        let (hub2, _s, _p) = resolve_connect_hub(HubInputs {
            hub_flag: Some("ws://flag:9".into()), shared: false, team: false, local: false, port: 7070,
            join_secret_flag: None, env_hub: Some("ws://env:1".into()), env_secret: None,
        });
        assert!(matches!(hub2, connect::Hub::Explicit(u) if u == "ws://flag:9"), "--hub flag wins over env");
    }

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

    #[tokio::test]
    async fn test_stale_session_key_detection() {
        let hub_url = start_hub().await;
        
        // Setup a temporary configuration directory
        let temp_dir = tempfile::tempdir().unwrap();
        let old_home = std::env::var("PARLER_HOME").ok();
        std::env::set_var("PARLER_HOME", temp_dir.path());

        // Create config pointing to our local hub
        let cfg = Config::create(&hub_url, "doctor_test", None).unwrap();
        cfg.save().unwrap();

        // Testing check_session_key with a stale key
        let stale_key = "INVALIDKEY";
        let res = check_session_key(&cfg, stale_key, true).await;
        
        assert!(res.is_err());
        let err_msg = res.unwrap_err();
        assert!(err_msg.contains("❌ STALE/CLOSED"));
        assert!(err_msg.contains(stale_key));
        
        // Clean up environment variables
        if let Some(h) = old_home {
            std::env::set_var("PARLER_HOME", h);
        } else {
            std::env::remove_var("PARLER_HOME");
        }
    }
}
