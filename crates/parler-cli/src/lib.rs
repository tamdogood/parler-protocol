//! parler-cli — the `parler` command-line surface.
//!
//! Every networked subcommand is a thin wrapper over [`parler_connector::MeshAgent`]: load the
//! local identity, connect to the hub, do one op, print. `parler hub` runs the bus in-process and
//! `parler mcp` exposes the same ops as MCP tools (see [`mcp`]).

pub mod bring;
pub mod connect;
pub mod conversation;
pub mod mcp;
pub mod worker;
pub(crate) mod names;
pub mod work;

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};
use parler_connector::{
    verify_message, BundleMeta, Config, ConnectorRuntime, HostWakeInjector, Lifecycle, MeshAgent,
    RoomAttention, SigStatus, ToolSend, WakeRequest,
};
use parler_protocol::{
    is_message_sig_part, AgentSkill, Attention, BundleRef, DirectoryEntry, DiscoverScope, DispatchRef,
    FileRef, HandoffRef, Part, RoomKind, StoredMessage, Target, TaskRef, TaskStatus, Visibility,
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
    // Hidden from `--help` (#112): the story is "no init needed" — `parler connect` (or just adding
    // the MCP server) bootstraps the identity. Kept as a working command for advanced/scripted use.
    #[command(hide = true)]
    Init(InitArgs),
    /// Mint an invite code/link to hand to another agent (default: a 1:1 DM).
    Invite(InviteArgs),
    /// Redeem a pasted invite code/link. In a Codex/Claude agent terminal, a channel or DM join
    /// starts a safe handoff worker automatically; use --passive to only join.
    Join(JoinArgs),
    /// Join a legacy broadcast service room as a worker, then `recv` it for tasks.
    Serve {
        service: String,
    },
    /// Run an optional local supervisor that watches a room or role queue and spawns an explicit runner.
    Supervise(SuperviseArgs),
    /// Compatibility controls for the low-level room/session flow. Prefer `conversation`.
    #[command(subcommand, hide = true)]
    Session(SessionCmd),
    /// Start or join one live interactive conversation. No key creates; a key joins.
    Conversation(ConversationArgs),
    /// Get a one-line second opinion from another AI agent — no copy-paste (v1: codex).
    Bring(BringArgs),
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
    /// Send a message (one of --room / --to / --service / --role).
    Send(SendArgs),
    /// Hand off the turn: post a structured "you're up next" so a watching agent continues.
    Handoff(HandoffArgs),
    /// Post a task status update (accepted/working/awaiting/done/failed/cancelled) to a room/peer/queue.
    Task(TaskArgs),
    /// Run as an autonomous worker: wake on room handoffs/tasks, execute them, and post the result.
    Work(WorkArgs),
    /// Hand off code: bundle a git ref and push it to a room/peer/service.
    Push(PushArgs),
    /// Transfer a file to a room/peer/service (content-addressed; the peer runs `parler fetch`).
    SendFile(SendFileArgs),
    /// Download a pushed blob's bytes by its blob id (a code bundle or a file).
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
    /// Permanently delete a room you own.
    DeleteRoom {
        #[arg(long)]
        room: String,
    },
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
    /// Set local interruption policy: open/dnd/focus globally, or quiet/muted/inherit for one room.
    Attention(AttentionArgs),
    /// Print this agent's identity and hub.
    Whoami,
    /// Run the MCP server (stdio) exposing the parler_* tools to an MCP host.
    Mcp,
    /// Check configuration, database, connections, and dependencies.
    Doctor,
    /// Handle a Claude Code / editor lifecycle hook. `stop` is the wake hook: it blocks briefly for
    /// peers' messages and continues the turn so agents auto-poll (wired by `parler connect`).
    Hook {
        /// The hook type (normal lifecycle values plus internal visible-conversation adapters).
        kind: String,
    },
    /// Consolidate the active conversation backlog into key semantic facts.
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
    /// Run a public hub (public cards are world-readable; private cards remain token-gated).
    #[arg(long, env = "PARLER_HUB_PUBLIC")]
    public: bool,
    /// Require this shared secret on connect. Strongly recommended for a private hub exposed on a
    /// public URL — otherwise anyone who can reach it can join. Agents present it via
    /// `PARLER_JOIN_SECRET`.
    #[arg(long, env = "PARLER_HUB_JOIN_SECRET")]
    join_secret: Option<String>,
    /// Trust proxy-provided client IP headers. Enable only behind a proxy that overwrites them.
    #[arg(long, env = "PARLER_HUB_TRUST_PROXY_HEADERS")]
    trust_proxy_headers: bool,
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
    /// Don't install the Claude Code wake (`Stop`) hook. By default `connect` wires it so agents in a
    /// session auto-poll for each other's messages and continue on their own; pass this to skip it
    /// (e.g. you prefer to fetch with `parler recv` yourself).
    #[arg(long)]
    no_hooks: bool,
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
        /// Require owner approval before a key holder joins. By default, possession of the private
        /// key admits immediately.
        #[arg(long, conflicts_with = "no_approval")]
        approval: bool,
        /// Compatibility alias from when approval was the default. Immediate admission is now the
        /// default, so this flag is accepted but has no additional effect.
        #[arg(long, hide = true)]
        no_approval: bool,
        /// How long the key stays valid, in seconds (default 86400).
        #[arg(long)]
        ttl: Option<u64>,
        /// How many agents may join with the key (default 50).
        #[arg(long)]
        max_uses: Option<u32>,
    },
    /// Join a shared session with a key, print the context so far, then stay active. In a
    /// Codex/Claude agent terminal this starts a safe handoff worker; otherwise it remains a live
    /// display listener until Ctrl-C.
    Join {
        /// The session key (or full link) you were given.
        key: String,
        /// Join, print the context, and exit immediately instead of holding the connection open.
        /// Use this for scripts/CI; a live agent should stay connected (the default) or use the MCP
        /// server so the host actually sees it in the room.
        #[arg(long, conflicts_with_all = ["active", "runner"])]
        once: bool,
        /// Start a bounded local worker after joining, even outside a detected Codex/Claude agent
        /// host. It executes only valid signed handoffs.
        #[arg(long, conflicts_with = "passive")]
        active: bool,
        /// Headless worker used by --active. Supplying it also activates the worker.
        #[arg(long, value_parser = ["codex", "claude"], conflicts_with = "passive")]
        runner: Option<String>,
        /// Do not auto-start a worker in a detected agent terminal; retain the display-only
        /// listener. Useful when another activation consumer owns this room.
        #[arg(long, conflicts_with_all = ["active", "runner"])]
        passive: bool,
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
struct JoinArgs {
    /// The code (or full link) the other agent gave you.
    code: String,
    /// Start a bounded local worker after joining, even outside a detected Codex/Claude agent host.
    /// It executes only valid signed handoffs.
    #[arg(long, conflicts_with = "passive")]
    active: bool,
    /// Headless worker used by --active. Supplying it also activates the worker.
    #[arg(long, value_parser = ["codex", "claude"], conflicts_with = "passive")]
    runner: Option<String>,
    /// Do not auto-start a worker in a detected agent terminal. Useful for scripts or when another
    /// activation consumer owns this room.
    #[arg(long, conflicts_with_all = ["active", "runner"])]
    passive: bool,
}

#[derive(Args)]
struct BringArgs {
    /// Which agent to ask for a second opinion. v1: codex.
    agent: String,
    /// The context to review — a recap of the code/decision and what you want a second opinion on.
    #[arg(long, conflicts_with = "context_file")]
    context: Option<String>,
    /// Read the context from a file instead of --context; use `-` for stdin. The `parler_bring`
    /// MCP tool uses `-` so a large recap never has to fit on the command line.
    #[arg(long)]
    context_file: Option<String>,
    /// Override the default "senior engineer, second opinion" instruction handed to the agent.
    #[arg(long)]
    instruction: Option<String>,
    /// Also post the review into this session room (so it lands in the conversation via recv).
    #[arg(long)]
    room: Option<String>,
    /// Don't print the review to stdout — only post it to --room. Used when spawned by the MCP tool.
    #[arg(long)]
    quiet: bool,
    /// Wall-clock timeout in seconds (default 300).
    #[arg(long)]
    timeout_secs: Option<u64>,
}

#[derive(Args)]
struct ConversationArgs {
    /// Portable conversation key to join. Omit it to create a new conversation.
    key: Option<String>,
    /// Visible agent host to open for this conversation.
    #[arg(long, value_enum, default_value_t = conversation::Host::Codex)]
    host: conversation::Host,
    /// Human-readable topic for a new conversation.
    #[arg(long)]
    topic: Option<String>,
    /// Resume an existing host conversation (`last` or a host session/thread id).
    #[arg(long, value_name = "LAST_OR_ID")]
    resume: Option<String>,
    /// Require the owner to approve joiners. By default, possession of the private key admits them.
    #[arg(long)]
    approval: bool,
    /// How long a new conversation key stays valid, in seconds (default 86400).
    #[arg(long)]
    ttl: Option<u64>,
    /// How many agents may join a new conversation with the key (default 50).
    #[arg(long)]
    max_uses: Option<u32>,
}

#[derive(Args)]
struct InitArgs {
    /// Hub address/URL (host:port, ws://, or parler://).
    #[arg(long, default_value = "parler://127.0.0.1:7070")]
    hub: String,
    /// Display name (defaults to a fun `adjective-animal-<tag>` handle).
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
    /// Send role-addressed anycast work. Exactly one available worker serving this role can claim it.
    #[arg(long)]
    role: Option<String>,
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
struct TaskArgs {
    /// Where the work stands: accepted | working | awaiting | done | failed | cancelled.
    status: String,
    /// Post the update to a channel room (one-to-many).
    #[arg(long)]
    room: Option<String>,
    /// Post the update as a DM to the requester's agent id (one-to-one).
    #[arg(long)]
    to: Option<String>,
    /// Post the update to a service queue (many-to-one).
    #[arg(long)]
    service: Option<String>,
    /// Correlate this update to one unit of work (the request's message id, or your own task id).
    #[arg(long)]
    task: Option<String>,
    /// A one-liner: what's happening / why it failed / the question when `awaiting`.
    #[arg(long)]
    note: Option<String>,
    /// A result blob id handed back with `done` (from a prior `parler push`/`send-file`).
    #[arg(long)]
    result: Option<String>,
    /// Estimated model tokens this work consumed (terminal receipts) — feeds directory telemetry.
    #[arg(long)]
    tokens: Option<u64>,
    /// Wall-clock milliseconds this work took (terminal receipts).
    #[arg(long = "elapsed-ms")]
    elapsed_ms: Option<u64>,
}

#[derive(Args)]
struct WorkArgs {
    /// Watch this channel/session room (default: the active session).
    #[arg(long, conflicts_with = "service")]
    room: Option<String>,
    /// Join and watch this service queue; results are sent back to each requester's DM.
    #[arg(long, conflicts_with = "room")]
    service: Option<String>,
    /// Headless agent used for each turn.
    #[arg(long, default_value = "codex", value_parser = ["codex", "claude"])]
    runner: String,
    /// Treat every signed peer text message as work. Intended for a trusted two-agent room; without
    /// this flag only valid signed handoffs execute.
    #[arg(long)]
    all_messages: bool,
    /// Only execute messages signed by this agent id (repeatable). Room mode otherwise trusts any
    /// approved member; service mode requires this or --allow-any.
    #[arg(long = "allow-from", value_name = "AGENT_ID")]
    allow_from: Vec<String>,
    /// Let any signed service requester run the worker. Unsafe on a public/untrusted hub; prefer
    /// --allow-from whenever possible.
    #[arg(long, conflicts_with = "allow_from")]
    allow_any: bool,
    /// Maximum model turns started per rolling hour. 0 disables the cap.
    #[arg(long, default_value_t = 20)]
    max_per_hour: u32,
    /// Wall-clock limit for each model turn, in seconds.
    #[arg(long, default_value_t = 900, value_parser = clap::value_parser!(u64).range(1..=7200))]
    timeout_secs: u64,
    /// Exit after one actionable message (useful for schedulers and tests).
    #[arg(long)]
    once: bool,
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
struct SendFileArgs {
    /// Send to a channel room (one-to-many).
    #[arg(long)]
    room: Option<String>,
    /// Send a DM to a peer agent id (one-to-one).
    #[arg(long)]
    to: Option<String>,
    /// Send to a service queue (many-to-one).
    #[arg(long)]
    service: Option<String>,
    /// An optional note posted alongside the file.
    #[arg(long)]
    note: Option<String>,
    /// The path of the file to send.
    path: String,
}

#[derive(Args)]
struct FetchArgs {
    /// The blob id (from a `com.parler.bundle` or `com.parler.file` message).
    blob: String,
    /// Output file (default: <blob-prefix>.bin).
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
struct SuperviseArgs {
    /// Serve this role and atomically claim role-addressed work from its service queue.
    #[arg(long)]
    role: Option<String>,
    /// Watch this already-joined room instead of a role queue (useful for a self-coordinating body agent).
    #[arg(long)]
    room: Option<String>,
    /// Explicit local command to run for each accepted message. The rendered task is passed on stdin.
    #[arg(long)]
    runner: String,
    /// Lease length for a role task; the supervisor renews it while the child runs (15–3600 seconds).
    #[arg(long, default_value_t = 300)]
    lease_secs: u64,
    /// Maximum wall-clock seconds for one local runner before it is stopped and reported failed.
    #[arg(long, default_value_t = 1800)]
    timeout_secs: u64,
    /// Exit after completing one task instead of supervising continuously.
    #[arg(long)]
    once: bool,
    /// Maximum bytes retained from each child stdout/stderr stream (the child is still fully drained).
    #[arg(long, default_value_t = 65_536)]
    max_output_bytes: usize,
}

#[derive(Args)]
struct AttentionArgs {
    /// Global: open | dnd | focus. With --room: quiet | muted | inherit. Omit to show the policy.
    mode: Option<String>,
    /// Apply the room-local override to this room instead of changing the global mode.
    #[arg(long)]
    room: Option<String>,
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
    // Agent-host commands and the MCP server must use workspace-scoped identities. Without this,
    // `parler mcp` was scoped while a Codex/Claude terminal-driven `parler join` read the old flat
    // ~/.parler/config.json; every terminal then appeared as that one saved agent. An ordinary human
    // CLI remains flat for backward compatibility with `parler init`.
    if command_uses_workspace_identity(&cli.cmd, agent_shell_detected()) {
        mcp::scope_identity_to_workspace();
    }
    match cli.cmd {
        Cmd::Hub(a) => cmd_hub(a).await,
        Cmd::Connect(a) => cmd_connect(a).await,
        Cmd::Init(a) => cmd_init(a),
        Cmd::Invite(a) => cmd_invite(a).await,
        Cmd::Join(a) => cmd_join(a).await,
        Cmd::Serve { service } => cmd_serve(service).await,
        Cmd::Supervise(a) => cmd_supervise(a).await,
        Cmd::Session(c) => cmd_session(c).await,
        Cmd::Conversation(a) => conversation::run(conversation::Options {
            key: a.key,
            host: a.host,
            topic: a.topic,
            resume: a.resume,
            approval: a.approval,
            ttl: a.ttl,
            max_uses: a.max_uses,
        })
        .await,
        Cmd::Bring(a) => cmd_bring(a).await,
        Cmd::Register(a) => cmd_register(a).await,
        Cmd::Discover(a) => cmd_discover(a).await,
        Cmd::Card { id } => cmd_card(id).await,
        Cmd::Token(a) => cmd_token(a).await,
        Cmd::Send(a) => cmd_send(a).await,
        Cmd::Handoff(a) => cmd_handoff(a).await,
        Cmd::Task(a) => cmd_task(a).await,
        Cmd::Work(a) => cmd_work(a).await,
        Cmd::Push(a) => cmd_push(a).await,
        Cmd::SendFile(a) => cmd_send_file(a).await,
        Cmd::Fetch(a) => cmd_fetch(a).await,
        Cmd::Apply(a) => cmd_apply(a).await,
        Cmd::Recv(a) => cmd_recv(a).await,
        Cmd::Remember(a) => cmd_remember(a).await,
        Cmd::Recall(a) => cmd_recall(a).await,
        Cmd::Rooms => cmd_rooms().await,
        Cmd::DeleteRoom { room } => cmd_delete_room(room).await,
        Cmd::Roster { room } => cmd_roster(room).await,
        Cmd::Presence { status, activity } => cmd_presence(status, activity).await,
        Cmd::Attention(a) => cmd_attention(a).await,
        Cmd::Whoami => cmd_whoami(),
        Cmd::Mcp => mcp::serve_stdio().await,
        Cmd::Doctor => cmd_doctor().await,
        Cmd::Hook { kind } => cmd_hook(kind).await,
        Cmd::Consolidate => cmd_consolidate().await,
    }
}

fn command_uses_workspace_identity(cmd: &Cmd, agent_shell: bool) -> bool {
    // `conversation` scopes itself with the terminal instance as well as the workspace, then passes
    // the unscoped base into its child host UI. Doing that here would scope twice in the child.
    matches!(cmd, Cmd::Mcp | Cmd::Supervise(_) | Cmd::Work(_) | Cmd::Hook { .. })
        || (agent_shell
            && !matches!(
                cmd,
                Cmd::Hub(_) | Cmd::Connect(_) | Cmd::Conversation(_) | Cmd::Init(_) | Cmd::Doctor
            ))
}

/// Whether this command was launched inside an AI-agent terminal rather than an ordinary human
/// shell. These are host-provided context markers, not identity material; their values are never
/// persisted or sent. The decision stays separate from command routing so tests don't mutate env.
fn agent_shell_detected() -> bool {
    [
        "CODEX_THREAD_ID",
        "CODEX_WORKING_DIR",
        "CONDUCTOR_WORKSPACE_PATH",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_PROJECT_DIR",
    ]
    .iter()
    .any(|key| env_is_set(key))
}

fn env_is_set(key: &str) -> bool {
    std::env::var(key).is_ok_and(|value| !value.is_empty())
}

/// The join command stays one-shot in an ordinary shell, but an agent-hosted Codex/Claude join can
/// own the missing activation boundary immediately. The worker defaults to valid signed handoffs, so
/// joining a room never turns arbitrary peer text into workspace-writing model input.
#[derive(Debug, Clone, PartialEq, Eq)]
enum JoinActivation {
    Passive,
    Worker(String),
}

impl JoinActivation {
    fn runner(&self) -> Option<&str> {
        match self {
            JoinActivation::Passive => None,
            JoinActivation::Worker(runner) => Some(runner),
        }
    }
}

struct JoinActivationInput<'a> {
    agent_shell: bool,
    codex: bool,
    claude: bool,
    active: bool,
    passive: bool,
    runner: Option<&'a str>,
}

fn join_activation(input: JoinActivationInput<'_>) -> JoinActivation {
    if input.passive {
        return JoinActivation::Passive;
    }
    if let Some(runner) = input.runner {
        return JoinActivation::Worker(runner.to_string());
    }
    let inferred = match (input.codex, input.claude) {
        (true, false) => Some("codex"),
        (false, true) => Some("claude"),
        _ => None,
    };
    if input.active {
        // `parler work` already makes Codex the documented default. Preserve that useful explicit
        // fallback when a person asks for activation from an otherwise unidentifiable shell.
        return JoinActivation::Worker(inferred.unwrap_or("codex").to_string());
    }
    if input.agent_shell {
        return inferred
            .map(|runner| JoinActivation::Worker(runner.to_string()))
            .unwrap_or(JoinActivation::Passive);
    }
    JoinActivation::Passive
}

fn join_activation_from_environment(active: bool, passive: bool, runner: Option<&str>) -> JoinActivation {
    join_activation(JoinActivationInput {
        agent_shell: agent_shell_detected(),
        codex: env_is_set("CODEX_THREAD_ID") || env_is_set("CODEX_WORKING_DIR"),
        claude: env_is_set("CLAUDE_CODE_SESSION_ID") || env_is_set("CLAUDE_PROJECT_DIR"),
        active,
        passive,
        runner,
    })
}

fn automatic_join_work_options() -> worker::WorkOptions {
    worker::WorkOptions {
        source: worker::WorkSource::Room,
        all_messages: false,
        allow_from: Default::default(),
        max_per_hour: 20,
        timeout: Duration::from_secs(900),
        once: false,
    }
}

fn join_supports_safe_worker(kind: RoomKind) -> bool {
    matches!(kind, RoomKind::Channel | RoomKind::Dm)
}

/// Convert an agent-hosted channel/DM join into the bounded worker users otherwise had to start in
/// a second command. Service workers retain their explicit configuration because their trust model
/// requires a dispatcher allowlist or `--allow-any`.
async fn start_join_worker(
    agent: &mut MeshAgent,
    room: &str,
    kind: RoomKind,
    activation: &JoinActivation,
) -> Result<bool> {
    let Some(runner_name) = activation.runner() else {
        return Ok(false);
    };
    if !join_supports_safe_worker(kind) {
        eprintln!(
            "joined service room '{room}'; use `parler work --service … --allow-from …` to choose its dispatcher policy"
        );
        return Ok(false);
    }
    let runner = worker::ProcessRunner::parse(runner_name)?;
    println!(
        "🤖 active {runner_name} listener started for '{room}' — future valid signed handoffs run automatically"
    );
    worker::run(agent, room, &automatic_join_work_options(), &runner).await?;
    Ok(true)
}

async fn connect() -> Result<MeshAgent> {
    connect_with_hub(None).await
}

/// Connect, optionally overriding the configured hub for *this* command only. An existing identity
/// and config stay untouched; a first-run identity is initialized to the carried hub so its follow-up
/// room commands work. Used by a **portable session join** (`<key>@<hub>`): a joiner whose default hub
/// differs from where the session lives dials the session's hub directly — the lightest form of
/// cross-hub handoff, with no hub-to-hub federation.
async fn connect_with_hub(hub_override: Option<&str>) -> Result<MeshAgent> {
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
            // The legacy `~/.parler-codex` migration path must never override an explicit or
            // workspace/terminal-scoped PARLER_HOME. Doing so would collapse two visible
            // conversation agents back onto one identity.
            if std::env::var_os("PARLER_HOME").is_none() && fallback_path.exists() {
                std::env::set_var("PARLER_HOME", fallback_path.parent().unwrap());
                mcp::apply_env_overrides(Config::load().map_err(|_| e)?)
            } else if !Config::exists() {
                // Zero-setup, same as `parler mcp`: mint an identity on first use (already env-aware)
                // instead of telling the user to go run `parler init` first.
                // For a portable first join, make the carried hub the bootstrap default up front so
                // both the saved config and the initialization message name the hub actually dialed.
                if let Some(h) = hub_override {
                    std::env::set_var("PARLER_HUB", h);
                }
                let cfg = mcp::load_or_bootstrap_config()?;
                eprintln!(
                    "✱ first run — created your identity at {} (hub: {})",
                    parler_connector::home_dir().display(),
                    cfg.hub_url
                );
                // A first-run identity that can't even reach its hub is a confusing half-success
                // (the "initialized new agent …" line followed by a connect error, as when a
                // sandbox blocks DNS). Roll the just-minted identity back so the next attempt —
                // after `parler doctor` fixes the network — starts clean instead of adopting a
                // stranded identity. Only this freshly-bootstrapped branch is rolled back.
                return MeshAgent::connect(&cfg).await.map_err(|err| {
                    let _ = Config::remove();
                    anyhow::anyhow!("{err} (run `parler doctor` to troubleshoot)")
                });
            } else {
                return Err(e);
            }
        }
    };
    let mut cfg = cfg;
    if let Some(h) = hub_override {
        cfg.hub_url = h.to_string();
    }
    MeshAgent::connect(&cfg).await.map_err(|e| {
        anyhow::anyhow!("{e} (run `parler doctor` to troubleshoot)")
    })
}

/// Split a portable session descriptor into its code and optional host hub. Accept both the compact
/// `<code>@<hub>` form and the invite URL the hub prints (`<scheme>://<hub>/join/<code>`). A full
/// link must carry its hub here, before connection, rather than relying on the server to strip the
/// final path segment after the client has already dialed the wrong hub.
fn split_portable_key(key: &str) -> (String, Option<String>) {
    let key = key.trim();
    if let Some((code, hub)) = key.split_once('@') {
        if !code.is_empty() && !hub.is_empty() && !code.contains([':', '/']) {
            return (code.to_string(), Some(hub.to_string()));
        }
    }
    if let Some((hub, tail)) = key.split_once("/join/") {
        let scheme = hub.split_once("://").map(|(scheme, _)| scheme);
        if matches!(scheme, Some("parler" | "ws" | "wss" | "http" | "https")) {
            let code = tail.split(['/', '?', '#']).next().unwrap_or_default();
            if !code.is_empty() {
                return (code.to_string(), Some(hub.to_string()));
            }
        }
    }
    (key.to_string(), None)
}

/// Build the website's read-only session viewer deep link for an owner-minted WATCH code.
pub(crate) fn session_view_link(watch_code: &str) -> String {
    format!("https://www.parlerprotocol.com/hub#sessions&k={watch_code}")
}

/// The hub returns a bare `invalid or unknown invite code` for any code it doesn't hold — and the
/// most common cause is a **hub mismatch**: the code was minted on a different hub than the one this
/// agent dials, so the *code* is fine but the *hub* is wrong (exactly what makes a cross-agent
/// hand-off fail). Rewrite that dead-end into a signpost — name the hub we tried and show the
/// portable form that carries the minting hub. Any other error passes through untouched.
fn explain_unknown_code(err: anyhow::Error, hub_url: &str, code: &str, join_cmd: &str) -> anyhow::Error {
    if err.to_string().contains("invalid or unknown invite code") {
        return anyhow::anyhow!(
            "invalid or unknown invite code on hub {hub_url}.\n  \
             If it was minted on a different hub, redeem the portable form that carries it:\n    \
             {join_cmd} {code}@<that-hub>\n  \
             (whoever shared the code sees its hub in `parler whoami`.)"
        );
    }
    err
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
    state.trust_proxy_headers = a.trust_proxy_headers;
    if mode == parler_hub::HubMode::Private
        && state.join_secret.is_none()
        && !hub_bind_is_loopback(&a.addr)
    {
        bail!(
            "refusing to expose a private hub on '{}' without a join secret; pass --join-secret or bind to loopback",
            a.addr
        );
    }
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

fn hub_bind_is_loopback(addr: &str) -> bool {
    addr.parse::<std::net::SocketAddr>()
        .map(|addr| addr.ip().is_loopback())
        .unwrap_or_else(|_| {
            addr.rsplit_once(':')
                .map(|(host, _)| host.eq_ignore_ascii_case("localhost"))
                .unwrap_or(false)
        })
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
        install_hooks: !a.no_hooks,
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

/// A throwaway in-memory identity for reachability/`--verify` probes. Minted fresh and **never saved
/// to disk**, so a probe can't create or re-pin `~/.parler/config.json` to whatever hub it happened
/// to test (#112) — the old code called `load_or_bootstrap_config()` inside a read-style check. The
/// hub is a relay, not a root of trust, so a fresh nkey authenticates fine for a read-only dial.
fn ephemeral_probe_config(hub: &str) -> Result<Config> {
    Config::create(hub.to_string(), "parler-probe", None)
}

/// Save/restore `PARLER_JOIN_SECRET` around a probe that must set it per-hub, so the probe doesn't
/// leak a secret into the process env that later adopt-bare/bootstrap logic would read (#112).
struct JoinSecretGuard(Option<String>);
impl JoinSecretGuard {
    fn capture() -> Self {
        Self(std::env::var("PARLER_JOIN_SECRET").ok())
    }
    /// Set the env to `secret` for the duration of one hub's dial (restored wholesale on drop).
    fn set(&self, secret: Option<&String>) {
        match secret {
            Some(s) => std::env::set_var("PARLER_JOIN_SECRET", s),
            None => std::env::remove_var("PARLER_JOIN_SECRET"),
        }
    }
}
impl Drop for JoinSecretGuard {
    fn drop(&mut self) {
        match &self.0 {
            Some(s) => std::env::set_var("PARLER_JOIN_SECRET", s),
            None => std::env::remove_var("PARLER_JOIN_SECRET"),
        }
    }
}

/// The tail of a bare `parler connect`: dial each hub the agents were wired to **once** (short
/// timeout) to prove reachability, then return. Not a substitute for `--verify` (which waits for the
/// agents themselves to come online) — just a fast "is this hub actually up?" so failures aren't silent.
async fn probe_hubs(wired: &[connect::WiredAgent]) {
    use std::collections::BTreeSet;
    // Restore PARLER_JOIN_SECRET when we're done — the probe sets it per-hub but must leave the
    // process env exactly as it found it.
    let secret_guard = JoinSecretGuard::capture();
    let hubs: BTreeSet<(String, Option<String>)> =
        wired.iter().map(|w| (w.hub.clone(), w.secret.clone())).collect();
    for (hub, secret) in hubs {
        // A gated hub needs the same join secret the agents were handed.
        secret_guard.set(secret.as_ref());
        let cfg = match ephemeral_probe_config(&hub) {
            Ok(c) => c,
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
    // Each pending agent carries its display name (for output) and its `PARLER_HOME` — the id is
    // read from there, not matched by name, so a same-named stranger on the shared hub can't be
    // mistaken for a freshly-wired agent (#103).
    let mut by_hub: BTreeMap<(String, Option<String>), Vec<connect::WiredAgent>> = BTreeMap::new();
    for w in wired {
        by_hub.entry((w.hub.clone(), w.secret.clone())).or_default().push(w);
    }
    println!("Waiting for your agents to dial in — restart them now (Ctrl-C to stop waiting).");
    // Restore PARLER_JOIN_SECRET when done; watch the hub with a throwaway identity so verifying
    // never creates or re-pins the user's `~/.parler/config.json` (#112).
    let secret_guard = JoinSecretGuard::capture();
    let started = std::time::Instant::now();
    for ((hub, secret), mut pending) in by_hub {
        secret_guard.set(secret.as_ref());
        let cfg = match ephemeral_probe_config(&hub) {
            Ok(c) => c,
            Err(e) => {
                println!("  ✗ couldn't prepare a local identity to watch {hub}: {e}");
                continue;
            }
        };
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
        while !pending.is_empty() && started.elapsed() < timeout {
            let seen = ag
                .discover(DiscoverScope::Hub, None, None, None, None, Some(500))
                .await
                .unwrap_or_default();
            let online_ids: Vec<&str> = seen.iter().map(|e| e.card.id.as_str()).collect();
            pending.retain(|w| {
                // Match on the wired identity's id (read from its PARLER_HOME once `parler mcp` has
                // launched and minted/saved it), never on name — an id is the agent's public key, so
                // this can't confirm a same-named stranger that happens to be online (#103). If the
                // agent hasn't booted yet its config.json is absent → no ids → still pending. A wired
                // host may have several per-workspace identities; it's "in" once any one is online.
                let online = wired_agent_ids(&w.home).iter().any(|id| online_ids.contains(&id.as_str()));
                if online {
                    println!("  ✓ {} dialed in ({}s)", w.name, started.elapsed().as_secs());
                }
                !online
            });
            if pending.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        for w in &pending {
            println!("  ⏳ {} hasn't dialed in after {}s — restart it, run `parler doctor` to troubleshoot, or check later: parler connect --list", w.name, timeout.as_secs());
        }
    }
    Ok(())
}

/// Read every wired identity id under `home` — the flat `<home>/config.json` **plus** any per-workspace
/// `<home>/ws/<hash>/config.json` (`parler mcp` scopes its identity per workspace, so one wired host can
/// have several). Returns them all so `--verify` reports the host "dialed in" once *any* of its
/// workspace agents is online. Empty before the agent has launched (no config yet), or when a file is
/// unreadable / has no `id`. Pure path→ids (no process env) so it's testable and the match stays
/// deterministic.
fn wired_agent_ids(home: &std::path::Path) -> Vec<String> {
    fn id_at(path: &std::path::Path) -> Option<String> {
        let text = std::fs::read_to_string(path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        v.get("id").and_then(|id| id.as_str()).map(str::to_string)
    }
    let mut ids: Vec<String> = id_at(&home.join("config.json")).into_iter().collect();
    if let Ok(entries) = std::fs::read_dir(home.join("ws")) {
        for e in entries.flatten() {
            ids.extend(id_at(&e.path().join("config.json")));
        }
    }
    ids
}

fn cmd_init(a: InitArgs) -> Result<()> {
    if Config::exists() && !a.force {
        bail!("already initialized — pass --force to overwrite the existing identity");
    }
    // No `--name`? Mint the identity first, then give it a fun `adjective-animal-<tag>` handle
    // seeded on its unique id — the same default `parler mcp` bootstraps, so both entry points agree.
    let mut cfg = Config::create(a.hub, a.name.clone().unwrap_or_else(|| "agent".into()), a.role)?;
    if a.name.is_none() {
        cfg.name = crate::names::fun_name(&cfg.identity.id);
    }
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
    // Lead with the *portable* code — the trailing `@<hub>` carries this hub, so it redeems even when
    // the other agent's default hub differs (the usual cause of "invalid or unknown invite code" on a
    // cross-hub hand-off). The bare code still works for an agent already on this hub.
    println!("Hand it to another agent and have it run:  parler join {}@{}", inv.code, ag.hub_url);
    println!("  (already on this hub? the bare code works too:  parler join {})", inv.code);
    if join_supports_safe_worker(inv.kind) {
        println!("  Codex/Claude agent joins start a safe handoff listener automatically; add --passive to only join.");
    } else {
        println!("  Service work stays explicit: use `parler work --service … --allow-from …` to choose its dispatcher policy.");
    }
    Ok(())
}

async fn cmd_join(a: JoinArgs) -> Result<()> {
    // A portable code `<code>@<hub>` carries the hub that minted it, so a joiner whose default hub
    // differs still lands in the right room (same trick as `session join`). Dial the embedded hub for
    // this one call; a bare code redeems against the configured hub, unchanged.
    let (bare, hub_override) = split_portable_key(&a.code);
    let mut ag = connect_with_hub(hub_override.as_deref()).await?;
    let hub = ag.hub_url.clone();
    let (room, kind) = ag
        .join(&bare)
        .await
        .map_err(|e| explain_unknown_code(e, &hub, &bare, "parler join"))?;
    println!("✓ joined {} room '{}'", kind.as_str(), room);
    let activation = join_activation_from_environment(a.active, a.passive, a.runner.as_deref());
    if activation.runner().is_some() {
        if join_supports_safe_worker(kind) {
            // A generic invite has no session-style catch-up render. Drain only the initial backlog
            // before handing the cursor to the worker, so an old handoff cannot unexpectedly run when
            // a new agent joins; messages that land after this Pull remain durable for the listener.
            let (backlog, _) = ag.pull(&room, None, None).await?;
            ag.commit_reads(&room).await?;
            if !backlog.is_empty() {
                println!("  caught up on {} prior message(s); the active listener handles new work", backlog.len());
            }
        }
        if start_join_worker(&mut ag, &room, kind, &activation).await? {
            return Ok(());
        }
    }
    println!("  keep a live display:  parler recv --room {room} --watch");
    if matches!(kind, RoomKind::Channel | RoomKind::Dm) {
        println!("  act on signed handoffs: parler work --room {room} --runner codex");
        println!("  start only one listener for this identity/room; don't wait for another human fetch");
    }
    Ok(())
}

async fn cmd_serve(service: String) -> Result<()> {
    let mut ag = connect().await?;
    let room = ag.serve(&service).await?;
    println!("✓ serving '{service}' (room '{room}')");
    println!("  legacy broadcast tasks:  parler recv --room {room}");
    println!("  autonomous role worker: parler supervise --role {service} --runner '<your-agent-command>'");
    Ok(())
}

/// Start the optional local supervisor. The role form is a real anycast queue: it only runs a child
/// after the hub grants an atomic claim; the room form is the useful "body agent" case that keeps one
/// explicitly configured local runner listening to an ongoing session without a human pressing enter.
async fn cmd_supervise(a: SuperviseArgs) -> Result<()> {
    if a.role.is_some() == a.room.is_some() {
        bail!("specify exactly one of --role (anycast queue) or --room (continuous body agent)");
    }
    if a.runner.trim().is_empty() {
        bail!("--runner needs an explicit local command");
    }
    let policy = if Config::exists() { Config::load()?.attention } else { mcp::load_or_bootstrap_config()?.attention };
    let ag = connect().await?;
    let mut runtime = ConnectorRuntime::persistent(ag, policy);
    let (room, kind, role) = match (a.role, a.room) {
        (Some(role), None) => {
            let role = role.trim().to_string();
            if role.is_empty() {
                bail!("--role needs a non-empty role name");
            }
            let room = runtime.agent_mut().serve(&role).await?;
            (room, RoomKind::Service, Some(role))
        }
        (None, Some(room)) => {
            let rooms = runtime.agent_mut().rooms().await?;
            let kind = rooms
                .iter()
                .find(|entry| entry.name == room)
                .map(|entry| entry.kind)
                .ok_or_else(|| anyhow::anyhow!("not a member of room '{room}' — join it before starting a body agent"))?;
            (room, kind, None)
        }
        _ => unreachable!("validated above"),
    };
    work::supervise(
        &mut runtime,
        work::WorkOptions {
            room,
            kind,
            role,
            runner: a.runner,
            lease_secs: a.lease_secs.clamp(15, 3_600),
            once: a.once,
            timeout_secs: a.timeout_secs.max(1),
            max_output_bytes: a.max_output_bytes,
        },
    )
    .await
}

/// `parler bring <agent>` — run another AI agent on some context and hand back its review, no
/// copy-paste. With `--room`, the review is also posted into that session so it surfaces in the
/// conversation via `parler recv`. The heavy lifting (spawning the agent, timeout, remedies) lives
/// in [`bring`]; this is the thin CLI adapter, mirroring the other `cmd_*` wrappers.
async fn cmd_bring(a: BringArgs) -> Result<()> {
    if !bring::is_supported(&a.agent) {
        bail!(
            "don't know how to bring '{}'. Supported: {}",
            a.agent,
            bring::SUPPORTED_AGENTS.join(", ")
        );
    }
    let context = match (a.context.as_deref(), a.context_file.as_deref()) {
        (Some(c), _) => c.to_string(),
        (None, Some("-")) => {
            use std::io::Read;
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
        (None, Some(path)) => std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("couldn't read --context-file {path}: {e}"))?,
        (None, None) => bail!(
            "nothing to review — pass --context \"…\" (what should {} look at?)",
            a.agent
        ),
    };
    if context.trim().is_empty() {
        bail!("the context is empty — give {} something to review", a.agent);
    }

    let prompt = bring::build_prompt(a.instruction.as_deref(), &context);
    let timeout =
        Duration::from_secs(a.timeout_secs.unwrap_or(bring::DEFAULT_TIMEOUT_SECS));
    if !a.quiet {
        eprintln!(
            "⋯ asking {} for a second opinion (up to {}s)…",
            a.agent,
            timeout.as_secs()
        );
    }
    let room = a.room.as_deref().map(str::trim).filter(|r| !r.is_empty());
    let review = match bring::run_review(&a.agent, &prompt, timeout).await {
        Ok(r) => r,
        Err(e) => {
            // The MCP tool runs us detached with stderr nulled, so the room is the only channel
            // back to the host — post the remedy there (best-effort) or the failure is invisible
            // and the host polls parler_recv into a dead end (the #100 phantom-tool trap).
            if let Some(room) = room {
                if let Ok(mut ag) = connect().await {
                    let notice =
                        format!("⚠ second opinion from {} failed: {}", a.agent, e.remedy());
                    let _ = ag.send_text(Target::Room { room: room.to_string() }, &notice).await;
                }
            }
            bail!("{}", e.remedy());
        }
    };

    // Print before posting: the review is already paid for (tokens + minutes), so a hub hiccup on
    // the post must not eat it.
    if !a.quiet {
        println!("{review}");
    }
    if let Some(room) = room {
        let mut ag = connect().await?;
        let body = format!("🔎 second opinion from {} (via parler bring):\n\n{review}", a.agent);
        ag.send_text(Target::Room { room: room.to_string() }, &body).await?;
        if !a.quiet {
            eprintln!("✓ posted the review into session '{room}'");
        }
    }
    Ok(())
}

async fn cmd_session(c: SessionCmd) -> Result<()> {
    // A portable session key may embed the host hub (`<code>@<hub>`) so a joiner whose default hub
    // differs still lands in the right room; dial that hub for the join, the configured one otherwise.
    let hub_override = match &c {
        SessionCmd::Join { key, .. } => split_portable_key(key).1,
        _ => None,
    };
    let mut ag = connect_with_hub(hub_override.as_deref()).await?;
    match c {
        SessionCmd::Open { context, topic, approval, no_approval, ttl, max_uses } => {
            let require_approval = approval && !no_approval;
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
            // Lead with the *portable* key — the trailing `@<hub>` carries this hub, so a joiner on a
            // *different* default hub still lands here without also being told PARLER_HUB (redeemed by
            // `session join <code>@<hub>`). The bare key still works for an agent already on this hub.
            println!("Hand the key to another agent:  parler session join {}@{}", inv.code, ag.hub_url);
            println!("  (already on this hub? the bare key works too:  parler session join {})", inv.code);
            println!("  Codex/Claude agent joins start a safe handoff listener automatically; add --passive to only join.");
            println!("…or launch its MCP server with env  PARLER_SESSION_KEY={}", inv.code);
            println!();
            // Mint the read-only WATCH code up front (same lifetime as the key) so the host has the
            // *right* code for the web/desktop viewer — pasting the join KEY there 401s and reads as
            // "invalid or expired". Best-effort: fall back to the manual command on an older hub.
            match ag.mint_watch_token(&inv.room, Some(ttl.unwrap_or(24 * 3600))).await {
                Ok((code, _)) => {
                    println!("Watch it live (read-only) in the web/desktop viewer — paste this WATCH code, not the key:");
                    println!("    {code}");
                    println!("Session viewer link:  {}", session_view_link(&code));
                    println!("  (re-mint anytime:  parler session watch --room {})", inv.room);
                }
                Err(_) => {
                    println!("Watch it live in your browser:  parler session watch --room {}", inv.room);
                }
            }
        }
        SessionCmd::Join { key, once, active, runner, passive } => {
            // Strip any `@<hub>` (already applied to the connection above) before redeeming the code.
            let code = split_portable_key(&key).0;
            let hub = ag.hub_url.clone();
            // An approval-gated session holds us as a pending request until the host admits us.
            let room = match ag
                .redeem(&code)
                .await
                .map_err(|e| explain_unknown_code(e, &hub, &code, "parler session join"))?
            {
                parler_connector::JoinOutcome::Joined { room, .. } => room,
                parler_connector::JoinOutcome::Pending { room } => {
                    println!("⏳ join request sent — waiting for the host to approve you into '{room}'.");
                    println!("You can't see the conversation until then. Re-run this to check:");
                    println!("  parler session join {key}");
                    return Ok(());
                }
            };
            // Backfill the whole context (since=None); the cursor advance is deferred to an ack (#85),
            // so commit_reads below flushes it — otherwise this one-shot join process exits with the
            // ack in memory and a later recv re-delivers the entire backlog.
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
            ag.commit_reads(&room).await?;
            if once {
                // Fire-and-exit: membership persists, but the joiner leaves the connection — it will
                // show `offline` to the host and won't receive messages live. Intended for scripts.
                println!("send with:  parler send --room {room} \"…\"    receive with:  parler recv --room {room}");
                return Ok(());
            }
            let activation = join_activation_from_environment(active, passive, runner.as_deref());
            if start_join_worker(&mut ag, &room, RoomKind::Channel, &activation).await? {
                return Ok(());
            }
            // Default: hold the connection open so "join" actually means "in the room" — the host
            // sees us `online` and messages arrive live, instead of a fire-and-exit that made a
            // joiner look present for a blink and then vanish (the "2 agents, roster says 1" bug).
            follow_session(&mut ag, &room).await?;
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
            println!("Open this session viewer link:");
            println!("    {}", session_view_link(&token));
            println!();
            println!("The link opens the Parler Protocol website's session viewer to watch the");
            println!("conversation and how many agents are in the room — read-only, no joining. Anyone with");
            println!("this code can read the session, so share it like a password.");
        }
    }
    Ok(())
}

/// Hold the connection open after joining a session so the agent is actually *in* the room:
/// visible as `online` in the host's roster and receiving messages as they land. Without this the
/// CLI `session join` was fire-and-exit — the joiner registered membership, then dropped the
/// connection, so the host saw one member and "couldn't find the other agent." Blocks until Ctrl-C.
async fn follow_session(ag: &mut MeshAgent, room: &str) -> Result<()> {
    let activity = || Some(format!("in session '{room}'"));
    // Announce presence up front so the host's roster flips us to `online` immediately.
    ag.presence("online", activity()).await?;
    let pushing = ag.subscribe().await.unwrap_or(false);
    eprintln!(
        "🟢 in '{room}' — the host now sees you online ({}). Ctrl-C to leave.",
        if pushing { "live push" } else { "polling every 2s" }
    );
    eprintln!("   Send from another shell:  parler send --room {room} \"…\"");
    // Presence decays to `offline` after PRESENCE_STALE_MS (5 min) without a heartbeat, so re-assert
    // it on a slower cadence than the wake loop rather than on every 2s poll.
    let mut last_beat = std::time::Instant::now();
    loop {
        if pushing {
            // Wake on a push for any of our rooms, or fall through every 25s to re-pull + heartbeat.
            let _ = ag.next_delivery(Duration::from_secs(25)).await?;
        } else {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        if last_beat.elapsed() >= Duration::from_secs(120) {
            ag.presence("online", activity()).await?;
            last_beat = std::time::Instant::now();
        }
        let (new, _cur) = ag.pull(room, None, None).await?;
        for m in &new {
            println!("{}", render_message(m));
        }
    }
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

/// Resolve a normal send target plus the additive role-dispatch marker. `--service` intentionally
/// preserves its historical broadcast behavior; only `--role` opts a request into atomic anycast.
fn send_target_from(
    room: Option<String>,
    to: Option<String>,
    service: Option<String>,
    role: Option<String>,
) -> Result<(Target, Option<String>)> {
    match role {
        Some(role) => {
            let role = role.trim().to_string();
            if role.is_empty() {
                bail!("--role needs a non-empty role name");
            }
            if room.is_some() || to.is_some() || service.is_some() {
                bail!("--role cannot be combined with --room, --to, or --service");
            }
            Ok((Target::Service { service: role.clone() }, Some(role)))
        }
        None => Ok((target_from(room, to, service)?, None)),
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
    let (target, role) = send_target_from(a.room, a.to, a.service, a.role)?;
    let text = a.text.join(" ");
    let mut ag = connect().await?;
    let target = resolve_target(&mut ag, target).await?;
    let mut parts = vec![Part::Text(text)];
    if let Some(role) = &role {
        parts.push(DispatchRef { role: role.clone() }.to_part());
    }
    let (_id, seq, room) = ag.send(target, parts, None, None).await?;
    if let Some(role) = role {
        println!("✓ role-dispatched to '{role}' in '{room}' (seq {seq})");
    } else {
        println!("✓ sent to '{room}' (seq {seq})");
    }
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

async fn cmd_task(a: TaskArgs) -> Result<()> {
    let status = TaskStatus::parse(&a.status)
        .ok_or_else(|| anyhow::anyhow!("unknown status '{}' — use one of: {}", a.status, TaskStatus::ALL.join(" | ")))?;
    let target = target_from(a.room, a.to, a.service)?;
    let task = TaskRef {
        status,
        task: a.task,
        note: a.note,
        result: a.result,
        tokens: a.tokens,
        elapsed_ms: a.elapsed_ms,
    };
    let mut ag = connect().await?;
    let target = resolve_target(&mut ag, target).await?;
    let (_id, seq, room) = ag.send(target, vec![task.to_part()], None, None).await?;
    let id = task.task.map(|i| format!(" ({i})")).unwrap_or_default();
    println!("{} task {}{id} posted to '{room}' (seq {seq})", status.marker(), status.label());
    Ok(())
}

async fn cmd_work(a: WorkArgs) -> Result<()> {
    if a.service.is_some() && a.allow_from.is_empty() && !a.allow_any {
        bail!(
            "service workers execute remote model input: pass --allow-from <agent-id> (repeatable), \
             or explicitly opt into every signed requester with --allow-any"
        );
    }
    if a.all_messages && a.service.is_some() {
        bail!("--all-messages is only for a room/session; every service request is already a task");
    }
    let mut ag = connect().await?;
    let (room, source) = match (a.room, a.service) {
        (Some(room), None) => (room, worker::WorkSource::Room),
        (None, Some(service)) => {
            let room = ag.serve(&service).await?;
            (room, worker::WorkSource::Service)
        }
        (None, None) => (
            load_active_session()
                .ok_or_else(|| anyhow::anyhow!("specify --room/--service, or open/join a session first"))?,
            worker::WorkSource::Room,
        ),
        (Some(_), Some(_)) => bail!("specify only one of --room or --service"),
    };
    let runner = worker::ProcessRunner::parse(&a.runner)?;
    let options = worker::WorkOptions {
        source,
        all_messages: a.all_messages,
        allow_from: a.allow_from.into_iter().collect(),
        max_per_hour: a.max_per_hour,
        timeout: Duration::from_secs(a.timeout_secs),
        once: a.once,
    };
    worker::run(&mut ag, &room, &options, &runner).await?;
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

async fn cmd_send_file(a: SendFileArgs) -> Result<()> {
    let target = target_from(a.room, a.to, a.service)?;
    let path = Path::new(&a.path);
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => bail!("cannot read '{}': {e}", a.path),
    };
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_string(),
        None => bail!("'{}' has no file name to send", a.path),
    };
    let media_type = guess_media_type(&name);
    let mut ag = connect().await?;
    let target = resolve_target(&mut ag, target).await?;
    let r = ag.send_file(target, &name, &bytes, media_type, a.note).await?;
    println!("✓ sent file '{name}' to '{}' (seq {}, {} bytes)", r.room, r.seq, bytes.len());
    println!("  blob: {}", r.blob_id);
    println!("  peer: parler fetch {} -o {name}", r.blob_id);
    Ok(())
}

/// Best-effort IANA media type from a file name's extension, for the handful of common types worth
/// labeling. `None` when unknown — the transfer works either way; this is only a display/save hint.
fn guess_media_type(name: &str) -> Option<String> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    let mt = match ext.as_str() {
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "wasm" => "application/wasm",
        _ => return None,
    };
    Some(mt.to_string())
}

async fn cmd_fetch(a: FetchArgs) -> Result<()> {
    let mut ag = connect().await?;
    let bytes = ag.fetch_blob(&a.blob).await?;
    let out = a.out.unwrap_or_else(|| format!("{}.bin", short(&a.blob)));
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
        // A one-shot `parler recv` does a single pull, so the deferred ack (#85) would die with the
        // process — commit it now the batch is rendered, so the next recv sees only newer messages
        // (not the whole history). A `--since`/`--all` re-read is a pure read and never commits. In
        // watch mode this flushes the initial backlog once; the loop below self-acks each later pull.
        if since.is_none() {
            ag.commit_reads(&room).await?;
        }
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
        // Each pull self-acks the previous batch (its `ack` commits what the last pull returned), so
        // the durable cursor tracks the loop. At-least-once: a Ctrl-C between a pull and the next
        // loses at most that last batch's commit — it's re-read on the next recv, never dropped.
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

async fn cmd_delete_room(room: String) -> Result<()> {
    let mut ag = connect().await?;
    ag.delete_room(&room).await?;
    if load_active_session().as_deref() == Some(room.as_str()) {
        clear_active_session()?;
    }
    println!("✓ deleted room '{room}'");
    Ok(())
}

async fn cmd_roster(room: String) -> Result<()> {
    let mut ag = connect().await?;
    let entries = ag.roster(&room).await?;
    println!("members of '{room}':");
    for e in &entries {
        let role = e.role.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
        let act = e.activity.as_deref().map(|a| format!(" — {a}")).unwrap_or_default();
        let attention = e.attention.map(|mode| format!(", {}", mode.as_str())).unwrap_or_default();
        let serving = e.service_role.as_deref().map(|role| format!(" serving:{role}")).unwrap_or_default();
        println!("  {} {}{role}  [{}{attention}]{serving}{act}", e.name, e.id, e.status);
    }
    Ok(())
}

async fn cmd_presence(status: String, activity: Option<String>) -> Result<()> {
    let mut ag = connect().await?;
    ag.presence(&status, activity).await?;
    println!("✓ presence: {status}");
    Ok(())
}

/// Persist the local interruption policy and immediately mirror only its global mode into presence.
/// The hub never receives quiet/muted room names: those are a receiver-side attention boundary.
async fn cmd_attention(a: AttentionArgs) -> Result<()> {
    let mut cfg = if Config::exists() { Config::load()? } else { mcp::load_or_bootstrap_config()? };
    let room_arg = a.room;
    let mode_arg = a.mode;
    match (room_arg.as_deref(), mode_arg.as_deref()) {
        (None, None) => {
            println!("global attention: {}", cfg.attention.mode.as_str());
            let quiet = if cfg.attention.quiet_rooms.is_empty() {
                "(none)".to_string()
            } else {
                cfg.attention.quiet_rooms.join(", ")
            };
            let muted = if cfg.attention.muted_rooms.is_empty() {
                "(none)".to_string()
            } else {
                cfg.attention.muted_rooms.join(", ")
            };
            println!("quiet rooms: {quiet}");
            println!("muted rooms: {muted}");
            return Ok(());
        }
        (None, Some(mode)) => {
            cfg.attention.mode = Attention::parse(mode).ok_or_else(|| {
                anyhow::anyhow!("unknown global attention '{mode}' — use open, dnd, or focus")
            })?;
        }
        (Some(room), Some(mode)) => {
            if room.trim().is_empty() {
                bail!("--room needs a room name");
            }
            let room_mode = RoomAttention::parse(mode).ok_or_else(|| {
                anyhow::anyhow!("unknown room attention '{mode}' — use quiet, muted, or inherit")
            })?;
            cfg.attention.set_room(room.trim(), room_mode);
        }
        (Some(_), None) => bail!("with --room, specify quiet, muted, or inherit"),
    }
    let global = cfg.attention.mode;
    cfg.save()?;
    let mut ag = connect().await?;
    ag.set_attention(global).await?;
    if let Some(room) = room_arg {
        println!(
            "✓ room attention for '{room}' saved locally ({}); global presence remains {}",
            mode_arg.unwrap_or_default(),
            global.as_str()
        );
    } else {
        println!("✓ global attention: {}", global.as_str());
    }
    Ok(())
}

fn cmd_whoami() -> Result<()> {
    let cfg = match Config::load() {
        Ok(cfg) => cfg,
        Err(_) if agent_shell_detected() && !Config::exists() => mcp::load_or_bootstrap_config()?,
        Err(err) => return Err(err),
    };
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
    let attention = e.attention.map(|mode| format!(", {}", mode.as_str())).unwrap_or_default();
    format!("● {}{role}  {}  [{}]  {}{attention}  {}", e.card.name, e.card.id, vis, e.status, tags)
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
    if let Some(attention) = e.attention {
        out.push_str(&format!("attention: {}\n", attention.as_str()));
    }
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
        if let Some(f) = FileRef::from_part(p) {
            let sum = f.summary.map(|s| format!(" — {s}")).unwrap_or_default();
            // The blob id is shown in full so the `parler fetch` command copy-pastes and works.
            out.push(format!("📎 {} ({} bytes){sum} — parler fetch {} -o {}", f.name, f.size, f.blob, f.name));
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
        if let Some(t) = TaskRef::from_part(p) {
            let id = t.task.map(|i| format!(" ({i})")).unwrap_or_default();
            let mut line = format!("{} task {}{id}", t.status.marker(), t.status.label());
            if let Some(n) = &t.note {
                line.push_str(&format!(": {n}"));
            }
            // A result rides the content-addressed blob store, so show the exact fetch command.
            if let Some(blob) = &t.result {
                line.push_str(&format!(" — parler fetch {blob}"));
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

/// The active-session pointer file, keyed to the *identity* (`$PARLER_HOME`, default `~/.parler`) so
/// it sits alongside `config.json`. It used to hard-code `$HOME/.parler/active_session`, which meant
/// two workspaces with distinct `PARLER_HOME`s clobbered one global file (issue #104); deriving it
/// from `home_dir()` gives each identity its own pointer. The default path is unchanged
/// (`home_dir()` == `$HOME/.parler` when `PARLER_HOME` is unset).
fn active_session_path_at(home: &std::path::Path) -> std::path::PathBuf {
    home.join("active_session")
}

pub fn save_active_session(room: &str) -> Result<()> {
    save_active_session_at(&parler_connector::home_dir(), room)
}

fn save_active_session_at(home: &std::path::Path, room: &str) -> Result<()> {
    let path = active_session_path_at(home);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, room)?;
    Ok(())
}

pub fn load_active_session() -> Option<String> {
    load_active_session_at(&parler_connector::home_dir())
}

pub(crate) fn load_active_session_at(home: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(active_session_path_at(home)).ok().map(|s| s.trim().to_string())
}

pub fn clear_active_session() -> Result<()> {
    clear_active_session_at(&parler_connector::home_dir())
}

fn clear_active_session_at(home: &std::path::Path) -> Result<()> {
    let path = active_session_path_at(home);
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
    match kind.as_str() {
        "conversation-prompt" => return conversation::claude::prompt_hook().await,
        "conversation-end" => return conversation::claude::end_hook().await,
        "conversation-wake" => {
            if let Some(reason) = conversation::claude::wake_hook().await? {
                // Claude Code's documented `asyncRewake` contract consumes stderr with exit 2 as
                // the next system reminder. Exit here so the binary wrapper cannot prepend an
                // anyhow `error:` label that would become model input.
                eprintln!("{reason}");
                std::process::exit(2);
            }
            return Ok(());
        }
        _ => {}
    }
    // The Stop/wake hook is the *inbound* path: block briefly for peers' messages and, if any land,
    // print Claude Code's continue-the-turn JSON so the agent keeps polling on its own — no human
    // running `parler recv`. Every other kind below is the *outbound* path (mirror lifecycle events
    // into the session room).
    if matches!(kind.as_str(), "stop" | "Stop" | "wake" | "Wake") {
        return wake_hook().await;
    }

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

    let policy = Config::load().map(|cfg| cfg.attention).unwrap_or_default();
    let ag = connect().await?;
    let mut runtime = ConnectorRuntime::persistent(ag, policy);
    let lifecycle = match kind.as_str() {
        "session-start" | "SessionStart" => Lifecycle::Started,
        "session-end" | "SessionEnd" => Lifecycle::Waiting { activity: Some("session ended".into()) },
        "user-prompt-submit" | "UserPromptSubmit" | "prompt-submit" | "PromptSubmit" => {
            Lifecycle::Working { activity: Some("responding to a prompt".into()) }
        }
        "post-tool-use" | "PostToolUse" | "post-tool-use-failure" | "PostToolUseFailure" => {
            Lifecycle::Working { activity: Some("running tools".into()) }
        }
        _ => return Ok(()),
    };
    runtime.lifecycle(lifecycle).await?;

    let parts = match kind.as_str() {
        "session-start" | "SessionStart" => {
            let cwd = data.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
            vec![Part::Text(format!(
                "🚀 Session started by agent {} in directory {cwd}",
                runtime.agent().name
            ))]
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
        let _ = runtime
            .send(ToolSend {
                target: Target::Room { room },
                parts,
                mentions: None,
                reply_to: None,
            })
            .await;
    }

    Ok(())
}

/// The `Stop`-hook worker (`parler hook stop`): when a Claude Code turn ends, block briefly for new
/// messages from the other agents in the active session and, if any arrive, print the host's
/// continue-the-turn JSON (`{"decision":"block","reason":…}`) so the agent resumes on its own. On a
/// quiet timeout it prints nothing and the turn ends — so back-and-forth keeps flowing while the
/// conversation is live, and stops when it goes quiet.
///
/// Bounded by design: it only runs when the agent is in a session (a local file read, so normal solo
/// turns stay instant and never touch the hub). Policy-admitted batches advance the durable cursor;
/// a quiet/focus hold intentionally does not, and the connector suppresses duplicate injections
/// while that held context is re-read.
async fn wake_hook() -> Result<()> {
    // A visible Claude conversation installs its own asyncRewake adapter. The ordinary user-scope
    // Stop hook is still present, but must not race that adapter or acknowledge its durable turn.
    if std::env::var("PARLER_ACTIVE_CONVERSATION_MANAGED")
        .ok()
        .is_some_and(|value| !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false"))
    {
        return Ok(());
    }
    // No active session → nothing to poll. Keep this before any hub round-trip so a plain Claude Code
    // turn pays zero latency for having Parler Protocol wired in.
    let Some(room) = load_active_session() else {
        return Ok(());
    };

    // Drain the host's Stop payload so the pipe doesn't block; we don't need its contents (the
    // cursor, not `stop_hook_active`, is what keeps us from looping — a drained message never repeats).
    let mut stdin_buffer = String::new();
    use tokio::io::AsyncReadExt;
    let _ = tokio::io::stdin().read_to_string(&mut stdin_buffer).await;

    // How long to wait for a peer at each turn-end before letting the turn stop. Long enough to catch
    // a replying agent, short enough that a finished conversation doesn't hang. Overridable for tests
    // and impatient setups.
    let wait_secs = std::env::var("PARLER_WAKE_WAIT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30);

    let policy = Config::load().map(|cfg| cfg.attention).unwrap_or_default();
    let ag = connect().await?;
    let mut runtime = ConnectorRuntime::persistent(ag, policy);
    // The Stop hook is the moment the host becomes interruptible again. Mirror that transition so
    // role routing sees a waiting worker rather than a stale last tool status.
    runtime
        .lifecycle(Lifecycle::Waiting {
            activity: Some(format!("waiting in session {room}")),
        })
        .await?;
    // Pull → policy-aware receive → host-native injection. Push only lowers latency; a muted room is
    // acknowledged without a wake, while a quiet/focus hold remains durable until attention opens.
    let mut injector = ClaudeStopInjector;
    let _ = runtime
        .listen_until(
            &mut injector,
            &room,
            RoomKind::Channel,
            None,
            None,
            Duration::from_secs(wait_secs),
        )
        .await?;
    Ok(())
}

/// Claude Code's documented Stop-hook injection seam. Other hosts need their own adapter for the
/// same [`HostWakeInjector`] contract; where none exists, `parler work --room …` is the autonomous
/// local process boundary instead of a fiction that MCP can start an idle model turn.
struct ClaudeStopInjector;

#[async_trait::async_trait]
impl HostWakeInjector for ClaudeStopInjector {
    async fn inject(&mut self, wake: WakeRequest) -> Result<()> {
        let body = truncate_wake(
            &wake.messages.iter().map(render_message).collect::<Vec<_>>().join("\n"),
            64 * 1024,
        );
        let reason = format!(
            "New messages from other agents in session '{}':\n{body}\n\n\
             Continue the conversation — reply with parler_send if a response is warranted; \
             otherwise you can stop.",
            wake.room
        );
        println!("{}", serde_json::json!({ "decision": "block", "reason": reason }));
        Ok(())
    }
}

fn truncate_wake(input: &str, cap: usize) -> String {
    if input.len() <= cap {
        return input.to_string();
    }
    let mut end = cap;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[...wake context truncated; use parler_recv for the full durable backlog]", &input[..end])
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
    fn portable_session_key_splits_code_and_hub() {
        // The portable form carries the host hub; the code is stripped before redeeming and the hub
        // dials that host.
        assert_eq!(
            split_portable_key("A3KELDJR@wss://parler-hub.fly.dev"),
            ("A3KELDJR".into(), Some("wss://parler-hub.fly.dev".into()))
        );
        // A bare code is unchanged (no hub override).
        assert_eq!(split_portable_key("A3KELDJR"), ("A3KELDJR".into(), None));
        // The user-facing join link is portable too: extract both the code and the hub before
        // connecting, including links with a trailing slash/query fragment.
        assert_eq!(
            split_portable_key("https://parler-hub.fly.dev/join/A3KELDJR"),
            ("A3KELDJR".into(), Some("https://parler-hub.fly.dev".into()))
        );
        assert_eq!(
            split_portable_key("parler://127.0.0.1:7099/join/CQXL5SJN/?source=desktop#share"),
            ("CQXL5SJN".into(), Some("parler://127.0.0.1:7099".into()))
        );
        // A trailing `@` with no hub is not portable.
        assert_eq!(split_portable_key("A3KELDJR@"), ("A3KELDJR@".into(), None));
    }

    #[test]
    fn private_hub_bind_detection_accepts_only_loopback_hosts() {
        assert!(hub_bind_is_loopback("127.0.0.1:7070"));
        assert!(hub_bind_is_loopback("[::1]:7070"));
        assert!(hub_bind_is_loopback("localhost:7070"));
        assert!(!hub_bind_is_loopback("0.0.0.0:7070"));
        assert!(!hub_bind_is_loopback("[::]:7070"));
        assert!(!hub_bind_is_loopback("hub.example:7070"));
        assert!(!hub_bind_is_loopback("not-an-address"));
    }

    #[test]
    fn session_view_link_attaches_the_watch_code_to_the_website_fragment() {
        assert_eq!(
            session_view_link("943HKUPDZSTA68KDSUG8K6TVXJFMUVCN"),
            "https://www.parlerprotocol.com/hub#sessions&k=943HKUPDZSTA68KDSUG8K6TVXJFMUVCN"
        );
    }

    #[test]
    fn agent_shell_commands_scope_identity_without_changing_human_cli() {
        assert!(command_uses_workspace_identity(&Cmd::Mcp, false));
        assert!(command_uses_workspace_identity(&Cmd::Hook { kind: "stop".into() }, false));
        assert!(command_uses_workspace_identity(&Cmd::Rooms, true));
        assert!(command_uses_workspace_identity(&Cmd::Whoami, true));
        let conversation = Cmd::Conversation(ConversationArgs {
            key: None,
            host: conversation::Host::Codex,
            topic: None,
            resume: None,
            approval: false,
            ttl: None,
            max_uses: None,
        });
        assert!(!command_uses_workspace_identity(&conversation, false));
        assert!(!command_uses_workspace_identity(&conversation, true));
        assert!(!command_uses_workspace_identity(&Cmd::Rooms, false));
        assert!(!command_uses_workspace_identity(&Cmd::Doctor, true));
    }

    #[test]
    fn agent_join_starts_a_safe_worker_only_when_it_can_identify_the_host() {
        let input = |agent_shell, codex, claude, active, passive, runner| JoinActivationInput {
            agent_shell,
            codex,
            claude,
            active,
            passive,
            runner,
        };
        assert_eq!(
            join_activation(input(true, true, false, false, false, None)),
            JoinActivation::Worker("codex".into()),
            "a Codex agent that runs `parler join` immediately owns a safe handoff listener"
        );
        assert_eq!(
            join_activation(input(true, false, true, false, false, None)),
            JoinActivation::Worker("claude".into())
        );
        assert_eq!(
            join_activation(input(true, false, false, false, false, None)),
            JoinActivation::Passive,
            "an unrecognized agent host must not silently spawn an arbitrary runner"
        );
        assert_eq!(
            join_activation(input(true, true, true, false, false, None)),
            JoinActivation::Passive,
            "an ambiguous host environment must not silently choose a runner"
        );
        assert_eq!(
            join_activation(input(false, false, false, true, false, None)),
            JoinActivation::Worker("codex".into()),
            "an explicit --active remains useful from an ordinary terminal"
        );
        assert_eq!(
            join_activation(input(false, false, false, false, false, Some("claude"))),
            JoinActivation::Worker("claude".into()),
            "an explicit runner also activates the worker"
        );
        assert_eq!(
            join_activation(input(true, true, false, false, true, Some("claude"))),
            JoinActivation::Passive,
            "the passive escape hatch wins even if a caller constructs conflicting inputs"
        );

        let options = automatic_join_work_options();
        assert!(!options.all_messages, "ordinary peer text must never run merely because of a join");
        assert!(options.allow_from.is_empty());
        assert_eq!(options.max_per_hour, 20);
        assert_eq!(options.timeout, Duration::from_secs(900));
        assert!(join_supports_safe_worker(RoomKind::Channel));
        assert!(join_supports_safe_worker(RoomKind::Dm));
        assert!(!join_supports_safe_worker(RoomKind::Service));
    }

    #[test]
    fn join_cli_exposes_active_and_passive_paths_without_ambiguous_combinations() {
        let cli = Cli::try_parse_from(["parler", "join", "KEY", "--runner", "claude"]).unwrap();
        let Cmd::Join(args) = cli.cmd else { panic!("join command") };
        assert_eq!(args.runner.as_deref(), Some("claude"));
        assert!(!args.active && !args.passive);

        let cli = Cli::try_parse_from(["parler", "session", "join", "KEY", "--passive"]).unwrap();
        let Cmd::Session(SessionCmd::Join { passive, once, .. }) = cli.cmd else {
            panic!("session join command")
        };
        assert!(passive && !once);

        assert!(Cli::try_parse_from(["parler", "join", "KEY", "--active", "--passive"]).is_err());
        assert!(Cli::try_parse_from(["parler", "session", "join", "KEY", "--once", "--active"]).is_err());
    }

    #[test]
    fn conversation_cli_selects_each_visible_host() {
        let cli = Cli::try_parse_from(["parler", "conversation"]).unwrap();
        let Cmd::Conversation(args) = cli.cmd else { panic!("conversation command") };
        assert_eq!(args.host, conversation::Host::Codex);

        for (value, expected) in [
            ("codex", conversation::Host::Codex),
            ("claude", conversation::Host::Claude),
            ("claude-code", conversation::Host::Claude),
            ("opencode", conversation::Host::Opencode),
            ("open-code", conversation::Host::Opencode),
        ] {
            let cli = Cli::try_parse_from(["parler", "conversation", "--host", value]).unwrap();
            let Cmd::Conversation(args) = cli.cmd else { panic!("conversation command") };
            assert_eq!(args.host, expected);
        }
    }

    #[test]
    fn session_open_defaults_to_immediate_admission_with_approval_opt_in() {
        let cli = Cli::try_parse_from(["parler", "session", "open"]).unwrap();
        let Cmd::Session(SessionCmd::Open { approval, no_approval, .. }) = cli.cmd else {
            panic!("session open command")
        };
        assert!(!approval && !no_approval, "the empty command must not request a gate");

        let cli = Cli::try_parse_from(["parler", "session", "open", "--approval"]).unwrap();
        let Cmd::Session(SessionCmd::Open { approval, no_approval, .. }) = cli.cmd else {
            panic!("session open command")
        };
        assert!(approval && !no_approval, "--approval explicitly enables the gate");

        let cli = Cli::try_parse_from(["parler", "session", "open", "--no-approval"]).unwrap();
        let Cmd::Session(SessionCmd::Open { approval, no_approval, .. }) = cli.cmd else {
            panic!("session open command")
        };
        assert!(!approval && no_approval, "the legacy flag remains accepted as an immediate join");
        assert!(Cli::try_parse_from([
            "parler",
            "session",
            "open",
            "--approval",
            "--no-approval",
        ])
        .is_err());
    }

    #[test]
    fn role_send_is_an_exclusive_anycast_target() {
        let (target, role) = send_target_from(None, None, None, Some(" reviewer ".into())).unwrap();
        assert_eq!(target, Target::Service { service: "reviewer".into() });
        assert_eq!(role.as_deref(), Some("reviewer"));
        assert!(send_target_from(Some("team".into()), None, None, Some("reviewer".into())).is_err());
        assert!(send_target_from(None, None, None, Some("   ".into())).is_err());
    }

    #[test]
    fn unknown_code_error_becomes_a_hub_signpost() {
        // The hub's terminal "unknown invite code" is rewritten to name the hub we tried and the
        // portable form that carries the minting hub — so a wrong-hub hand-off is self-diagnosing.
        let rewritten = explain_unknown_code(
            anyhow::anyhow!("invalid or unknown invite code"),
            "wss://parler-hub.fly.dev",
            "ZX6Y2QPX",
            "parler join",
        );
        let msg = rewritten.to_string();
        assert!(msg.contains("wss://parler-hub.fly.dev"), "names the hub tried: {msg}");
        assert!(msg.contains("parler join ZX6Y2QPX@<that-hub>"), "shows the portable form: {msg}");
        // Any unrelated error is passed through verbatim — we only signpost the hub-mismatch case.
        let other = explain_unknown_code(
            anyhow::anyhow!("connection refused"),
            "wss://h",
            "ZX6Y2QPX",
            "parler join",
        );
        assert_eq!(other.to_string(), "connection refused");
    }

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
        // This check only consumes the supplied Config; persisting it through the process-global
        // PARLER_HOME would make an otherwise pure transport test race unrelated tests.
        let cfg = Config::create(&hub_url, "doctor_test", None).unwrap();

        // Testing check_session_key with a stale key
        let stale_key = "INVALIDKEY";
        let res = check_session_key(&cfg, stale_key, true).await;
        
        assert!(res.is_err());
        let err_msg = res.unwrap_err();
        assert!(err_msg.contains("❌ STALE/CLOSED"));
        assert!(err_msg.contains(stale_key));
        
    }

    /// Write a `config.json` with the given id into `home` — mirrors what `parler mcp` persists on
    /// launch, without touching the process-global `PARLER_HOME` (so it can't race a parallel test).
    fn write_config_json(home: &std::path::Path, id: &str) {
        let body = serde_json::json!({ "hub_url": "ws://h", "id": id, "seed": "x", "name": "claude-code-tam" });
        std::fs::write(home.join("config.json"), body.to_string()).unwrap();
    }

    #[test]
    fn wired_agent_ids_reads_flat_and_per_workspace_configs() {
        // A pure path→ids read: empty before `parler mcp` launches, then the flat config's id, then —
        // because `parler mcp` scopes its identity per workspace under `<home>/ws/<hash>/` — every
        // per-workspace id too, so `--verify` confirms a scoped agent, not just a flat one.
        let home = tempfile::tempdir().unwrap();
        assert!(wired_agent_ids(home.path()).is_empty(), "no config yet → not booted → empty");

        write_config_json(home.path(), "UFLATID000");
        assert_eq!(wired_agent_ids(home.path()), vec!["UFLATID000".to_string()]);

        // Two workspaces each mint their own identity under ws/<hash>.
        let ws_a = home.path().join("ws").join("aaaa");
        let ws_b = home.path().join("ws").join("bbbb");
        std::fs::create_dir_all(&ws_a).unwrap();
        std::fs::create_dir_all(&ws_b).unwrap();
        write_config_json(&ws_a, "UWSAAAA111");
        write_config_json(&ws_b, "UWSBBBB222");
        let mut ids = wired_agent_ids(home.path());
        ids.sort();
        assert_eq!(ids, vec!["UFLATID000".to_string(), "UWSAAAA111".to_string(), "UWSBBBB222".to_string()]);
    }

    #[tokio::test]
    async fn verify_matches_the_wired_id_not_a_same_named_stranger() {
        // #103 AC2: on the shared hub, a stranger can register the same display name. `--verify` must
        // confirm the *wired identity's id* (read from its PARLER_HOME), never a same-named card — an
        // id is the agent's public key, so name collisions can't spoof a dial-in.
        let hub_url = start_hub().await;

        // The wired agent: its identity is saved under a per-host PARLER_HOME (what `parler connect`
        // points the host at, and what `parler mcp` writes on first launch), then it registers.
        let home = tempfile::tempdir().unwrap();
        let wired_cfg = Config::create(&hub_url, "claude-code-tam", None).unwrap();
        let wired_id = wired_cfg.identity.id.clone();
        write_config_json(home.path(), &wired_id);
        let mut wired_agent = MeshAgent::connect(&wired_cfg).await.unwrap();
        wired_agent.register(Visibility::Private, vec![], vec![], None).await.unwrap();

        // A stranger with the *same name* but a different identity, also online.
        let stranger_cfg = Config::create(&hub_url, "claude-code-tam", None).unwrap();
        assert_ne!(stranger_cfg.identity.id, wired_id, "distinct identities");
        let mut stranger = MeshAgent::connect(&stranger_cfg).await.unwrap();
        stranger.register(Visibility::Private, vec![], vec![], None).await.unwrap();

        // What `--verify` sees: the whole directory (two cards sharing the name "claude-code-tam").
        let mut watcher = MeshAgent::connect(&ephemeral_probe_config(&hub_url).unwrap()).await.unwrap();
        let seen = watcher.discover(DiscoverScope::Hub, None, None, None, None, Some(500)).await.unwrap();
        let online_ids: Vec<&str> = seen.iter().map(|e| e.card.id.as_str()).collect();
        assert!(online_ids.iter().filter(|id| **id == wired_id || **id == stranger_cfg.identity.id).count() == 2, "both same-named cards are online");

        // The ids read from the wired home include the wired identity, and never the stranger's.
        let read = wired_agent_ids(home.path());
        assert!(read.contains(&wired_id), "id comes from the wired home, not the name");
        assert!(online_ids.contains(&wired_id.as_str()), "the wired id confirms");
        // The name-based check the old code used would have matched the stranger too — prove the
        // id-based check does not confirm the stranger's id via the wired home.
        assert!(!read.contains(&stranger_cfg.identity.id), "the stranger is never confirmed as the wired agent");
    }

    #[tokio::test]
    async fn ambiguous_name_resolution_errors_with_the_candidates() {
        // #103 AC3: DM-by-name must refuse to silently pick one of several same-named agents. It
        // errors listing every candidate with its id so the sender can disambiguate.
        let hub_url = start_hub().await;
        let a_cfg = Config::create(&hub_url, "claude-code", None).unwrap();
        let mut a = MeshAgent::connect(&a_cfg).await.unwrap();
        a.register(Visibility::Private, vec![], vec![], None).await.unwrap();
        let b_cfg = Config::create(&hub_url, "claude-code", None).unwrap();
        let mut b = MeshAgent::connect(&b_cfg).await.unwrap();
        b.register(Visibility::Private, vec![], vec![], None).await.unwrap();

        let mut sender = MeshAgent::connect(&ephemeral_probe_config(&hub_url).unwrap()).await.unwrap();
        let err = resolve_target(&mut sender, Target::Dm { agent: "claude-code".into() })
            .await
            .expect_err("ambiguous name must error, never silently pick one");
        let msg = err.to_string();
        assert!(msg.contains("matches more than one agent"), "actionable error: {msg}");
        assert!(msg.contains(&a_cfg.identity.id), "lists the first candidate id: {msg}");
        assert!(msg.contains(&b_cfg.identity.id), "lists the second candidate id: {msg}");
    }

    #[test]
    fn active_session_is_scoped_per_home_dir() {
        // Issue #104: the active-session pointer used to hard-code `$HOME/.parler/active_session`, so
        // two workspaces with distinct `PARLER_HOME`s clobbered one global file. Keyed off
        // `home_dir()`, each workspace holds its own active session without stepping on the other.
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();

        save_active_session_at(ws_a.path(), "room-a").unwrap();
        // The pointer lands *inside* this workspace's home, not a shared global path.
        assert!(ws_a.path().join("active_session").exists(), "workspace A owns its own pointer file");

        assert_eq!(load_active_session_at(ws_b.path()), None, "a fresh workspace sees no active session");
        save_active_session_at(ws_b.path(), "room-b").unwrap();

        // Switching back proves B never clobbered A, and vice versa.
        assert_eq!(load_active_session_at(ws_a.path()).as_deref(), Some("room-a"), "workspace A kept its session");
        assert_eq!(load_active_session_at(ws_b.path()).as_deref(), Some("room-b"), "workspace B kept its session");

        // clear only affects the active workspace.
        clear_active_session_at(ws_b.path()).unwrap();
        assert_eq!(load_active_session_at(ws_b.path()), None, "cleared B");
        assert_eq!(load_active_session_at(ws_a.path()).as_deref(), Some("room-a"), "A survives B's clear");
    }
}
