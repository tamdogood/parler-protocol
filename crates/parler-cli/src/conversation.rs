//! One live, interactive conversation surface.
//!
//! `parler conversation [KEY]` is the user-facing composition of the existing durable room,
//! portable invite, backlog, file transport, and host-wake pieces. Each supported host keeps its
//! normal visible session while signed peer messages become real turns in that same conversation.

pub(crate) mod claude;
mod opencode;

use anyhow::{anyhow, bail, Context, Result};
use clap::ValueEnum;
use futures::{SinkExt, StreamExt};
use parler_connector::{verify_message, JoinOutcome, MeshAgent, SigStatus};
use parler_protocol::{FileRef, HandoffRef, MessageSig, Part, RoomKind, StoredMessage, Target, TaskRef, TaskStatus};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::IsTerminal;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

const APP_SERVER_START_TIMEOUT: Duration = Duration::from_secs(10);
const TUI_ATTACH_GRACE: Duration = Duration::from_millis(750);
const JOIN_RETRY: Duration = Duration::from_secs(2);
const RECEIVE_WAIT_SECS: u64 = 25;
const PRESENCE_HEARTBEAT: Duration = Duration::from_secs(60);
const LOCAL_TURN_POLL: Duration = Duration::from_secs(2);
const CODEX_TURN_PAGE_SIZE: u32 = 64;
const CODEX_SEEN_TURNS: usize = 256;
const BACKLOG_PAGE_SIZE: u32 = 1_000;
const MAX_BACKLOG_MESSAGES: usize = 10_000;
const MAX_CONTEXT_CHARS: usize = 24_000;
const CLAUDE_WAKE_PROMPT_CHARS: usize = 9_000;
const MAX_REPLY_CHARS: usize = 16_000;
const MAX_AUTO_FILES: usize = 32;
const MAX_AUTO_FILE_BYTES: u64 = 100 * 1024 * 1024;
const HANDOFF_MARKER: &str = "PARLER_HANDOFF ";

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Host {
    Codex,
    #[value(name = "claude", alias = "claude-code")]
    Claude,
    #[value(alias = "open-code")]
    Opencode,
}

impl Host {
    fn binary(self) -> &'static str {
        match self {
            Host::Codex => "codex",
            Host::Claude => "claude",
            Host::Opencode => "opencode",
        }
    }

    fn display(self) -> &'static str {
        match self {
            Host::Codex => "Codex",
            Host::Claude => "Claude Code",
            Host::Opencode => "OpenCode",
        }
    }

    fn catchup_prompt_chars(self) -> usize {
        match self {
            Host::Claude => CLAUDE_WAKE_PROMPT_CHARS,
            Host::Codex | Host::Opencode => MAX_CONTEXT_CHARS,
        }
    }
}

impl std::fmt::Display for Host {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.binary())
    }
}

pub struct Options {
    pub key: Option<String>,
    pub host: Host,
    pub topic: Option<String>,
    pub resume: Option<String>,
    pub approval: bool,
    pub ttl: Option<u64>,
    pub max_uses: Option<u32>,
}

struct AdapterContext {
    options: Options,
    identity: TuiIdentity,
    cwd: PathBuf,
    hub_override: Option<String>,
    sender: MeshAgent,
}

#[derive(Debug)]
struct RecentIds {
    capacity: usize,
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl RecentIds {
    fn new(capacity: usize) -> RecentIds {
        RecentIds { capacity, order: VecDeque::new(), set: HashSet::new() }
    }

    fn contains(&self, id: &str) -> bool {
        self.set.contains(id)
    }

    fn insert(&mut self, id: String) -> bool {
        if self.capacity == 0 || self.set.contains(&id) {
            return false;
        }
        while self.order.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            }
        }
        self.set.insert(id.clone());
        self.order.push_back(id);
        true
    }

    fn extend<I>(&mut self, ids: I)
    where
        I: IntoIterator<Item = String>,
    {
        for id in ids {
            self.insert(id);
        }
    }
}

type AppSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Start or join a conversation, then keep the selected host's normal interactive UI attached while
/// Parler injects signed peer messages into that same visible conversation.
pub async fn run(options: Options) -> Result<()> {
    let tui_identity = prepare_identity_scope()?;
    ensure_host_available(options.host).await?;
    let cwd = std::env::current_dir()?.canonicalize().unwrap_or(std::env::current_dir()?);
    // A portable key carries its hub. The conversation command owns the whole flow, so unlike a
    // long-lived generic MCP server it can dial that hub without rewriting the user's configuration.
    let hub_override = options.key.as_deref().and_then(|key| crate::split_portable_key(key).1);
    let sender = crate::connect_with_hub(hub_override.as_deref()).await?;
    let host = options.host;
    let context = AdapterContext {
        options,
        identity: tui_identity,
        cwd,
        hub_override,
        sender,
    };
    match host {
        Host::Codex => run_codex(context).await,
        Host::Claude => claude::run(context).await,
        Host::Opencode => opencode::run(context).await,
    }
}

async fn run_codex(context: AdapterContext) -> Result<()> {
    let AdapterContext {
        options,
        identity: tui_identity,
        cwd,
        hub_override,
        mut sender,
    } = context;
    let mut host = CodexHost::start(
        &cwd,
        options.resume.as_deref(),
        &tui_identity,
        &sender,
    )
    .await?;
    let entry = enter_conversation(&mut sender, &options, &host.transcript, Host::Codex.display()).await?;
    let ConversationEntry { room, initial, created, share } = entry;
    crate::save_active_session(&room)?;

    let needs_visible_thread = host.thread_id.is_none();
    let mut tui = host.launch_tui(&cwd, &tui_identity, &sender).await?;
    // Give the visible client one short attach window, and fail immediately if it could not open.
    tokio::time::sleep(TUI_ATTACH_GRACE).await;
    if let Some(status) = tui.try_wait()? {
        bail!("interactive Codex exited before attaching to the live conversation ({status})");
    }
    if needs_visible_thread {
        // A blank remote TUI creates the native thread itself and broadcasts thread/started. Adopt
        // that exact id instead of creating a second bridge-only thread beside the visible one.
        tokio::time::timeout(APP_SERVER_START_TIMEOUT, host.adopt_visible_thread(&cwd))
            .await
            .context("Codex did not announce the visible conversation thread in time")??;
    }
    let thread_id = host.thread_id.clone().ok_or_else(|| anyhow!("Codex thread was not established"))?;

    // The provider-independent preparation validates signatures and materializes files. Codex owns
    // the acknowledgement point: advance only after the visible thread accepted the catch-up turn.
    let backlog = prepare_backlog(&mut sender, &room, &initial, created, Host::Codex).await?;
    if let Some(prompt) = backlog.prompt {
        let _ = host.run_bootstrap_turn(&thread_id, &prompt).await?;
        let cursor = backlog
            .commit_cursor
            .ok_or_else(|| anyhow!("prepared Codex backlog is missing its commit cursor"))?;
        sender.commit_reads_through(&room, cursor).await?;
    }

    announce_arrival(&mut sender, &room, created).await?;

    // Use a second connection for the blocking receive path. One task owns it end-to-end, so a
    // long-poll is never cancelled halfway through a WebSocket request when a TUI event arrives.
    let receiver = crate::connect_with_hub(hub_override.as_deref()).await?;
    let (incoming_tx, incoming_rx) = mpsc::channel(1);
    let receive_task = tokio::spawn(receive_loop(receiver, room.clone(), incoming_tx));

    print_connected(&sender, Host::Codex, share);

    let outcome = coordinate_codex(&mut host, &mut tui, &mut sender, &room, incoming_rx).await;
    receive_task.abort();
    outcome
}

struct TuiIdentity {
    base_home: PathBuf,
    terminal_session: String,
}

/// Give every visible terminal agent its own stable identity, even when two terminals use the same
/// workspace. The child TUI receives the unscoped base + the same private terminal key so its MCP
/// server and any `parler` commands it runs resolve to exactly this identity (without nested scopes).
fn prepare_identity_scope() -> Result<TuiIdentity> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("`parler conversation` needs an interactive terminal because it opens a visible agent session");
    }
    let base_home = parler_connector::home_dir();
    let terminal_session = std::env::var("PARLER_AGENT_SESSION")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            ["TERM_SESSION_ID", "ITERM_SESSION_ID", "WEZTERM_PANE", "TMUX_PANE", "KITTY_WINDOW_ID"]
                .iter()
                .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
        })
        .or_else(|| {
            std::fs::canonicalize("/dev/fd/0")
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
        })
        .ok_or_else(|| anyhow!("could not identify this terminal; set PARLER_AGENT_SESSION to a stable private label"))?;
    std::env::set_var("PARLER_AGENT_SESSION", &terminal_session);
    crate::mcp::scope_identity_to_workspace();
    Ok(TuiIdentity { base_home, terminal_session })
}

async fn ensure_host_available(host: Host) -> Result<()> {
    let status = Command::new(host.binary())
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => bail!("{} is installed but `{} --version` exited with {status}", host.display(), host.binary()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!("{} is not on PATH; install/login to {}, then retry", host.display(), host.display())
        }
        Err(error) => Err(error.into()),
    }
}

/// The identity and policy boundary every visible provider process and invocation-local MCP server
/// must receive. Provider adapters encode this map into their native configuration format.
fn managed_host_environment(identity: &TuiIdentity, agent: &MeshAgent) -> Map<String, Value> {
    let mut environment = Map::new();
    environment.insert("PARLER_HOME".into(), json!(identity.base_home));
    environment.insert("PARLER_AGENT_SESSION".into(), json!(identity.terminal_session));
    environment.insert("PARLER_HUB".into(), json!(agent.hub_url));
    environment.insert("PARLER_NAME".into(), json!(agent.name));
    environment.insert("PARLER_PRESENCE_MANAGED".into(), json!("1"));
    environment.insert("PARLER_ACTIVE_CONVERSATION_MANAGED".into(), json!("1"));
    if let Some(role) = &agent.role {
        environment.insert("PARLER_ROLE".into(), json!(role));
    }
    if let Ok(secret) = std::env::var("PARLER_JOIN_SECRET") {
        if !secret.is_empty() {
            environment.insert("PARLER_JOIN_SECRET".into(), json!(secret));
        }
    }
    environment
}

fn configure_host_process(command: &mut Command, identity: &TuiIdentity, agent: &MeshAgent) {
    for (key, value) in managed_host_environment(identity, agent) {
        if let Some(value) = value.as_str() {
            command.env(key, value);
        }
    }
}

/// Overlay Codex's canonical `parler` MCP entry for this invocation only. `parler connect` normally
/// pins that entry to one saved PARLER_HOME/HUB; without these `-c` overrides, a remote TUI can look
/// correct while its tools quietly use a different identity and hub. Supplying command + args also
/// makes the visible flow self-contained when Codex had no Parler MCP entry yet.
fn configure_parler_mcp(
    command: &mut Command,
    identity: &TuiIdentity,
    agent: &MeshAgent,
) -> Result<()> {
    let executable = std::env::current_exe().context("could not locate the running parler binary")?;
    let values = [
        ("mcp_servers.parler.command", json!(executable)),
        ("mcp_servers.parler.args", json!(["mcp"])),
        ("mcp_servers.parler.enabled", json!(true)),
    ];
    for (key, value) in values {
        command.arg("-c").arg(format!("{key}={}", serde_json::to_string(&value)?));
    }
    for (key, value) in managed_host_environment(identity, agent) {
        command.arg("-c").arg(format!(
            "mcp_servers.parler.env.{key}={}",
            serde_json::to_string(&value)?
        ));
    }
    Ok(())
}

async fn enter_conversation(
    agent: &mut MeshAgent,
    options: &Options,
    transcript: &str,
    transcript_host: &str,
) -> Result<ConversationEntry> {
    match &options.key {
        None => {
            // `--topic` is display/context, not a room identifier. Always ask the hub for a unique
            // room so starting the same topic twice cannot silently reopen an old transcript and
            // expose it through a newly shared key.
            let invite = agent
                .invite_with_approval(
                    RoomKind::Channel,
                    None,
                    options.ttl,
                    options.max_uses,
                    options.approval,
                )
                .await?;
            let mut seed = Vec::new();
            if let Some(topic) = options.topic.as_deref().map(str::trim).filter(|topic| !topic.is_empty()) {
                seed.push(format!("🧭 conversation topic: {topic}"));
            }
            if !transcript.trim().is_empty() {
                seed.push(format!(
                    "📋 context shared from {}'s resumed {transcript_host} conversation:\n{}",
                    agent.name,
                    transcript.trim()
                ));
            }
            if !seed.is_empty() {
                let seed = seed.join("\n\n");
                agent.send_text(Target::Room { room: invite.room.clone() }, &seed).await?;
            }
            let share = print_created(agent, &invite.room, &invite.code, options.approval, options.ttl).await;
            let (initial, _) = agent.pull(&invite.room, None, None).await?;
            Ok(ConversationEntry { room: invite.room, initial, created: true, share: Some(share) })
        }
        Some(key) => {
            let code = crate::split_portable_key(key).0;
            let mut announced = false;
            let room = loop {
                match agent.redeem(&code).await.with_context(|| {
                    format!("could not join this conversation on hub {}", agent.hub_url)
                })? {
                    JoinOutcome::Joined { room, kind: RoomKind::Channel } => break room,
                    JoinOutcome::Joined { room, kind } => {
                        bail!("the key opens a {} endpoint, not a shared conversation ('{room}')", kind.as_str())
                    }
                    JoinOutcome::Pending { room } => {
                        if !announced {
                            announced = true;
                            eprintln!(
                                "⏳ this legacy conversation requires owner approval; waiting in '{room}' (Ctrl-C to stop)"
                            );
                        }
                        tokio::select! {
                            _ = tokio::time::sleep(JOIN_RETRY) => {}
                            _ = tokio::signal::ctrl_c() => bail!("join cancelled while waiting for owner approval"),
                        }
                    }
                }
            };
            let (initial, _) = agent.pull(&room, None, None).await?;
            eprintln!("✓ joined conversation on {} — signed backlog ready", agent.hub_url);
            Ok(ConversationEntry { room, initial, created: false, share: None })
        }
    }
}

struct ConversationEntry {
    room: String,
    initial: Vec<StoredMessage>,
    created: bool,
    share: Option<ShareDetails>,
}

struct PreparedBacklog {
    prompt: Option<String>,
    commit_cursor: Option<i64>,
}

struct BacklogWindow {
    messages: VecDeque<(StoredMessage, usize)>,
    retained_chars: usize,
    total: usize,
    rejected: usize,
    cursor: i64,
}

impl BacklogWindow {
    fn new() -> BacklogWindow {
        BacklogWindow {
            messages: VecDeque::new(),
            retained_chars: 0,
            total: 0,
            rejected: 0,
            cursor: 0,
        }
    }

    fn push(&mut self, room: &str, message: StoredMessage) {
        self.total += 1;
        self.cursor = self.cursor.max(message.seq);
        if !valid_in_conversation(&message, room) {
            self.rejected += 1;
            return;
        }
        let rendered_chars = crate::render_message(&message).chars().count().saturating_add(1);
        self.retained_chars = self.retained_chars.saturating_add(rendered_chars);
        self.messages.push_back((message, rendered_chars));
        while self.retained_chars > MAX_CONTEXT_CHARS && self.messages.len() > 1 {
            if let Some((_, chars)) = self.messages.pop_front() {
                self.retained_chars = self.retained_chars.saturating_sub(chars);
            }
        }
    }

    fn into_messages(self) -> Vec<StoredMessage> {
        self.messages.into_iter().map(|(message, _)| message).collect()
    }
}

/// Validate and materialize a joined backlog once for every provider. A non-empty prompt remains
/// deliberately uncommitted: each adapter advances the durable cursor only after its native host
/// has accepted that context.
async fn prepare_backlog(
    agent: &mut MeshAgent,
    room: &str,
    initial: &[StoredMessage],
    created: bool,
    host: Host,
) -> Result<PreparedBacklog> {
    if initial.is_empty() {
        return Ok(PreparedBacklog { prompt: None, commit_cursor: None });
    }
    if created {
        // The owner already has this resumed transcript in the visible host. Commit the local seed
        // echo without spending a duplicate model turn re-reading it.
        let cursor = initial.last().map(|message| message.seq).unwrap_or_default();
        agent.commit_reads_through(room, cursor).await?;
        return Ok(PreparedBacklog { prompt: None, commit_cursor: None });
    }

    let mut backlog = BacklogWindow::new();
    for message in initial.iter().cloned() {
        backlog.push(room, message);
    }
    loop {
        let remaining = MAX_BACKLOG_MESSAGES.saturating_add(1).saturating_sub(backlog.total);
        let limit = BACKLOG_PAGE_SIZE.min(u32::try_from(remaining).unwrap_or(BACKLOG_PAGE_SIZE));
        let (page, cursor) = agent.pull(room, Some(backlog.cursor), Some(limit)).await?;
        if page.is_empty() {
            break;
        }
        if cursor <= backlog.cursor {
            bail!("conversation backlog pagination did not advance past cursor {}", backlog.cursor);
        }
        let page_len = page.len();
        for message in page {
            backlog.push(room, message);
        }
        if backlog.total > MAX_BACKLOG_MESSAGES {
            bail!(
                "conversation backlog exceeds {MAX_BACKLOG_MESSAGES} messages; start a fresh conversation or consolidate this one before joining"
            );
        }
        if page_len < limit as usize {
            break;
        }
    }

    let rejected = backlog.rejected;
    let commit_cursor = backlog.cursor;
    if rejected > 0 {
        eprintln!("⚠ ignored {rejected} unsigned, invalid, or wrong-conversation backlog message(s)");
    }
    let trusted = backlog.into_messages();
    if trusted.is_empty() {
        agent.commit_reads_through(room, commit_cursor).await?;
        return Ok(PreparedBacklog { prompt: None, commit_cursor: None });
    }

    let files = materialize_backlog_files(agent, &trusted).await;
    let prompt = catchup_prompt(room, &trusted, &files, host);
    Ok(PreparedBacklog {
        prompt: Some(prompt),
        commit_cursor: Some(commit_cursor),
    })
}

async fn announce_arrival(agent: &mut MeshAgent, room: &str, created: bool) -> Result<()> {
    let arrival = if created {
        format!("{} started this live conversation", agent.name)
    } else {
        format!("{} joined this live conversation", agent.name)
    };
    agent.send_text(Target::Room { room: room.to_string() }, &arrival).await?;
    agent
        .presence("waiting", Some(format!("live conversation '{room}'")))
        .await?;
    Ok(())
}

fn print_connected(agent: &MeshAgent, host: Host, share: Option<ShareDetails>) {
    eprintln!(
        "🟢 live conversation connected — peer messages now start turns in this {} window",
        host.display()
    );
    eprintln!(
        "   Exit {} normally to leave. Command/tool approvals remain under your {} policy.",
        host.display(),
        host.display()
    );
    if let Some(share) = share {
        eprintln!();
        eprintln!("   Share:  parler conversation {}@{}", share.code, agent.hub_url);
        if let Some(watch) = share.watch {
            eprintln!("   Viewer: {watch}  (this exact conversation)");
        }
    }
}

struct ShareDetails {
    code: String,
    watch: Option<String>,
}

async fn print_created(
    agent: &mut MeshAgent,
    room: &str,
    code: &str,
    approval: bool,
    ttl: Option<u64>,
) -> ShareDetails {
    println!("✓ live conversation ready on {}", agent.hub_url);
    println!();
    println!("Invite another interactive agent with:");
    println!("  parler conversation {}@{}", code, agent.hub_url);
    println!();
    if approval {
        println!("This key requests access; the owner must approve each new participant.");
    } else {
        println!("Anyone holding this private key can read the context and send agent turns. Keep it secret.");
    }
    let watch = match agent.mint_watch_token(room, Some(ttl.unwrap_or(24 * 3600))).await {
        Ok((watch, _)) => {
            println!("Read-only viewer code for this same conversation:");
            println!("  {watch}");
            println!();
            Some(watch)
        }
        Err(error) => {
            eprintln!("viewer code unavailable: {error}");
            None
        }
    };
    ShareDetails { code: code.to_string(), watch }
}

fn catchup_prompt(room: &str, messages: &[StoredMessage], files: &[String], host: Host) -> String {
    let max_chars = host.catchup_prompt_chars();
    let prefix = format!(
        "You have joined the live Parler conversation '{room}' in this visible {} conversation. Read the \n\
         signed backlog below as conversation context, including any shared-file paths or fetch \n\
         instructions. Catch up silently: do not re-summarize it, call Parler, or claim work merely \n\
         because it appears in history. After this turn, new signed peer messages will arrive here \n\
         automatically.\n\n--- CONVERSATION SO FAR ---\n",
        host.display()
    );
    let closing = "\n--- END CONTEXT ---\n";
    let raw_files = if files.is_empty() {
        String::new()
    } else {
        format!("\nShared files already materialized in this agent's local Parler inbox:\n{}\n", files.join("\n"))
    };
    let fixed_chars = prefix.chars().count() + closing.chars().count();
    let file_budget = max_chars.saturating_sub(fixed_chars).min(max_chars / 4);
    let files = if file_budget == 0 { String::new() } else { clip(&raw_files, file_budget) };
    let context_budget = max_chars
        .saturating_sub(fixed_chars)
        .saturating_sub(files.chars().count());
    let rendered = messages.iter().map(crate::render_message).collect::<Vec<_>>().join("\n");
    let rendered = clip_tail(&rendered, context_budget);
    format!("{prefix}{rendered}{closing}{files}")
}

struct CodexHost {
    server: Child,
    socket: AppSocket,
    url: String,
    next_id: u64,
    buffered: VecDeque<Value>,
    thread_id: Option<String>,
    transcript: String,
    known_turns: RecentIds,
    initially_active: bool,
}

impl CodexHost {
    async fn start(
        cwd: &Path,
        resume: Option<&str>,
        identity: &TuiIdentity,
        agent: &MeshAgent,
    ) -> Result<CodexHost> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let url = format!("ws://127.0.0.1:{port}");
        let mut command = Command::new("codex");
        command
            .arg("app-server");
        configure_parler_mcp(&mut command, identity, agent)?;
        command
            .args(["--listen", &url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // The app-server owns model turns and MCP execution. Give it the same unscoped base +
        // terminal key as the remote TUI, otherwise its nested `parler mcp` would scope twice and
        // appear as a third agent.
        configure_host_process(&mut command, identity, agent);
        let mut server = command
            .spawn()
            .context("failed to launch `codex app-server`; update Codex and retry")?;
        let socket = connect_app_server(&url, &mut server).await?;
        let mut host = CodexHost {
            server,
            socket,
            url,
            next_id: 1,
            buffered: VecDeque::new(),
            thread_id: None,
            transcript: String::new(),
            known_turns: RecentIds::new(CODEX_SEEN_TURNS),
            initially_active: false,
        };
        host.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "parler_protocol",
                    "title": "Parler Protocol live conversation",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "optOutNotificationMethods": ["app/list/updated"]
                }
            }),
        )
        .await
        .context("Codex app-server rejected initialization; update Codex and retry")?;
        host.send(json!({ "method": "initialized", "params": {} })).await?;

        if let Some(requested) = resume {
            let thread_id = if requested.eq_ignore_ascii_case("last") {
                host.latest_thread(cwd).await?
            } else {
                requested.to_string()
            };
            host.request(
                    "thread/resume",
                    json!({ "threadId": thread_id, "cwd": cwd, "includeTurns": false }),
                )
                .await
                .with_context(|| format!("could not resume Codex thread '{thread_id}'"))?;
            let thread = host
                .read_recent_thread(&thread_id)
                .await?
                .unwrap_or_else(|| json!({ "turns": [] }));
            host.known_turns.extend(terminal_thread_ids(&thread));
            host.transcript = transcript_from_thread(&thread);
            host.initially_active = thread_has_running_turn(&thread);
            host.thread_id = Some(thread_id);
            // Notifications generated by our own metadata resume predate the visible TUI client.
            host.buffered.clear();
        }
        Ok(host)
    }

    async fn latest_thread(&mut self, cwd: &Path) -> Result<String> {
        let response = self
            .request(
                "thread/list",
                json!({
                    "cwd": cwd,
                    "limit": 20,
                    "sortKey": "updated_at",
                    "sortDirection": "desc"
                }),
            )
            .await?;
        response["data"]
            .as_array()
            .and_then(|threads| threads.first())
            .and_then(|thread| thread["id"].as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("no resumable Codex thread found in {}; omit --resume to start a new one", cwd.display()))
    }

    async fn launch_tui(&self, cwd: &Path, identity: &TuiIdentity, agent: &MeshAgent) -> Result<Child> {
        let mut command = Command::new("codex");
        if let Some(thread_id) = &self.thread_id {
            command.arg("resume");
            configure_parler_mcp(&mut command, identity, agent)?;
            command.args(["--remote", &self.url, "-C"]).arg(cwd).arg(thread_id);
        } else {
            configure_parler_mcp(&mut command, identity, agent)?;
            command.args(["--remote", &self.url, "-C"]).arg(cwd);
        }
        command.arg("--no-alt-screen");
        // A portable KEY@HUB may intentionally differ from the saved default. Keep the nested MCP
        // server and any agent-run `parler` command on this conversation's exact identity and hub.
        configure_host_process(&mut command, identity, agent);
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("failed to launch the interactive Codex TUI")
    }

    async fn adopt_visible_thread(&mut self, cwd: &Path) -> Result<()> {
        let expected_cwd = cwd.to_string_lossy();
        loop {
            let value = self.next_value().await?;
            if self.handle_bridge_request(&value).await? {
                continue;
            }
            if value["method"] == "thread/started"
                && value["params"]["thread"]["cwd"].as_str() == Some(expected_cwd.as_ref())
            {
                let thread_id = value["params"]["thread"]["id"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Codex announced a visible thread without an id"))?;
                self.thread_id = Some(thread_id.to_string());
                return Ok(());
            }
            // Startup notifications such as app/list/updated are snapshots for the TUI itself and
            // are safe to discard before live turn coordination begins.
        }
    }

    async fn run_bootstrap_turn(&mut self, thread_id: &str, prompt: &str) -> Result<String> {
        let response = self
            .request(
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": [{ "type": "text", "text": prompt }]
                }),
            )
            .await?;
        let turn_id = response["turn"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("Codex accepted the catch-up turn without returning a turn id"))?
            .to_string();
        // This is bridge scaffolding for a late join, not a local-human contribution to publish.
        self.known_turns.insert(turn_id.clone());
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if let Some(thread) = self.read_recent_thread(thread_id).await? {
                if let Some(turn) = thread["turns"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|turn| turn["id"].as_str() == Some(turn_id.as_str()))
                {
                    if let Some(outcome) = terminal_thread_outcome(turn) {
                        self.buffered.clear();
                        if let Some(error) = outcome.error {
                            bail!("Codex could not finish the conversation catch-up turn: {error}");
                        }
                        return Ok(outcome.text);
                    }
                }
            }
        }
    }

    async fn read_recent_thread(&mut self, thread_id: &str) -> Result<Option<Value>> {
        match self
            .request(
                "thread/turns/list",
                codex_turn_page_params(thread_id),
            )
            .await
        {
            Ok(response) => {
                let mut turns = response["data"]
                    .as_array()
                    .cloned()
                    .ok_or_else(|| anyhow!("Codex app-server returned a non-array turn page"))?;
                // The API returns newest-first so the page starts at the stable tail. Downstream
                // transcript and publication code consumes chronological order.
                turns.reverse();
                Ok(Some(json!({ "turns": turns })))
            }
            // A blank remote TUI has a thread id but no rollout file until its first user turn.
            // That is a normal idle state, not a broken app-server connection.
            Err(error) if error.to_string().contains("not materialized yet") => Ok(None),
            Err(error) => Err(error).context("Codex app-server could not list the visible conversation turns"),
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "method": method, "id": id, "params": params })).await?;
        loop {
            let value = self.read_value().await?;
            if value["id"].as_u64() == Some(id) && value.get("method").is_none() {
                if let Some(error) = value.get("error") {
                    bail!("{}", error["message"].as_str().unwrap_or("Codex app-server request failed"));
                }
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
            if !self.handle_bridge_request(&value).await? {
                self.buffered.push_back(value);
            }
        }
    }

    async fn next_value(&mut self) -> Result<Value> {
        match self.buffered.pop_front() {
            Some(value) => Ok(value),
            None => self.read_value().await,
        }
    }

    async fn read_value(&mut self) -> Result<Value> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Text(text))) => return Ok(serde_json::from_str(&text)?),
                Some(Ok(Message::Ping(bytes))) => self.socket.send(Message::Pong(bytes)).await?,
                Some(Ok(Message::Close(_))) | None => bail!("Codex app-server connection closed"),
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error.into()),
            }
        }
    }

    async fn send(&mut self, value: Value) -> Result<()> {
        self.socket.send(Message::Text(serde_json::to_string(&value)?)).await?;
        Ok(())
    }

    /// Handle requests routed to this bridge connection. Codex routes approvals back to the client
    /// that started a turn, so these are peer-injected turns only; human TUI turns keep the TUI's
    /// normal approval flow. A peer can request work but can never grant itself more authority.
    async fn handle_bridge_request(&mut self, value: &Value) -> Result<bool> {
        let Some(method) = value.get("method").and_then(Value::as_str) else { return Ok(false) };
        let Some(id) = value.get("id").cloned() else { return Ok(false) };
        let Some(response) = bridge_server_response(method) else { return Ok(false) };
        let envelope = match response {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(message) => json!({
                "id": id,
                "error": { "code": -32601, "message": message }
            }),
        };
        self.send(envelope).await?;
        Ok(true)
    }
}

/// Safely answer app-server requests for a turn that Parler injected. Each response matches Codex's
/// method-specific schema; no response grants a peer extra permissions or fabricates human input.
fn bridge_server_response(method: &str) -> Option<std::result::Result<Value, &'static str>> {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            Some(Ok(json!({ "decision": "decline" })))
        }
        "item/permissions/requestApproval" => Some(Ok(json!({ "permissions": {} }))),
        "item/tool/requestUserInput" => Some(Ok(json!({ "answers": {} }))),
        "mcpServer/elicitation/request" => {
            Some(Ok(json!({ "action": "decline", "content": null })))
        }
        "item/tool/call" => Some(Ok(json!({
            "success": false,
            "contentItems": [{
                "type": "inputText",
                "text": "A peer-injected turn cannot execute client-side dynamic tools."
            }]
        }))),
        "applyPatchApproval" | "execCommandApproval" => {
            Some(Ok(json!({ "decision": "denied" })))
        }
        "account/chatgptAuthTokens/refresh" | "attestation/generate" => {
            Some(Err("the Parler bridge cannot provide client credentials or attestation"))
        }
        _ => None,
    }
}

impl Drop for CodexHost {
    fn drop(&mut self) {
        let _ = self.server.start_kill();
    }
}

async fn connect_app_server(url: &str, server: &mut Child) -> Result<AppSocket> {
    let deadline = tokio::time::Instant::now() + APP_SERVER_START_TIMEOUT;
    loop {
        match connect_async(url).await {
            Ok((socket, _)) => return Ok(socket),
            Err(error) if tokio::time::Instant::now() < deadline => {
                let _ = error;
                if let Some(status) = server.try_wait()? {
                    bail!("`codex app-server` exited before accepting the visible session ({status}); update Codex and retry");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => {
                return Err(anyhow!(
                    "Codex app-server did not become ready at {url}: {error}. Update Codex and retry"
                ));
            }
        }
    }
}

struct Incoming {
    message: StoredMessage,
    ack: oneshot::Sender<()>,
}

async fn receive_loop(mut agent: MeshAgent, room: String, tx: mpsc::Sender<Incoming>) -> Result<()> {
    let _ = agent.subscribe().await;
    loop {
        let (messages, _, _) = agent.pull_wait(&room, Some(1), RECEIVE_WAIT_SECS).await?;
        for message in messages {
            if !is_actionable(
                &message,
                &room,
                &agent.id,
                &agent.name,
                agent.role.as_deref(),
            ) {
                agent.commit_reads(&room).await?;
                continue;
            }
            let (ack_tx, ack_rx) = oneshot::channel();
            if tx.send(Incoming { message, ack: ack_tx }).await.is_err() {
                return Ok(());
            }
            if ack_rx.await.is_err() {
                return Ok(()); // no ack: leave the durable cursor for a later run
            }
            agent.commit_reads(&room).await?;
        }
    }
}

fn is_actionable(
    message: &StoredMessage,
    room: &str,
    agent_id: &str,
    agent_name: &str,
    agent_role: Option<&str>,
) -> bool {
    if message.from.id == agent_id
        || !valid_in_conversation(message, room)
    {
        return false;
    }
    let handoffs: Vec<HandoffRef> = message.parts.iter().filter_map(HandoffRef::from_part).collect();
    if !handoffs.is_empty() {
        return handoffs.iter().any(|handoff| {
            handoff.is_for(agent_name, agent_role)
                || handoff.to.as_deref().is_some_and(|to| to.eq_ignore_ascii_case(agent_id))
        });
    }
    if message.parts.iter().any(|part| {
        let Part::Text(text) = part else { return false };
        text == &format!("{} started this live conversation", message.from.name)
            || text == &format!("{} joined this live conversation", message.from.name)
    }) {
        return false;
    }
    // Status/result messages are observations, not a reason to start another turn. This is the
    // loop breaker: ordinary visible conversation messages wake peers; bridge-posted replies do not
    // unless they carry an explicit addressed HandoffRef handled above.
    !message.parts.iter().any(|part| TaskRef::from_part(part).is_some())
}

fn valid_in_conversation(message: &StoredMessage, room: &str) -> bool {
    verify_message(&message.from.id, &message.parts, message.reply_to.as_deref()) == SigStatus::Valid
        && MessageSig::from_parts(&message.parts).is_some_and(|signature| {
            matches!(&signature.target, Target::Room { room: signed_room } if signed_room == room)
        })
}

struct TurnCapture {
    incoming: Option<Incoming>,
    text: String,
}

struct ThreadTurnOutcome {
    id: String,
    text: String,
    local_text: String,
    error: Option<String>,
}

fn codex_turn_page_params(thread_id: &str) -> Value {
    json!({
        "threadId": thread_id,
        "limit": CODEX_TURN_PAGE_SIZE,
        "sortDirection": "desc",
        "itemsView": "full"
    })
}

fn terminal_thread_ids(thread: &Value) -> Vec<String> {
    thread["turns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|turn| terminal_thread_outcome(turn).is_some())
        .filter_map(|turn| turn["id"].as_str().map(str::to_string))
        .collect()
}

fn thread_has_running_turn(thread: &Value) -> bool {
    thread["turns"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|turn| turn["status"] == "inProgress")
}

fn terminal_thread_outcome(turn: &Value) -> Option<ThreadTurnOutcome> {
    let id = turn["id"].as_str()?.to_string();
    let status = turn["status"].as_str()?;
    if status == "inProgress" {
        return None;
    }
    let text = turn["items"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .find(|item| item["type"] == "agentMessage")
        .and_then(|item| item["text"].as_str())
        .unwrap_or_default()
        .to_string();
    let local_text = local_conversation_turn(turn, &text);
    let error = if status == "completed" {
        None
    } else {
        Some(
            turn["error"]["message"]
                .as_str()
                .filter(|message| !message.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("turn ended with status '{status}'")),
        )
    };
    Some(ThreadTurnOutcome { id, text, local_text, error })
}

/// A locally typed turn needs both sides of the exchange in the shared conversation. Publishing
/// only the assistant's final sentence loses the question that gave it meaning. Bridge prompts are
/// stripped by `visible_user_transcript`; injected turns still publish only `text` at their call
/// site because the original signed peer message is already in the room.
fn local_conversation_turn(turn: &Value, answer: &str) -> String {
    let prompt = turn["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["type"] == "userMessage")
        .filter_map(|item| {
            let text = item["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|input| input["type"] == "text")
                .filter_map(|input| input["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            visible_user_transcript(&text)
        })
        .map(|(speaker, text)| format!("{speaker}: {text}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    match (prompt.is_empty(), answer.trim().is_empty()) {
        (true, _) => answer.to_string(),
        (false, true) => prompt,
        (false, false) => format!("{prompt}\n\nAgent: {}", answer.trim()),
    }
}

/// Detailed turn notifications can be routed to the visible TUI connection even when the bridge
/// initiated the turn. The canonical thread history is shared by both clients, so it is the source
/// of truth for completed local-human and peer-injected turns alike.
fn collect_unseen_terminal_turns(
    thread: &Value,
    terminal_turns: &mut RecentIds,
) -> Vec<ThreadTurnOutcome> {
    let mut outcomes = Vec::new();
    for turn in thread["turns"].as_array().into_iter().flatten() {
        let Some(turn_id) = turn["id"].as_str() else { continue };
        if terminal_turns.contains(turn_id) {
            continue;
        }
        if let Some(outcome) = terminal_thread_outcome(turn) {
            terminal_turns.insert(turn_id.to_string());
            outcomes.push(outcome);
        }
    }
    outcomes
}

struct PendingStart {
    incoming: Incoming,
}

async fn coordinate_codex(
    host: &mut CodexHost,
    tui: &mut Child,
    sender: &mut MeshAgent,
    room: &str,
    mut incoming_rx: mpsc::Receiver<Incoming>,
) -> Result<()> {
    let thread_id = host.thread_id.clone().ok_or_else(|| anyhow!("Codex thread was not established"))?;
    let mut queued: VecDeque<Incoming> = VecDeque::new();
    let mut pending_starts: HashMap<u64, PendingStart> = HashMap::new();
    let mut turns: HashMap<String, TurnCapture> = HashMap::new();
    let mut active_turns = HashSet::new();
    let mut injected_turns = HashSet::new();
    let mut terminal_turns = std::mem::replace(
        &mut host.known_turns,
        RecentIds::new(CODEX_SEEN_TURNS),
    );
    // Human TUI turns do not reliably emit detailed turn notifications to this second app-server
    // client. Thread status + canonical history therefore gate injection and publication. The sync
    // flag also preserves room ordering: finish publishing a local turn before starting queued peer
    // work that arrived while it was running.
    let mut thread_active = host.initially_active;
    let mut thread_needs_sync = thread_active;
    let mut presence_state = "waiting";
    let mut heartbeat = tokio::time::interval(PRESENCE_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let mut local_turn_poll = tokio::time::interval(LOCAL_TURN_POLL);
    local_turn_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    local_turn_poll.tick().await;

    loop {
        if !thread_active
            && !thread_needs_sync
            && active_turns.is_empty()
            && pending_starts.is_empty()
            && injected_turns.is_empty()
        {
            if let Some(incoming) = queued.pop_front() {
                let prompt = live_turn_prompt(sender, room, &incoming.message, Host::Codex.display()).await;
                let request_id = host.next_id;
                host.next_id += 1;
                host.send(json!({
                    "method": "turn/start",
                    "id": request_id,
                    "params": {
                        "threadId": thread_id,
                        "input": [{ "type": "text", "text": prompt }]
                    }
                }))
                .await?;
                pending_starts.insert(request_id, PendingStart { incoming });
            }
        }

        tokio::select! {
            status = tui.wait() => {
                let status = status?;
                if !status.success() {
                    bail!("interactive Codex exited with {status}");
                }
                return Ok(());
            }
            maybe = incoming_rx.recv() => {
                match maybe {
                    Some(incoming) => queued.push_back(incoming),
                    None => bail!("conversation receive loop stopped"),
                }
            }
            _ = heartbeat.tick() => {
                let _ = sender.presence(presence_state, Some(format!("live conversation '{room}'"))).await;
            }
            _ = local_turn_poll.tick(), if pending_starts.is_empty() && (thread_active || thread_needs_sync) => {
                let Some(thread) = host.read_recent_thread(&thread_id).await? else {
                    // A brand-new idle TUI has no rollout file yet. There is nothing to publish,
                    // so an idle-status notification must not strand queued peer work behind sync.
                    if !thread_active {
                        thread_needs_sync = false;
                    }
                    continue;
                };
                thread_active = thread_has_running_turn(&thread);
                let next_state = if thread_active { "working" } else { "waiting" };
                if next_state != presence_state {
                    let _ = sender.presence(next_state, Some(format!("live conversation '{room}'"))).await;
                    presence_state = next_state;
                }
                for outcome in collect_unseen_terminal_turns(&thread, &mut terminal_turns) {
                    active_turns.remove(&outcome.id);
                    if injected_turns.remove(&outcome.id) {
                        let mut capture = turns
                            .remove(&outcome.id)
                            .ok_or_else(|| anyhow!("Codex completed an injected turn without its delivery context"))?;
                        if let Some(error) = outcome.error {
                            // Dropping the capture drops its ack, leaving the durable cursor for a
                            // later retry instead of pretending the peer request was delivered.
                            drop(capture);
                            bail!("Codex could not finish an injected peer turn: {error}");
                        }
                        capture.text = outcome.text;
                        publish_turn(sender, room, capture).await?;
                    } else if outcome.error.is_none() {
                        publish_turn(
                            sender,
                            room,
                            TurnCapture { incoming: None, text: outcome.local_text },
                        )
                        .await?;
                    }
                }
                thread_needs_sync = false;
            }
            value = host.next_value() => {
                let value = value?;
                if host.handle_bridge_request(&value).await? {
                    continue;
                }
                if value.get("method").is_none() {
                    if let Some(id) = value["id"].as_u64() {
                        if let Some(pending) = pending_starts.remove(&id) {
                            if let Some(error) = value.get("error") {
                                let message = error["message"].as_str().unwrap_or_default();
                                if message.contains("turn") && message.contains("running") {
                                    queued.push_front(pending.incoming);
                                    thread_active = true;
                                    thread_needs_sync = true;
                                } else {
                                    bail!("Codex rejected a peer turn: {message}");
                                }
                            } else if let Some(turn_id) = value["result"]["turn"]["id"].as_str() {
                                injected_turns.insert(turn_id.to_string());
                                let capture = turns.entry(turn_id.to_string()).or_insert(TurnCapture {
                                    incoming: None,
                                    text: String::new(),
                                });
                                capture.incoming = Some(pending.incoming);
                            }
                        }
                    }
                    continue;
                }
                match value["method"].as_str().unwrap_or_default() {
                    "turn/started" => {
                        if let Some(turn_id) = value["params"]["turn"]["id"].as_str() {
                            thread_active = true;
                            active_turns.insert(turn_id.to_string());
                            turns.entry(turn_id.to_string()).or_insert(TurnCapture { incoming: None, text: String::new() });
                            if presence_state != "working" {
                                let _ = sender.presence("working", Some(format!("live conversation '{room}'"))).await;
                                presence_state = "working";
                            }
                        }
                    }
                    "thread/status/changed" => {
                        if value["params"]["threadId"].as_str() == Some(thread_id.as_str()) {
                            let next_state = match value["params"]["status"]["type"].as_str() {
                                Some("active") => {
                                    thread_active = true;
                                    Some("working")
                                }
                                Some("idle" | "systemError") => {
                                    thread_active = false;
                                    thread_needs_sync = true;
                                    Some("waiting")
                                }
                                _ => None,
                            };
                            if let Some(next_state) = next_state {
                                if next_state != presence_state {
                                    let _ = sender
                                        .presence(next_state, Some(format!("live conversation '{room}'")))
                                        .await;
                                    presence_state = next_state;
                                }
                            }
                        }
                    }
                    "item/completed" => {
                        if value["params"]["item"]["type"] == "agentMessage" {
                            if let (Some(turn_id), Some(text)) = (
                                value["params"]["turnId"].as_str(),
                                value["params"]["item"]["text"].as_str(),
                            ) {
                                turns.entry(turn_id.to_string())
                                    .or_insert(TurnCapture { incoming: None, text: String::new() })
                                    .text = text.to_string();
                            }
                        }
                    }
                    "turn/completed" => {
                        if let Some(turn_id) = value["params"]["turn"]["id"].as_str() {
                            active_turns.remove(turn_id);
                            let capture = turns.remove(turn_id);
                            // A notification can arrive on this connection without its matching
                            // start/item events. In that case leave the turn unseen so canonical
                            // thread polling can publish it with complete context.
                            let first_terminal = capture.is_some()
                                && terminal_turns.insert(turn_id.to_string());
                            thread_active = false;
                            thread_needs_sync = true;
                            if let Some(error) = incomplete_turn_reason(&value["params"]["turn"]) {
                                injected_turns.remove(turn_id);
                                if first_terminal && capture.as_ref().is_some_and(|capture| capture.incoming.is_some()) {
                                    // Dropping the capture drops its ack. The receive task therefore
                                    // leaves the durable cursor untouched for a later retry.
                                    bail!("Codex could not finish an injected peer turn: {error}");
                                }
                                if active_turns.is_empty() && presence_state != "waiting" {
                                    let _ = sender.presence("waiting", Some(format!("live conversation '{room}'"))).await;
                                    presence_state = "waiting";
                                }
                                continue;
                            }
                            if first_terminal {
                                if let Some(mut capture) = capture {
                                    if let Some(outcome) = terminal_thread_outcome(&value["params"]["turn"]) {
                                        capture.text = if capture.incoming.is_some() {
                                            outcome.text
                                        } else {
                                            outcome.local_text
                                        };
                                    } else if capture.incoming.is_none() {
                                        capture.text = local_conversation_turn(&value["params"]["turn"], &capture.text);
                                    }
                                    publish_turn(sender, room, capture).await?;
                                }
                            }
                            injected_turns.remove(turn_id);
                            if active_turns.is_empty() && presence_state != "waiting" {
                                let _ = sender.presence("waiting", Some(format!("live conversation '{room}'"))).await;
                                presence_state = "waiting";
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn incomplete_turn_reason(turn: &Value) -> Option<String> {
    match turn["status"].as_str() {
        Some("completed") => None,
        Some(status) => Some(
            turn["error"]["message"]
                .as_str()
                .filter(|message| !message.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("turn ended with status '{status}'")),
        ),
        None => Some("turn completed without a status".into()),
    }
}

async fn live_turn_prompt(agent: &mut MeshAgent, room: &str, message: &StoredMessage, host: &str) -> String {
    let files = materialize_files(agent, message).await;
    let file_lines = if files.is_empty() {
        String::new()
    } else {
        format!("\nShared files materialized locally:\n{}\n", files.join("\n"))
    };
    format!(
        "A cryptographically signed peer message arrived in your live Parler conversation '{room}'. \n\
         Continue the conversation naturally in this visible {host} conversation and act on any request in \n\
         the current workspace. Do not merely say you received it. Your final response is shared back \n\
         automatically; do not call Parler yourself. If a specific participant should take another \n\
         autonomous turn after you, put exactly one marker on the final non-empty line:\n\
         PARLER_HANDOFF {{\"to\":\"agent-name-or-role\",\"next\":\"specific next step\",\"summary\":\"what you completed\"}}\n\
         Otherwise omit that marker, which ends this turn without an accidental reply loop.\n\n\
         PEER MESSAGE:\n{}{}",
        crate::render_message(message),
        file_lines
    )
}

async fn materialize_files(agent: &mut MeshAgent, message: &StoredMessage) -> Vec<String> {
    let mut paths = Vec::new();
    let mut count = 0_usize;
    let mut total = 0_u64;
    let mut omitted = 0_usize;
    for file in message.parts.iter().filter_map(FileRef::from_part) {
        if count >= MAX_AUTO_FILES || total.saturating_add(file.size) > MAX_AUTO_FILE_BYTES {
            omitted += 1;
            continue;
        }
        count += 1;
        total = total.saturating_add(file.size);
        match materialize_file(agent, &file).await {
            Ok(path) => paths.push(format!("- {}", path.display())),
            Err(error) => paths.push(format!("- {} could not be downloaded: {error}", file.name)),
        }
    }
    if omitted > 0 {
        paths.push(format!("- {omitted} additional file(s) not downloaded: per-turn limit reached"));
    }
    paths
}

async fn materialize_backlog_files(agent: &mut MeshAgent, messages: &[StoredMessage]) -> Vec<String> {
    let mut files = Vec::new();
    let mut count = 0_usize;
    let mut total = 0_u64;
    let mut omitted = 0_usize;
    for message in messages {
        for file in message.parts.iter().filter_map(FileRef::from_part) {
            if count >= MAX_AUTO_FILES || total.saturating_add(file.size) > MAX_AUTO_FILE_BYTES {
                omitted += 1;
                continue;
            }
            count += 1;
            total = total.saturating_add(file.size);
            match materialize_file(agent, &file).await {
                Ok(path) => files.push(format!("- {}", path.display())),
                Err(error) => files.push(format!("- {} could not be downloaded: {error}", file.name)),
            }
        }
    }
    if omitted > 0 {
        files.push(format!("- {omitted} additional file(s) not downloaded: catch-up limit reached"));
    }
    files.sort();
    files.dedup();
    files
}

async fn materialize_file(agent: &mut MeshAgent, file: &FileRef) -> Result<PathBuf> {
    if !is_content_id(&file.blob) {
        bail!("signed file reference contains an invalid SHA-256 content id");
    }
    let name = Path::new(&file.name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .unwrap_or("shared-file");
    let dir = parler_connector::home_dir().join("inbox").join(&file.blob);
    let path = dir.join(name);
    if let Ok(existing) = tokio::fs::read(&path).await {
        if existing.len() as u64 == file.size && parler_auth::content_id(&existing) == file.blob {
            return Ok(path);
        }
    }
    let bytes = agent.fetch_blob(&file.blob).await?;
    if bytes.len() as u64 != file.size {
        bail!(
            "downloaded {} bytes but the signed file reference declares {}",
            bytes.len(),
            file.size
        );
    }
    let actual = parler_auth::content_id(&bytes);
    if actual != file.blob {
        bail!("downloaded content hash {actual} does not match signed blob id {}", file.blob);
    }
    let write_path = path.clone();
    tokio::task::spawn_blocking(move || parler_auth::write_private_file(&write_path, &bytes))
        .await
        .context("shared-file writer stopped")??;
    Ok(path)
}

fn is_content_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

async fn publish_turn(agent: &mut MeshAgent, room: &str, capture: TurnCapture) -> Result<()> {
    let text = capture.text.trim();
    if text.is_empty() {
        if let Some(incoming) = capture.incoming {
            let _ = incoming.ack.send(());
        }
        return Ok(());
    }
    match capture.incoming {
        None => {
            // A turn typed by the local human belongs to the live conversation and wakes peers.
            agent.send_text(Target::Room { room: room.to_string() }, &clip(text, MAX_REPLY_CHARS)).await?;
        }
        Some(incoming) => {
            send_peer_result(agent, room, &incoming.message, text).await?;
            let _ = incoming.ack.send(());
        }
    }
    Ok(())
}

/// All visible-host adapters publish the same terminal receipt. The TaskRef prevents an automatic
/// result from waking every peer again; an explicit addressed handoff is the only continuation.
async fn send_peer_result(
    agent: &mut MeshAgent,
    room: &str,
    incoming: &StoredMessage,
    text: &str,
) -> Result<()> {
    let (body, continuation) = parse_handoff(text);
    let task = TaskRef {
        status: TaskStatus::Done,
        task: Some(incoming.id.clone()),
        note: None,
        result: None,
        tokens: None,
        elapsed_ms: None,
    };
    let mut parts = vec![Part::text(clip(&body, MAX_REPLY_CHARS)), task.to_part()];
    let mentions = continuation
        .as_ref()
        .and_then(|handoff| handoff.to.clone())
        .map(|to| vec![to]);
    if let Some(handoff) = continuation {
        parts.push(handoff.to_part());
    }
    agent
        .send(
            Target::Room { room: room.to_string() },
            parts,
            mentions,
            Some(incoming.id.clone()),
        )
        .await?;
    Ok(())
}

fn parse_handoff(output: &str) -> (String, Option<HandoffRef>) {
    let output = output.trim();
    let Some((body, line)) = output.rsplit_once('\n') else {
        return (output.to_string(), None);
    };
    let Some(payload) = line.trim().strip_prefix(HANDOFF_MARKER) else {
        return (output.to_string(), None);
    };
    let Ok(handoff) = serde_json::from_str::<HandoffRef>(payload) else {
        return (output.to_string(), None);
    };
    let valid = handoff.to.as_deref().is_some_and(|to| !to.trim().is_empty())
        && !handoff.next.trim().is_empty()
        && !body.trim().is_empty();
    if valid {
        (body.trim().to_string(), Some(handoff))
    } else {
        (output.to_string(), None)
    }
}

fn transcript_from_thread(thread: &Value) -> String {
    let mut lines = Vec::new();
    for turn in thread["turns"].as_array().into_iter().flatten() {
        for item in turn["items"].as_array().into_iter().flatten() {
            match item["type"].as_str() {
                Some("userMessage") => {
                    let text = item["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|input| input["type"] == "text")
                        .filter_map(|input| input["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if let Some((speaker, text)) = visible_user_transcript(&text) {
                        lines.push(format!("{speaker}: {text}"));
                    }
                }
                Some("agentMessage") => {
                    if let Some(text) = item["text"].as_str().map(str::trim).filter(|text| !text.is_empty()) {
                        lines.push(format!("Agent: {text}"));
                    }
                }
                _ => {}
            }
        }
    }
    clip_tail(&lines.join("\n\n"), MAX_CONTEXT_CHARS)
}

/// Remove Parler's delivery instructions when a live thread is shared again. The visible peer text
/// and catch-up content remain, but internal wake/loop-control scaffolding is not recursively copied
/// into every later conversation.
fn visible_user_transcript(text: &str) -> Option<(&'static str, String)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.starts_with("A cryptographically signed peer message arrived") {
        return text
            .split_once("PEER MESSAGE:\n")
            .map(|(_, message)| ("Peer", message.trim().to_string()))
            .filter(|(_, message)| !message.is_empty());
    }
    if text.starts_with("You have joined the live Parler conversation") {
        return text
            .split_once("--- CONVERSATION SO FAR ---\n")
            .and_then(|(_, rest)| rest.split_once("\n--- END CONTEXT ---"))
            .map(|(context, _)| ("Conversation context", context.trim().to_string()))
            .filter(|(_, context)| !context.is_empty());
    }
    Some(("User", text.to_string()))
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut clipped = text.chars().take(max_chars.saturating_sub(1)).collect::<String>();
    clipped.push('…');
    clipped
}

fn clip_tail(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let marker = "… earlier context omitted …\n";
    let marker_chars = marker.chars().count();
    if max_chars <= marker_chars {
        return text.chars().skip(count - max_chars).collect();
    }
    let tail_chars = max_chars - marker_chars;
    let tail = text.chars().skip(count - tail_chars).collect::<String>();
    format!("{marker}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use parler_connector::Config;
    use parler_protocol::{canonical_message_bytes, EndpointRef, MessageSig};
    use std::sync::Arc;

    async fn start_hub() -> (String, parler_hub::Store) {
        let store = parler_hub::Store::open(None).unwrap();
        let state = Arc::new(parler_hub::HubState::new(
            store.clone(),
            "parler://test".into(),
            "Conversation Test".into(),
            parler_hub::HubMode::Private,
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = parler_hub::serve(listener, state).await;
        });
        (format!("ws://{addr}"), store)
    }

    fn signed_message(parts: Vec<Part>) -> StoredMessage {
        StoredMessage {
            seq: 1,
            id: "msg-1".into(),
            room: "conversation".into(),
            from: EndpointRef { id: "peer".into(), name: "peer".into(), role: None },
            parts,
            mentions: None,
            reply_to: None,
            ts: 1,
        }
    }

    fn valid_message(parts: Vec<Part>) -> StoredMessage {
        let identity = parler_auth::new_identity().unwrap();
        let target = Target::Room { room: "conversation".into() };
        let ts = 1_710_000_000_000;
        let uid = "conversation-test-uid";
        let bytes = canonical_message_bytes(&identity.id, &target, &parts, None, ts, uid);
        let sig = parler_auth::sign(&identity.seed, &bytes).unwrap();
        let mut signed_parts = parts;
        signed_parts.push(MessageSig { sig, ts, uid: uid.into(), target }.to_part());
        StoredMessage {
            seq: 1,
            id: "msg-signed".into(),
            room: "conversation".into(),
            from: EndpointRef { id: identity.id, name: "peer".into(), role: None },
            parts: signed_parts,
            mentions: None,
            reply_to: None,
            ts: 1,
        }
    }

    #[test]
    fn explicit_handoff_marker_is_removed_and_validated() {
        let (body, handoff) = parse_handoff(
            "Finished the audit.\nPARLER_HANDOFF {\"to\":\"reviewer\",\"next\":\"verify it\",\"summary\":\"audit done\"}",
        );
        assert_eq!(body, "Finished the audit.");
        assert_eq!(handoff.unwrap().to.as_deref(), Some("reviewer"));

        let (body, handoff) = parse_handoff("Keep this\nPARLER_HANDOFF {\"next\":\"broadcast\"}");
        assert!(handoff.is_none());
        assert!(body.contains("PARLER_HANDOFF"));
    }

    #[test]
    fn transcript_exports_only_visible_user_and_agent_messages() {
        let thread = json!({
            "turns": [{
                "items": [
                    {"type":"userMessage", "content":[{"type":"text", "text":"Please audit."}]},
                    {"type":"reasoning", "summary":["private chain"]},
                    {"type":"commandExecution", "command":"secret"},
                    {"type":"agentMessage", "text":"Audit complete."}
                ]
            }]
        });
        assert_eq!(transcript_from_thread(&thread), "User: Please audit.\n\nAgent: Audit complete.");
    }

    #[test]
    fn transcript_keeps_peer_content_without_recursive_bridge_instructions() {
        let thread = json!({
            "turns": [{
                "items": [{
                    "type":"userMessage",
                    "content":[{"type":"text", "text":
                        "A cryptographically signed peer message arrived in your live Parler conversation 'audit'.\n\
                         Do work.\n\nPEER MESSAGE:\n[2] alice: Check auth.rs"
                    }]
                }]
            }]
        });
        let transcript = transcript_from_thread(&thread);
        assert_eq!(transcript, "Peer: [2] alice: Check auth.rs");
        assert!(!transcript.contains("cryptographically signed"));
    }

    #[test]
    fn shared_thread_poll_reports_each_new_terminal_turn_once() {
        let thread = json!({
            "turns": [
                {"id":"old", "status":"completed", "items":[{"type":"agentMessage", "text":"old reply"}]},
                {"id":"peer-injected", "status":"completed", "items":[{"type":"agentMessage", "text":"peer reply"}]},
                {"id":"still-running", "status":"inProgress", "items":[]},
                {"id":"failed", "status":"failed", "items":[]},
                {"id":"local", "status":"completed", "items":[
                    {"type":"userMessage", "content":[{"type":"text", "text":"What was the result?"}]},
                    {"type":"agentMessage", "text":"draft"},
                    {"type":"agentMessage", "text":"visible final reply"}
                ]}
            ]
        });
        let mut terminal = RecentIds::new(CODEX_SEEN_TURNS);
        terminal.insert("old".to_string());

        let outcomes = collect_unseen_terminal_turns(&thread, &mut terminal);
        assert_eq!(outcomes.len(), 3);
        let local = outcomes.iter().find(|outcome| outcome.id == "local").unwrap();
        assert_eq!(local.text, "visible final reply");
        assert_eq!(
            local.local_text,
            "User: What was the result?\n\nAgent: visible final reply"
        );
        assert!(local.error.is_none());
        assert!(outcomes.iter().find(|outcome| outcome.id == "failed").unwrap().error.is_some());
        assert!(terminal.contains("failed"));
        assert!(terminal.contains("local"));
        assert!(terminal.contains("peer-injected"));
        assert!(!terminal.contains("still-running"));
        assert!(thread_has_running_turn(&thread));

        assert!(collect_unseen_terminal_turns(&thread, &mut terminal).is_empty());
    }

    #[test]
    fn recent_ids_evict_oldest_entries_at_the_configured_bound() {
        let mut ids = RecentIds::new(2);
        assert!(ids.insert("one".into()));
        assert!(ids.insert("two".into()));
        assert!(!ids.insert("two".into()));
        assert!(ids.insert("three".into()));
        assert!(!ids.contains("one"));
        assert!(ids.contains("two"));
        assert!(ids.contains("three"));
    }

    #[test]
    fn codex_turn_reads_are_bounded_newest_first_full_pages() {
        let params = codex_turn_page_params("thread-1");
        assert_eq!(params["threadId"], "thread-1");
        assert_eq!(params["limit"], CODEX_TURN_PAGE_SIZE);
        assert_eq!(params["sortDirection"], "desc");
        assert_eq!(params["itemsView"], "full");
    }

    #[test]
    fn lifecycle_message_is_not_a_fresh_conversation_turn() {
        let message = valid_message(vec![Part::text("peer joined this live conversation")]);
        assert!(!is_actionable(&message, "conversation", "U_ME", "codex", None));
    }

    #[test]
    fn only_signed_conversation_or_addressed_handoff_messages_wake_the_agent() {
        let ordinary = valid_message(vec![Part::text("please review")]);
        assert!(is_actionable(&ordinary, "conversation", "U_ME", "codex", Some("reviewer")));

        assert!(!is_actionable(
            &ordinary,
            "another-conversation",
            "U_ME",
            "codex",
            Some("reviewer")
        ));

        let task = TaskRef {
            status: TaskStatus::Done,
            task: Some("request".into()),
            note: None,
            result: None,
            tokens: None,
            elapsed_ms: None,
        };
        let result_only = valid_message(vec![Part::text("done"), task.to_part()]);
        assert!(!is_actionable(
            &result_only,
            "conversation",
            "U_ME",
            "codex",
            Some("reviewer")
        ));

        let for_me = valid_message(vec![
            Part::text("continue"),
            task.to_part(),
            HandoffRef {
                to: Some("reviewer".into()),
                next: "verify the result".into(),
                summary: None,
                bundle: None,
            }
            .to_part(),
        ]);
        assert!(is_actionable(
            &for_me,
            "conversation",
            "U_ME",
            "codex",
            Some("reviewer")
        ));

        let for_someone_else = valid_message(vec![
            Part::text("continue"),
            HandoffRef {
                to: Some("writer".into()),
                next: "draft docs".into(),
                summary: None,
                bundle: None,
            }
            .to_part(),
        ]);
        assert!(!is_actionable(
            &for_someone_else,
            "conversation",
            "U_ME",
            "codex",
            Some("reviewer")
        ));
    }

    #[test]
    fn catchup_context_keeps_file_fetch_instructions() {
        let message = signed_message(vec![FileRef {
            blob: "a".repeat(64),
            name: "report.pdf".into(),
            size: 12,
            media_type: Some("application/pdf".into()),
            summary: None,
        }
        .to_part()]);
        let prompt = catchup_prompt("audit", &[message], &[], Host::Codex);
        assert!(prompt.contains("report.pdf"));
        assert!(prompt.contains("parler fetch"));
    }

    #[test]
    fn catchup_context_names_materialized_files() {
        let prompt = catchup_prompt(
            "audit",
            &[signed_message(vec![Part::text("read the report")])],
            &["- /tmp/parler/inbox/abc/report.pdf".into()],
            Host::Codex,
        );
        assert!(prompt.contains("already materialized"));
        assert!(prompt.contains("/tmp/parler/inbox/abc/report.pdf"));
    }

    #[test]
    fn claude_catchup_fits_its_native_wake_limit_and_keeps_the_newest_context() {
        let messages = (0..100)
            .map(|index| {
                signed_message(vec![Part::text(format!(
                    "catchup-{index:03} {}",
                    "x".repeat(160)
                ))])
            })
            .collect::<Vec<_>>();
        let prompt = catchup_prompt("audit", &messages, &[], Host::Claude);
        assert!(prompt.chars().count() <= CLAUDE_WAKE_PROMPT_CHARS);
        assert!(prompt.contains("catchup-099"));
        assert!(!prompt.contains("catchup-000"));
        assert!(prompt.contains("--- END CONTEXT ---"));
    }

    #[test]
    fn tail_clipping_includes_its_omission_marker_inside_the_bound() {
        let clipped = clip_tail(&"x".repeat(1_000), 100);
        assert_eq!(clipped.chars().count(), 100);
        assert!(clipped.starts_with("… earlier context omitted …"));
    }

    #[test]
    fn only_canonical_content_ids_can_shape_inbox_paths() {
        assert!(is_content_id(&"01abcdef".repeat(8)));
        assert!(!is_content_id("../../outside"));
        assert!(!is_content_id(&"A".repeat(64)));
        assert!(!is_content_id(&"0".repeat(63)));
    }

    #[test]
    fn injected_turn_never_self_approves_or_invents_human_input() {
        assert_eq!(
            bridge_server_response("item/commandExecution/requestApproval").unwrap().unwrap(),
            json!({ "decision": "decline" })
        );
        assert_eq!(
            bridge_server_response("item/permissions/requestApproval").unwrap().unwrap(),
            json!({ "permissions": {} })
        );
        assert_eq!(
            bridge_server_response("item/tool/requestUserInput").unwrap().unwrap(),
            json!({ "answers": {} })
        );
        assert!(bridge_server_response("account/chatgptAuthTokens/refresh").unwrap().is_err());
    }

    #[test]
    fn failed_turns_are_not_treated_as_delivered_peer_work() {
        assert_eq!(incomplete_turn_reason(&json!({ "status": "completed" })), None);
        assert_eq!(
            incomplete_turn_reason(&json!({
                "status": "failed",
                "error": { "message": "model unavailable" }
            }))
            .as_deref(),
            Some("model unavailable")
        );
        assert!(incomplete_turn_reason(&json!({ "status": "interrupted" })).is_some());
    }

    #[tokio::test]
    async fn canonical_key_immediately_joins_same_conversation_and_backlog() {
        let (hub, store) = start_hub().await;
        let alice_cfg = Config::create(hub.clone(), "alice", None).unwrap();
        let bob_cfg = Config::create(hub.clone(), "bob", None).unwrap();
        let mut alice = MeshAgent::connect(&alice_cfg).await.unwrap();
        let mut bob = MeshAgent::connect(&bob_cfg).await.unwrap();

        let opened = enter_conversation(
            &mut alice,
            &Options {
                key: None,
                host: Host::Codex,
                topic: Some("same-conversation".into()),
                resume: None,
                approval: false,
                ttl: None,
                max_uses: None,
            },
            "",
            "Codex",
        )
        .await
        .unwrap();
        let share = opened.share.unwrap();
        let watch = share.watch.clone().expect("the owner mints a viewer code at creation");
        alice
            .send_text(
                Target::Room { room: opened.room.clone() },
                "durable context before bob joins",
            )
            .await
            .unwrap();

        let joined = enter_conversation(
            &mut bob,
            &Options {
                key: Some(format!("{}@{hub}", share.code)),
                host: Host::Codex,
                topic: None,
                resume: None,
                approval: false,
                ttl: None,
                max_uses: None,
            },
            "",
            "Codex",
        )
        .await
        .unwrap();
        assert_eq!(joined.room, opened.room);
        assert!(joined
            .initial
            .iter()
            .any(|message| crate::render_message(message).contains("durable context before bob joins")));
        assert_eq!(alice.roster(&opened.room).await.unwrap().len(), 2);
        assert_eq!(
            store.validate_watch_token(&watch, i64::MAX / 2).unwrap().as_deref(),
            None,
            "an expired viewer code is rejected"
        );
        let watched_room = store
            .validate_watch_token(&watch, 1)
            .unwrap()
            .expect("the fresh viewer code resolves");
        assert_eq!(watched_room, opened.room, "the viewer code is bound to the original conversation");
        assert_eq!(store.roster(&watched_room, 1).unwrap().len(), 2, "the viewer sees both members");

        let second = enter_conversation(
            &mut alice,
            &Options {
                key: None,
                host: Host::Codex,
                topic: Some("same-conversation".into()),
                resume: None,
                approval: false,
                ttl: None,
                max_uses: None,
            },
            "",
            "Codex",
        )
        .await
        .unwrap();
        assert_ne!(second.room, opened.room, "a repeated topic starts a fresh conversation");
        assert!(!second
            .initial
            .iter()
            .any(|message| crate::render_message(message).contains("durable context before bob joins")));
    }

    #[tokio::test]
    async fn catchup_pages_past_the_hub_default_and_retains_only_the_context_tail() {
        let (hub, _) = start_hub().await;
        let alice_cfg = Config::create(hub.clone(), "alice", None).unwrap();
        let bob_cfg = Config::create(hub, "bob", None).unwrap();
        let mut alice = MeshAgent::connect(&alice_cfg).await.unwrap();
        let mut bob = MeshAgent::connect(&bob_cfg).await.unwrap();
        let invitation = alice
            .invite(RoomKind::Channel, Some("long-history".into()), None, None)
            .await
            .unwrap();
        bob.join(&invitation.code).await.unwrap();
        for index in 0..225 {
            alice
                .send_text(
                    Target::Room { room: invitation.room.clone() },
                    &format!("backlog-{index:03} {}", "x".repeat(160)),
                )
                .await
                .unwrap();
        }

        let (initial, _) = bob.pull(&invitation.room, None, None).await.unwrap();
        assert_eq!(initial.len(), 200, "the test must cross the hub's default page");
        let backlog = prepare_backlog(
            &mut bob,
            &invitation.room,
            &initial,
            false,
            Host::Codex,
        )
        .await
        .unwrap();
        let prompt = backlog.prompt.unwrap();
        assert!(prompt.contains("backlog-224"));
        assert!(!prompt.contains("backlog-000"));
        bob.commit_reads_through(&invitation.room, backlog.commit_cursor.unwrap())
            .await
            .unwrap();
        let (remaining, _) = bob.pull(&invitation.room, None, None).await.unwrap();
        assert!(remaining.is_empty(), "paged history must not return later as fresh work");
    }

    #[tokio::test]
    async fn every_visible_host_result_has_the_same_signed_terminal_receipt() {
        let (hub, _) = start_hub().await;
        let alice_cfg = Config::create(hub.clone(), "alice", None).unwrap();
        let bob_cfg = Config::create(hub.clone(), "bob", None).unwrap();
        let mut alice = MeshAgent::connect(&alice_cfg).await.unwrap();
        let mut bob = MeshAgent::connect(&bob_cfg).await.unwrap();
        let opened = enter_conversation(
            &mut alice,
            &Options {
                key: None,
                host: Host::Codex,
                topic: None,
                resume: None,
                approval: false,
                ttl: None,
                max_uses: None,
            },
            "",
            "Codex",
        )
        .await
        .unwrap();
        let share = opened.share.unwrap();
        let joined = enter_conversation(
            &mut bob,
            &Options {
                key: Some(format!("{}@{hub}", share.code)),
                host: Host::Opencode,
                topic: None,
                resume: None,
                approval: false,
                ttl: None,
                max_uses: None,
            },
            "",
            "OpenCode",
        )
        .await
        .unwrap();
        bob.commit_reads(&joined.room).await.unwrap();

        let (request_id, _, _) = bob
            .send_text(Target::Room { room: joined.room.clone() }, "review this")
            .await
            .unwrap();
        let (messages, _) = alice.pull(&opened.room, None, Some(100)).await.unwrap();
        let request = messages
            .into_iter()
            .find(|message| message.id == request_id)
            .expect("the peer request is durable");
        send_peer_result(
            &mut alice,
            &opened.room,
            &request,
            "Reviewed.\nPARLER_HANDOFF {\"to\":\"reviewer\",\"next\":\"verify\"}",
        )
        .await
        .unwrap();
        alice.commit_reads(&opened.room).await.unwrap();

        let (messages, _) = bob.pull(&joined.room, None, Some(100)).await.unwrap();
        let result = messages
            .iter()
            .find(|message| message.reply_to.as_deref() == Some(request_id.as_str()))
            .expect("the result replies to the durable request");
        assert_eq!(verify_message(&result.from.id, &result.parts, result.reply_to.as_deref()), SigStatus::Valid);
        assert!(result.parts.iter().filter_map(TaskRef::from_part).any(|task| {
            task.status == TaskStatus::Done && task.task.as_deref() == Some(request_id.as_str())
        }));
        assert!(result
            .parts
            .iter()
            .filter_map(HandoffRef::from_part)
            .any(|handoff| handoff.to.as_deref() == Some("reviewer")));
    }
}
