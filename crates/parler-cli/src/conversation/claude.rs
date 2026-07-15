use super::*;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::time::{Instant, SystemTime};
use tokio::io::AsyncReadExt;

const MAX_HOOK_INPUT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TRANSCRIPT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LOCAL_PROMPTS: usize = 32;
const HOOK_POLL_SECS: u64 = 1;
const HOOK_LIFETIME_SECS: u64 = 24 * 60 * 60;
const LOCK_OWNER_GRACE: Duration = Duration::from_secs(1);
const WATCH_LOCK_WAIT: Duration = Duration::from_secs(15);
const END_WATCH_WAIT: Duration = Duration::from_secs(5);
const END_STATE_LOCK_WAIT: Duration = Duration::from_secs(2);

pub(super) async fn run(context: AdapterContext) -> Result<()> {
    let AdapterContext {
        options,
        identity,
        cwd,
        hub_override: _,
        mut sender,
    } = context;
    let transcript = load_resume_transcript(&cwd, options.resume.as_deref()).await;
    let entry = enter_conversation(
        &mut sender,
        &options,
        &transcript,
        Host::Claude.display(),
    )
    .await?;
    let ConversationEntry { room, initial, created, share } = entry;
    crate::save_active_session(&room)?;

    let backlog = prepare_backlog(&mut sender, &room, &initial, created, Host::Claude).await?;
    announce_arrival(&mut sender, &room, created).await?;

    write_bootstrap(&Bootstrap {
        prompt: backlog.prompt,
        commit_cursor: backlog.commit_cursor,
    })?;
    let mut tui = launch_tui(&cwd, options.resume.as_deref(), &identity, &sender)?;
    wait_for_hook_start(&mut tui).await?;

    print_connected(&sender, Host::Claude, share);

    let status = tui.wait().await?;
    let _ = std::fs::remove_file(bootstrap_path());
    if status.success() {
        Ok(())
    } else {
        bail!("interactive Claude Code exited with {status}")
    }
}

fn launch_tui(
    cwd: &Path,
    resume: Option<&str>,
    identity: &TuiIdentity,
    agent: &MeshAgent,
) -> Result<Child> {
    let executable = std::env::current_exe().context("could not locate the running parler binary")?;
    let mcp = claude_mcp_config(&executable, identity, agent);
    let settings = claude_hook_settings(&executable);
    let mut command = Command::new("claude");
    if let Some(value) = resume {
        if value.eq_ignore_ascii_case("last") {
            command.arg("--continue");
        } else {
            command.arg("--resume").arg(value);
        }
    }
    command
        .arg("--mcp-config")
        .arg(serde_json::to_string(&mcp)?)
        .arg("--settings")
        .arg(serde_json::to_string(&settings)?);
    configure_host_process(&mut command, identity, agent);
    command
        .current_dir(cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    command.spawn().context("failed to launch the interactive Claude Code TUI")
}

fn claude_mcp_config(executable: &Path, identity: &TuiIdentity, agent: &MeshAgent) -> Value {
    let env = managed_host_environment(identity, agent);
    json!({
        "mcpServers": {
            "parler": {
                "type": "stdio",
                "command": executable,
                "args": ["mcp"],
                "env": env,
            }
        }
    })
}

fn claude_hook_settings(executable: &Path) -> Value {
    let command = executable.to_string_lossy();
    let waiter = || {
        json!({
            "type": "command",
            "command": command,
            "args": ["hook", "conversation-wake"],
            "asyncRewake": true,
            "timeout": HOOK_LIFETIME_SECS,
            "statusMessage": "Waiting for signed Parler peer turns",
        })
    };
    json!({
        "hooks": {
            "SessionStart": [{ "hooks": [waiter()] }],
            "UserPromptSubmit": [{
                "hooks": [{
                    "type": "command",
                    "command": command,
                    "args": ["hook", "conversation-prompt"],
                    "timeout": 10,
                }]
            }],
            "Stop": [{ "hooks": [waiter()] }],
            "SessionEnd": [{
                "hooks": [{
                    "type": "command",
                    "command": command,
                    "args": ["hook", "conversation-end"],
                    "timeout": 10,
                }]
            }],
        }
    })
}

async fn wait_for_hook_start(tui: &mut Child) -> Result<()> {
    let deadline = tokio::time::Instant::now() + APP_SERVER_START_TIMEOUT;
    loop {
        if !bootstrap_path().exists() {
            return Ok(());
        }
        if let Some(status) = tui.try_wait()? {
            bail!("interactive Claude Code exited before its conversation hooks started ({status})");
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "Claude Code did not start the conversation hooks; update Claude Code, confirm managed policy permits session hooks, and retry"
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct HookState {
    generation: u64,
    prompts: Vec<String>,
    pending: Option<Pending>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Pending {
    Bootstrap { cursor: i64 },
    Message { id: String },
}

#[derive(Debug, Deserialize, Serialize)]
struct Bootstrap {
    prompt: Option<String>,
    commit_cursor: Option<i64>,
}

#[derive(Debug)]
struct HookInput {
    session_id: String,
    event: String,
    source: Option<String>,
    prompt: Option<String>,
    last_assistant_message: Option<String>,
}

pub(crate) async fn prompt_hook() -> Result<()> {
    let input = read_hook_input().await?;
    let Some(prompt) = input.prompt.as_deref().map(str::trim).filter(|prompt| !prompt.is_empty()) else {
        return Ok(());
    };
    update_state(&input.session_id, |state| {
        state.generation = state.generation.wrapping_add(1);
        state.prompts.push(clip(prompt, MAX_CONTEXT_CHARS));
        if state.prompts.len() > MAX_LOCAL_PROMPTS {
            state.prompts.drain(..state.prompts.len() - MAX_LOCAL_PROMPTS);
        }
    })?;
    Ok(())
}

pub(crate) async fn end_hook() -> Result<()> {
    let input = read_hook_input().await?;
    let ending_generation = modify_state(&input.session_id, |state| {
        state.generation = state.generation.wrapping_add(1);
        state.generation
    })?;
    // The generation change asks the long-lived waiter to release its lock. Once it does, remove
    // only this ended generation; a fast resume may already have created newer state.
    let Some(_watch) = acquire_watch_lock_for(&input.session_id, END_WATCH_WAIT).await? else {
        return Ok(())
    };
    let Some(_state_lock) = acquire_file_lock(&state_lock_path(&input.session_id), END_STATE_LOCK_WAIT)? else {
        return Ok(())
    };
    if read_state(&input.session_id)?.generation == ending_generation {
        match std::fs::remove_file(state_path(&input.session_id)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("could not remove ended Claude conversation state"),
        }
    }
    Ok(())
}

/// Return a reason for Claude Code's `asyncRewake` hook. The CLI boundary writes this to stderr and
/// exits 2, which is the documented signal that wakes the same idle visible session.
pub(crate) async fn wake_hook() -> Result<Option<String>> {
    let input = read_hook_input().await?;
    let should_watch = modify_state(&input.session_id, |state| prepare_wake_state(&input, state))?;
    if !should_watch {
        return Ok(None);
    }
    let Some(_watch) = acquire_watch_lock(&input.session_id).await? else {
        return Ok(None);
    };
    let Some(room) = crate::load_active_session() else {
        return Ok(None);
    };
    let mut agent = crate::connect().await?;

    if input.event == "SessionStart" {
        if let Some(bootstrap) = take_bootstrap()? {
            if let Some(cursor) = bootstrap.commit_cursor {
                update_state(&input.session_id, |state| {
                    state.pending = Some(Pending::Bootstrap { cursor });
                })?;
            }
            if let Some(prompt) = bootstrap.prompt {
                let _ = agent
                    .presence("working", Some(format!("live conversation '{room}'")))
                    .await;
                return Ok(Some(prompt));
            }
        }
    }

    if input.event == "Stop" {
        complete_turn(
            &mut agent,
            &room,
            &input.session_id,
            input.last_assistant_message.as_deref().unwrap_or_default(),
        )
        .await?;
    }

    let generation = read_state(&input.session_id)?.generation;
    let _ = agent
        .presence("waiting", Some(format!("live conversation '{room}'")))
        .await;
    let mut heartbeat = tokio::time::interval(PRESENCE_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    loop {
        if read_state(&input.session_id)?.generation != generation {
            return Ok(None);
        }
        let (messages, _, _) = agent.pull_wait(&room, Some(1), HOOK_POLL_SECS).await?;
        for message in messages {
            if read_state(&input.session_id)?.generation != generation {
                agent.defer_reads(&room);
                return Ok(None);
            }
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
            let prompt = live_turn_prompt(&mut agent, &room, &message, Host::Claude.display()).await;
            let accepted = modify_state(&input.session_id, |state| {
                if state.generation != generation || state.pending.is_some() {
                    false
                } else {
                    state.pending = Some(Pending::Message { id: message.id.clone() });
                    true
                }
            })?;
            if !accepted {
                agent.defer_reads(&room);
                return Ok(None);
            }
            let _ = agent
                .presence("working", Some(format!("live conversation '{room}'")))
                .await;
            return Ok(Some(clip(&prompt, CLAUDE_WAKE_PROMPT_CHARS)));
        }
        tokio::select! {
            _ = heartbeat.tick() => {
                let _ = agent.presence("waiting", Some(format!("live conversation '{room}'"))).await;
            }
            _ = tokio::time::sleep(Duration::from_millis(1)) => {}
        }
    }
}

/// Start/resume creates one long-lived idle watcher. SessionStart also fires after `/clear` and
/// compaction, while a model turn may already own a durable peer message; those events must not
/// reset delivery state or start a competing watcher. Every Stop supersedes the prior idle watcher
/// before it completes and mirrors the just-finished visible turn.
fn prepare_wake_state(input: &HookInput, state: &mut HookState) -> bool {
    match input.event.as_str() {
        "SessionStart" if matches!(input.source.as_deref(), Some("clear" | "compact")) => false,
        "SessionStart" => {
            state.generation = state.generation.wrapping_add(1);
            state.prompts.clear();
            state.pending = None;
            true
        }
        "Stop" => {
            state.generation = state.generation.wrapping_add(1);
            true
        }
        _ => false,
    }
}

async fn complete_turn(
    agent: &mut MeshAgent,
    room: &str,
    session_id: &str,
    answer: &str,
) -> Result<()> {
    let snapshot = read_state(session_id)?;
    match &snapshot.pending {
        Some(Pending::Bootstrap { cursor }) => {
            agent.commit_reads_through(room, *cursor).await?;
            if !snapshot.prompts.is_empty() {
                let text = local_turn(&snapshot.prompts, answer);
                agent
                    .send_text(Target::Room { room: room.to_string() }, &clip(&text, MAX_REPLY_CHARS))
                    .await?;
            }
        }
        Some(Pending::Message { id }) => {
            let message = pull_pending_message(agent, room, id).await?;
            let text = if snapshot.prompts.is_empty() {
                answer.to_string()
            } else {
                local_turn(&snapshot.prompts, answer)
            };
            send_injected_result(agent, room, &message, &text).await?;
            agent.commit_reads(room).await?;
        }
        None if !snapshot.prompts.is_empty() => {
            let text = local_turn(&snapshot.prompts, answer);
            if !text.trim().is_empty() {
                agent
                    .send_text(Target::Room { room: room.to_string() }, &clip(&text, MAX_REPLY_CHARS))
                    .await?;
            }
        }
        None => {}
    }
    modify_state(session_id, |state| {
        if state.pending == snapshot.pending {
            state.pending = None;
        }
        if state.prompts.starts_with(&snapshot.prompts) {
            state.prompts.drain(..snapshot.prompts.len());
        }
    })?;
    Ok(())
}

async fn pull_pending_message(agent: &mut MeshAgent, room: &str, expected_id: &str) -> Result<StoredMessage> {
    loop {
        let (messages, _) = agent.pull(room, None, Some(1)).await?;
        let Some(message) = messages.into_iter().next() else {
            bail!("Claude Code completed peer work, but the durable request is not available")
        };
        if message.id == expected_id {
            return Ok(message);
        }
        if is_actionable(
            &message,
            room,
            &agent.id,
            &agent.name,
            agent.role.as_deref(),
        ) {
            agent.defer_reads(room);
            bail!("Claude Code's pending peer request no longer matches the durable room cursor")
        }
        agent.commit_reads(room).await?;
    }
}

async fn send_injected_result(
    agent: &mut MeshAgent,
    room: &str,
    incoming: &StoredMessage,
    answer: &str,
) -> Result<()> {
    let text = answer.trim();
    if text.is_empty() {
        return Ok(());
    }
    send_peer_result(agent, room, incoming, text).await
}

fn local_turn(prompts: &[String], answer: &str) -> String {
    let prompt = prompts
        .iter()
        .map(|prompt| prompt.trim())
        .filter(|prompt| !prompt.is_empty())
        .collect::<Vec<_>>()
        .join("\n\nUser: ");
    match (prompt.is_empty(), answer.trim().is_empty()) {
        (true, _) => answer.trim().to_string(),
        (false, true) => format!("User: {prompt}"),
        (false, false) => format!("User: {prompt}\n\nAgent: {}", answer.trim()),
    }
}

async fn read_hook_input() -> Result<HookInput> {
    let mut bytes = Vec::new();
    tokio::io::stdin()
        .take(MAX_HOOK_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > MAX_HOOK_INPUT_BYTES {
        bail!("Claude Code hook input exceeded 2 MiB");
    }
    let value: Value = serde_json::from_slice(&bytes).context("Claude Code hook input is not valid JSON")?;
    let session_id = value["session_id"]
        .as_str()
        .ok_or_else(|| anyhow!("Claude Code hook input is missing session_id"))?;
    validate_session_token(session_id)?;
    Ok(HookInput {
        session_id: session_id.to_string(),
        event: value["hook_event_name"].as_str().unwrap_or_default().to_string(),
        source: value["source"].as_str().map(str::to_string),
        prompt: value["prompt"].as_str().map(str::to_string),
        last_assistant_message: value["last_assistant_message"].as_str().map(str::to_string),
    })
}

fn hook_dir() -> PathBuf {
    parler_connector::home_dir().join("conversation-hooks")
}

fn state_path(session_id: &str) -> PathBuf {
    hook_dir().join(format!("{session_id}.json"))
}

fn state_lock_path(session_id: &str) -> PathBuf {
    hook_dir().join(format!("{session_id}.state.lock"))
}

fn watch_lock_path(session_id: &str) -> PathBuf {
    hook_dir().join(format!("{session_id}.watch.lock"))
}

fn bootstrap_path() -> PathBuf {
    hook_dir().join("bootstrap.json")
}

fn validate_session_token(value: &str) -> Result<()> {
    if !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Ok(());
    }
    bail!("Claude Code supplied an invalid hook session id")
}

fn read_state(session_id: &str) -> Result<HookState> {
    let path = state_path(session_id);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("could not parse Claude conversation state at {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HookState::default()),
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}

fn update_state<F>(session_id: &str, update: F) -> Result<()>
where
    F: FnOnce(&mut HookState),
{
    modify_state(session_id, |state| {
        update(state);
    })
}

fn modify_state<T, F>(session_id: &str, update: F) -> Result<T>
where
    F: FnOnce(&mut HookState) -> T,
{
    let _lock = acquire_file_lock(&state_lock_path(session_id), Duration::from_secs(5))?
        .ok_or_else(|| anyhow!("timed out locking Claude conversation state"))?;
    let mut state = read_state(session_id)?;
    let output = update(&mut state);
    let bytes = serde_json::to_vec(&state)?;
    parler_auth::write_private_file(&state_path(session_id), &bytes)?;
    Ok(output)
}

fn write_bootstrap(bootstrap: &Bootstrap) -> Result<()> {
    std::fs::create_dir_all(hook_dir())?;
    parler_auth::write_private_file(&bootstrap_path(), &serde_json::to_vec(bootstrap)?)?;
    Ok(())
}

fn take_bootstrap() -> Result<Option<Bootstrap>> {
    let path = bootstrap_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("could not read {}", path.display())),
    };
    let bootstrap = serde_json::from_slice(&bytes)
        .with_context(|| format!("could not parse {}", path.display()))?;
    std::fs::remove_file(&path).with_context(|| format!("could not consume {}", path.display()))?;
    Ok(Some(bootstrap))
}

struct FileLock {
    path: PathBuf,
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn try_file_lock(path: &Path) -> Result<Option<FileLock>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            writeln!(file, "{}", std::process::id())?;
            Ok(Some(FileLock { path: path.to_path_buf() }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if lock_is_stale(path) {
                let _ = std::fs::remove_file(path);
            }
            Ok(None)
        }
        Err(error) => Err(error).with_context(|| format!("could not create {}", path.display())),
    }
}

fn acquire_file_lock(path: &Path, timeout: Duration) -> Result<Option<FileLock>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(lock) = try_file_lock(path)? {
            return Ok(Some(lock));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

async fn acquire_watch_lock(session_id: &str) -> Result<Option<FileLock>> {
    acquire_watch_lock_for(session_id, WATCH_LOCK_WAIT).await
}

async fn acquire_watch_lock_for(session_id: &str, timeout: Duration) -> Result<Option<FileLock>> {
    let path = watch_lock_path(session_id);
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(lock) = try_file_lock(&path)? {
            return Ok(Some(lock));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn lock_is_stale(path: &Path) -> bool {
    let age = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .unwrap_or_default();
    if age <= LOCK_OWNER_GRACE {
        return false;
    }
    #[cfg(unix)]
    {
        let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|value| value.trim().parse::<i32>().ok())
        else {
            return true;
        };
        // SAFETY: signal 0 does not modify the target process; it only probes whether this valid,
        // parsed PID still exists. EPERM also means the owner exists. We never replace a live
        // owner's path, so its Drop cannot accidentally unlink a newer owner's lock.
        let status = unsafe { libc::kill(pid, 0) };
        status != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        age > Duration::from_secs(HOOK_LIFETIME_SECS + 300)
    }
}

async fn load_resume_transcript(cwd: &Path, resume: Option<&str>) -> String {
    let Some(resume) = resume.map(str::to_string) else { return String::new() };
    let cwd = cwd.to_path_buf();
    tokio::task::spawn_blocking(move || find_resume_transcript(&cwd, &resume))
        .await
        .ok()
        .flatten()
        .and_then(|path| read_transcript_tail(&path).ok())
        .map(|contents| transcript_from_jsonl(&contents))
        .unwrap_or_default()
}

fn find_resume_transcript(cwd: &Path, resume: &str) -> Option<PathBuf> {
    if !resume.eq_ignore_ascii_case("last") && validate_session_token(resume).is_err() {
        return None;
    }
    let root = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))?;
    let project = cwd
        .to_string_lossy()
        .chars()
        .map(|character| if character.is_ascii_alphanumeric() { character } else { '-' })
        .collect::<String>();
    let dir = root.join("projects").join(project);
    if !resume.eq_ignore_ascii_case("last") {
        let exact = dir.join(format!("{resume}.jsonl"));
        if exact.is_file() {
            return Some(exact);
        }
        // Claude Code can resolve named sessions itself, but choosing an unrelated newest JSONL
        // here would expose the wrong conversation. Only `last` may use modification ordering.
        return None;
    }
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl"))
        .max_by_key(|entry| {
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        })
        .map(|entry| entry.path())
}

fn read_transcript_tail(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let offset = len.saturating_sub(MAX_TRANSCRIPT_BYTES);
    if offset > 0 {
        file.seek(SeekFrom::Start(offset))?;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    // The bounded seek can land inside a UTF-8 code point. That damage is confined to the first,
    // already-partial JSONL record, which is discarded below.
    let mut contents = String::from_utf8_lossy(&bytes).into_owned();
    if offset > 0 {
        contents = contents.split_once('\n').map(|(_, tail)| tail.to_string()).unwrap_or_default();
    }
    Ok(contents)
}

fn transcript_from_jsonl(contents: &str) -> String {
    let mut lines = Vec::new();
    for value in contents.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
        if value["isSidechain"].as_bool().unwrap_or(false) || value["isMeta"].as_bool().unwrap_or(false) {
            continue;
        }
        let role = value["message"]["role"].as_str().or_else(|| value["type"].as_str());
        let text = message_text(&value["message"]["content"]);
        match role {
            Some("user") => {
                if let Some((speaker, text)) = visible_user_transcript(&text) {
                    lines.push(format!("{speaker}: {text}"));
                }
            }
            Some("assistant") if !text.is_empty() => lines.push(format!("Agent: {text}")),
            _ => {}
        }
    }
    clip_tail(&lines.join("\n\n"), MAX_CONTEXT_CHARS)
}

fn message_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.trim().to_string(),
        Value::Array(parts) => parts
            .iter()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str().map(str::trim))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parler_connector::Config;

    fn identity() -> TuiIdentity {
        TuiIdentity { base_home: PathBuf::from("/tmp/parler"), terminal_session: "tty-claude".into() }
    }

    fn agent() -> MeshAgent {
        let config = Config::create("ws://127.0.0.1:1", "claude-agent", None).unwrap();
        MeshAgent::with_transport(
            Box::new(NoTransport),
            config.identity.id,
            config.name,
            config.role,
            config.hub_url,
        )
    }

    struct NoTransport;

    #[async_trait::async_trait]
    impl parler_connector::MeshTransport for NoTransport {
        async fn request(&mut self, _frame: parler_protocol::ClientFrame) -> Result<parler_protocol::ServerFrame> {
            bail!("not used")
        }
    }

    #[test]
    fn settings_use_native_async_rewake_without_permission_hooks() {
        let settings = claude_hook_settings(Path::new("/usr/local/bin/parler"));
        let start = &settings["hooks"]["SessionStart"][0]["hooks"][0];
        let stop = &settings["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(start["asyncRewake"], true);
        assert_eq!(stop["asyncRewake"], true);
        assert_eq!(stop["args"], json!(["hook", "conversation-wake"]));
        assert!(settings["hooks"].get("PermissionRequest").is_none());
        assert!(settings["hooks"].get("PreToolUse").is_none());
    }

    #[test]
    fn mcp_config_carries_the_exact_terminal_identity() {
        let config = claude_mcp_config(Path::new("/usr/local/bin/parler"), &identity(), &agent());
        let parler = &config["mcpServers"]["parler"];
        assert_eq!(parler["command"], "/usr/local/bin/parler");
        assert_eq!(parler["args"], json!(["mcp"]));
        assert_eq!(parler["env"]["PARLER_AGENT_SESSION"], "tty-claude");
        assert_eq!(parler["env"]["PARLER_HUB"], "ws://127.0.0.1:1");
    }

    #[test]
    fn jsonl_parser_exports_only_visible_user_and_assistant_text() {
        let transcript = concat!(
            r#"{"type":"user","message":{"role":"user","content":"Please audit."}}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"secret"},{"type":"text","text":"Audit complete."}]}}"#, "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"private output"}]}}"#, "\n",
            r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"subagent"}]}}"#,
        );
        assert_eq!(
            transcript_from_jsonl(transcript),
            "User: Please audit.\n\nAgent: Audit complete."
        );
    }

    #[test]
    fn transcript_tail_discards_a_partial_multibyte_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut bytes = concat!(
            "🙂\n",
            r#"{"type":"user","message":{"role":"user","content":"Still visible."}}"#,
            "\n",
        )
        .as_bytes()
        .to_vec();
        bytes.resize(MAX_TRANSCRIPT_BYTES as usize + 1, b' ');
        std::fs::write(&path, bytes).unwrap();

        let tail = read_transcript_tail(&path).unwrap();
        assert_eq!(transcript_from_jsonl(&tail), "User: Still visible.");
    }

    #[test]
    fn hook_session_id_cannot_escape_the_state_directory() {
        assert!(validate_session_token("2a4f_session-1").is_ok());
        assert!(validate_session_token("../../settings").is_err());
        assert!(validate_session_token("").is_err());
        assert!(find_resume_transcript(Path::new("/tmp"), "../../outside").is_none());
    }

    #[test]
    fn local_turn_keeps_all_steering_prompts_with_one_answer() {
        assert_eq!(
            local_turn(&["First request".into(), "Second detail".into()], "Done"),
            "User: First request\n\nUser: Second detail\n\nAgent: Done"
        );
    }

    #[test]
    fn lifecycle_state_supersedes_waiters_without_losing_an_active_delivery() {
        let mut state = HookState {
            generation: 4,
            prompts: vec!["local detail".into()],
            pending: Some(Pending::Message { id: "peer-1".into() }),
        };
        let compact = HookInput {
            session_id: "session".into(),
            event: "SessionStart".into(),
            source: Some("compact".into()),
            prompt: None,
            last_assistant_message: None,
        };
        assert!(!prepare_wake_state(&compact, &mut state));
        assert_eq!(state.pending, Some(Pending::Message { id: "peer-1".into() }));

        let stop = HookInput { event: "Stop".into(), source: None, ..compact };
        assert!(prepare_wake_state(&stop, &mut state));
        assert_eq!(state.generation, 5);
        assert_eq!(state.pending, Some(Pending::Message { id: "peer-1".into() }));

        let startup = HookInput {
            event: "SessionStart".into(),
            source: Some("resume".into()),
            ..stop
        };
        assert!(prepare_wake_state(&startup, &mut state));
        assert!(state.pending.is_none());
        assert!(state.prompts.is_empty());
    }
}
