//! Autonomous room/service worker.
//!
//! A Parler message is durable, but an LLM host is inert between turns. `parler work` owns that
//! missing activation boundary: long-poll one signed request, run a bounded headless agent turn, and
//! post a signed lifecycle/result message. It deliberately ignores lifecycle-only messages so two
//! workers cannot turn acknowledgements into an infinite conversation.

use anyhow::{bail, Result};
use parler_connector::{verify_message, MeshAgent, SigStatus};
use parler_protocol::{HandoffRef, Part, StoredMessage, Target, TaskRef, TaskStatus};
use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::time::{Duration, Instant};

const WAIT_CHUNK_SECS: u64 = 25;
const MAX_RESULT_CHARS: usize = 16_000;
const MAX_ERROR_CHARS: usize = 1_000;
const HANDOFF_MARKER: &str = "PARLER_HANDOFF ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkSource {
    Room,
    Service,
}

pub struct WorkOptions {
    pub source: WorkSource,
    pub all_messages: bool,
    pub allow_from: HashSet<String>,
    pub max_per_hour: u32,
    pub timeout: Duration,
    pub once: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WorkReport {
    pub handled: usize,
    pub last_response_room: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunnerKind {
    Codex,
    Claude,
}

pub struct ProcessRunner {
    kind: RunnerKind,
}

impl ProcessRunner {
    pub fn parse(value: &str) -> Result<ProcessRunner> {
        let kind = match value {
            "codex" => RunnerKind::Codex,
            "claude" => RunnerKind::Claude,
            _ => bail!("unknown runner '{value}' — use codex or claude"),
        };
        Ok(ProcessRunner { kind })
    }
}

pub trait Runner: Send + Sync {
    fn name(&self) -> &'static str;

    fn run<'a>(
        &'a self,
        prompt: &'a str,
        cwd: &'a Path,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<String, RunnerFailure>> + Send + 'a>>;
}

impl Runner for ProcessRunner {
    fn name(&self) -> &'static str {
        match self.kind {
            RunnerKind::Codex => "codex",
            RunnerKind::Claude => "claude",
        }
    }

    fn run<'a>(
        &'a self,
        prompt: &'a str,
        cwd: &'a Path,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<String, RunnerFailure>> + Send + 'a>> {
        Box::pin(run_process(self.kind, prompt, cwd, timeout))
    }
}

pub enum RunnerFailure {
    NotInstalled(&'static str),
    NotAuthed(&'static str),
    TimedOut { runner: &'static str, secs: u64 },
    Failed { runner: &'static str, detail: String },
}

impl RunnerFailure {
    pub fn remedy(&self) -> String {
        match self {
            RunnerFailure::NotInstalled(runner) => {
                format!("'{runner}' is not installed (not on PATH); install it, then restart the worker")
            }
            RunnerFailure::NotAuthed(runner) => {
                let login = if *runner == "claude" { "claude auth login" } else { "codex login" };
                format!("'{runner}' is not logged in; run `{login}`, then restart the worker")
            }
            RunnerFailure::TimedOut { runner, secs } => {
                format!("{runner} exceeded the {secs}s turn timeout")
            }
            RunnerFailure::Failed { runner, detail } => format!("{runner} failed: {detail}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum IgnoreReason {
    OwnMessage,
    Untrusted,
    Disallowed,
    NotAddressed,
    Lifecycle,
    NonActionable,
}

struct WorkItem {
    task_id: String,
    sender_id: String,
    sender_name: String,
    instruction: String,
    summary: Option<String>,
    context: Option<String>,
    bundle: Option<String>,
    had_task_ref: bool,
}

enum Selection {
    Work(WorkItem),
    Ignore(IgnoreReason),
}

/// Run until interrupted, or until one actionable request when `options.once` is set.
pub async fn run(
    agent: &mut MeshAgent,
    room: &str,
    options: &WorkOptions,
    runner: &dyn Runner,
) -> Result<WorkReport> {
    let activity = format!("autonomous {} worker in '{room}'", runner.name());
    agent.presence("waiting", Some(activity.clone())).await?;
    eprintln!(
        "🤖 {} worker watching '{}' — {} (Ctrl-C to stop)",
        runner.name(),
        room,
        match options.source {
            WorkSource::Room if options.all_messages => "all signed peer messages",
            WorkSource::Room => "valid signed handoffs",
            WorkSource::Service => "service requests",
        }
    );

    let cwd = std::env::current_dir()?;
    let mut rate = RateGate::new(options.max_per_hour);
    let mut report = WorkReport::default();
    loop {
        let (messages, _, _) = agent.pull_wait(room, Some(1), WAIT_CHUNK_SECS).await?;
        for message in messages {
            let trusted = verify_message(&message.from.id, &message.parts, message.reply_to.as_deref());
            let selection = select_message(
                &message,
                trusted,
                &agent.id,
                &agent.name,
                agent.role.as_deref(),
                options,
            );
            let item = match selection {
                Selection::Work(item) => item,
                Selection::Ignore(reason) => {
                    if matches!(reason, IgnoreReason::Untrusted | IgnoreReason::Disallowed) {
                        eprintln!(
                            "⚠ ignored message {} from {} ({reason:?})",
                            message.id, message.from.name
                        );
                    }
                    agent.commit_reads(room).await?;
                    continue;
                }
            };

            if !rate.admit(Instant::now()) {
                let note = format!(
                    "worker rate limit reached ({} turn(s)/hour); retry later",
                    options.max_per_hour
                );
                let response_room = post_terminal(
                    agent,
                    room,
                    options.source,
                    &item,
                    TerminalResult {
                        status: TaskStatus::Failed,
                        body: &note,
                        elapsed_ms: 0,
                        continuation: None,
                    },
                )
                .await?;
                agent.commit_reads(room).await?;
                report.handled += 1;
                report.last_response_room = Some(response_room);
                if options.once {
                    return Ok(report);
                }
                continue;
            }

            post_working(agent, room, &item, runner.name()).await?;
            agent.presence("working", Some(format!("{} task {}", runner.name(), item.task_id))).await?;
            let prompt = build_prompt(&item, room, options.source);
            let started = Instant::now();
            let outcome = runner.run(&prompt, &cwd, options.timeout).await;
            let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            let (status, body, continuation) = match outcome {
                Ok(output) => {
                    let result = parse_runner_result(&output);
                    (
                        TaskStatus::Done,
                        clip(&result.body, MAX_RESULT_CHARS),
                        result.continuation,
                    )
                }
                Err(error) => (
                    TaskStatus::Failed,
                    clip(&error.remedy(), MAX_ERROR_CHARS),
                    None,
                ),
            };
            let response_room = post_terminal(
                agent,
                room,
                options.source,
                &item,
                TerminalResult {
                    status,
                    body: &body,
                    elapsed_ms,
                    continuation: continuation.as_ref(),
                },
            )
            .await?;
            // Pull uses deferred acks. Commit only after the terminal result lands: a crash before
            // this point redelivers the request instead of silently losing unreported work.
            agent.commit_reads(room).await?;
            let _ = agent.presence("waiting", Some(activity.clone())).await;
            report.handled += 1;
            report.last_response_room = Some(response_room);
            if options.once {
                return Ok(report);
            }
        }
    }
}

fn select_message(
    message: &StoredMessage,
    trusted: SigStatus,
    my_id: &str,
    my_name: &str,
    my_role: Option<&str>,
    options: &WorkOptions,
) -> Selection {
    if message.from.id == my_id {
        return Selection::Ignore(IgnoreReason::OwnMessage);
    }
    if trusted != SigStatus::Valid {
        return Selection::Ignore(IgnoreReason::Untrusted);
    }
    if !options.allow_from.is_empty() && !options.allow_from.contains(&message.from.id) {
        return Selection::Ignore(IgnoreReason::Disallowed);
    }

    let handoffs: Vec<HandoffRef> = message.parts.iter().filter_map(HandoffRef::from_part).collect();
    if !handoffs.is_empty() {
        let addressed = handoffs.into_iter().find(|handoff| {
            handoff.is_for(my_name, my_role)
                || handoff.to.as_deref().is_some_and(|to| to.eq_ignore_ascii_case(my_id))
        });
        return match addressed {
            Some(handoff) if !handoff.next.trim().is_empty() => Selection::Work(WorkItem {
                task_id: message.id.clone(),
                sender_id: message.from.id.clone(),
                sender_name: message.from.name.clone(),
                instruction: handoff.next,
                summary: handoff.summary,
                context: message_text(message),
                bundle: handoff.bundle,
                had_task_ref: message.parts.iter().any(|part| TaskRef::from_part(part).is_some()),
            }),
            Some(_) => Selection::Ignore(IgnoreReason::NonActionable),
            None => Selection::Ignore(IgnoreReason::NotAddressed),
        };
    }

    // Lifecycle/result messages are observations, not fresh work. This boundary prevents two
    // `--all-messages` workers from executing each other's acknowledgements forever.
    if message.parts.iter().any(|part| TaskRef::from_part(part).is_some()) {
        return Selection::Ignore(IgnoreReason::Lifecycle);
    }
    // `mentions` are normalized by the hub and deliberately excluded from the author's signature,
    // so they cannot authorize a workspace-writing turn. Room text requires the explicit
    // `--all-messages` opt-in; default room work uses the signed HandoffRef above.
    let accepts_plain = options.source == WorkSource::Service || options.all_messages;
    let Some(text) = message_text(message).filter(|text| !text.trim().is_empty()) else {
        return Selection::Ignore(IgnoreReason::NonActionable);
    };
    if !accepts_plain {
        return Selection::Ignore(IgnoreReason::NotAddressed);
    }
    Selection::Work(WorkItem {
        task_id: message.id.clone(),
        sender_id: message.from.id.clone(),
        sender_name: message.from.name.clone(),
        instruction: text,
        summary: None,
        context: None,
        bundle: None,
        had_task_ref: false,
    })
}

fn message_text(message: &StoredMessage) -> Option<String> {
    let text = message
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.trim()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn build_prompt(item: &WorkItem, room: &str, source: WorkSource) -> String {
    let mut prompt = format!(
        "You are the autonomous worker for Parler room '{room}'. A cryptographically signed peer \
         assigned you the task below. Execute the task now in the current workspace; do not merely \
         summarize the request or report that you received it. Follow the repository's AGENTS.md \
         and local engineering rules, make the necessary changes or investigation, and verify the \
         result in proportion to risk. Do not call Parler or wait for another room message: the \
         worker daemon will post your final response and any requested continuation. If genuinely \
         blocked, state the exact missing input in your final response.\n\nSender: {} ({})\nTask id: {}\n",
        item.sender_name, item.sender_id, item.task_id
    );
    if let Some(summary) = &item.summary {
        prompt.push_str(&format!("Prior state: {summary}\n"));
    }
    if let Some(context) = &item.context {
        prompt.push_str(&format!("Peer context: {context}\n"));
    }
    if let Some(bundle) = &item.bundle {
        prompt.push_str(&format!(
            "Attached bundle id: {bundle} (inspect explicitly if needed; never auto-merge it)\n"
        ));
    }
    if source == WorkSource::Room {
        prompt.push_str(
            "\nIf another room member must continue after you, put exactly one addressed \
             continuation on the final non-empty line (JSON on one line):\n\
             PARLER_HANDOFF {\"to\":\"agent-name-or-role\",\"next\":\"specific next task\",\
             \"summary\":\"what you completed\"}\n\
             Otherwise do not include PARLER_HANDOFF. The daemon validates and posts it; never run \
             Parler yourself.\n",
        );
    }
    prompt.push_str(&format!("\nTASK TO EXECUTE:\n{}", item.instruction));
    prompt
}

struct RunnerResult {
    body: String,
    continuation: Option<HandoffRef>,
}

/// Extract an optional, daemon-owned next turn from the final output line. Invalid or unaddressed
/// markers stay ordinary result text, so a model formatting mistake never silently drops content or
/// broadcasts work to every room member.
fn parse_runner_result(output: &str) -> RunnerResult {
    let output = output.trim();
    let Some((body, final_line)) = output.rsplit_once('\n') else {
        return RunnerResult { body: output.to_string(), continuation: None };
    };
    let Some(payload) = final_line.trim().strip_prefix(HANDOFF_MARKER) else {
        return RunnerResult { body: output.to_string(), continuation: None };
    };
    let Ok(handoff) = serde_json::from_str::<HandoffRef>(payload) else {
        return RunnerResult { body: output.to_string(), continuation: None };
    };
    let addressed = handoff.to.as_deref().is_some_and(|to| !to.trim().is_empty());
    if body.trim().is_empty() || handoff.next.trim().is_empty() || !addressed {
        return RunnerResult { body: output.to_string(), continuation: None };
    }
    RunnerResult { body: body.trim().to_string(), continuation: Some(handoff) }
}

async fn post_working(
    agent: &mut MeshAgent,
    room: &str,
    item: &WorkItem,
    runner: &str,
) -> Result<()> {
    let status = TaskRef {
        status: TaskStatus::Working,
        task: Some(item.task_id.clone()),
        note: Some(format!("{runner} is executing the request")),
        result: None,
        tokens: None,
        elapsed_ms: None,
    };
    agent
        .send(
            Target::Room { room: room.to_string() },
            vec![status.to_part()],
            None,
            Some(item.task_id.clone()),
        )
        .await?;
    Ok(())
}

struct TerminalResult<'a> {
    status: TaskStatus,
    body: &'a str,
    elapsed_ms: u64,
    continuation: Option<&'a HandoffRef>,
}

async fn post_terminal(
    agent: &mut MeshAgent,
    room: &str,
    source: WorkSource,
    item: &WorkItem,
    result: TerminalResult<'_>,
) -> Result<String> {
    let task = TaskRef {
        status: result.status,
        task: Some(item.task_id.clone()),
        note: (result.status == TaskStatus::Failed).then(|| result.body.to_string()),
        result: None,
        tokens: None,
        elapsed_ms: Some(result.elapsed_ms),
    };
    let mut parts = vec![
        Part::text(match result.status {
            TaskStatus::Done => format!("🤖 autonomous worker result:\n\n{}", result.body),
            _ => format!("⚠ autonomous worker could not complete the task: {}", result.body),
        }),
        task.to_part(),
    ];
    // A runner may explicitly route the next turn. Otherwise close the common two-agent loop with
    // one return turn to the original sender. A result-handoff already carries a TaskRef, so it gets
    // no automatic bounce; only an explicit continuation can extend the chain.
    let continuation = (source == WorkSource::Room && result.status == TaskStatus::Done)
        .then(|| {
            result.continuation.cloned().or_else(|| {
                (!item.had_task_ref).then(|| HandoffRef {
                    next: "Use the completed worker result above to continue the original task. Act \
                           on it now; do not merely summarize it."
                        .into(),
                    summary: Some(format!("worker completed task {}", item.task_id)),
                    to: Some(item.sender_name.clone()),
                    bundle: None,
                })
            })
        })
        .flatten();
    if let Some(handoff) = &continuation {
        parts.push(handoff.to_part());
    }
    match source {
        WorkSource::Room => {
            let mut mentions = vec![item.sender_name.clone()];
            if let Some(to) = continuation.as_ref().and_then(|handoff| handoff.to.as_ref()) {
                if !mentions.iter().any(|name| name.eq_ignore_ascii_case(to)) {
                    mentions.push(to.clone());
                }
            }
            let (_, _, response_room) = agent
                .send(
                    Target::Room { room: room.to_string() },
                    parts,
                    Some(mentions),
                    Some(item.task_id.clone()),
                )
                .await?;
            Ok(response_room)
        }
        WorkSource::Service => {
            // Prefer a private result DM. A bare CLI requester may not have registered a directory
            // card, in which case the hub cannot resolve a fresh DM; fall back to the shared service
            // room rather than completing work whose result nobody can retrieve.
            match agent
                .send(
                    Target::Dm { agent: item.sender_id.clone() },
                    parts.clone(),
                    None,
                    None,
                )
                .await
            {
                Ok((_, _, response_room)) => Ok(response_room),
                Err(_) => {
                    let (_, _, response_room) = agent
                        .send(
                            Target::Room { room: room.to_string() },
                            parts,
                            Some(vec![item.sender_name.clone()]),
                            Some(item.task_id.clone()),
                        )
                        .await?;
                    Ok(response_room)
                }
            }
        }
    }
}

struct RateGate {
    max: u32,
    starts: VecDeque<Instant>,
}

impl RateGate {
    fn new(max: u32) -> RateGate {
        RateGate { max, starts: VecDeque::new() }
    }

    fn admit(&mut self, now: Instant) -> bool {
        if self.max == 0 {
            return true;
        }
        while self.starts.front().is_some_and(|started| {
            now.saturating_duration_since(*started) >= Duration::from_secs(3600)
        }) {
            self.starts.pop_front();
        }
        if self.starts.len() >= self.max as usize {
            return false;
        }
        self.starts.push_back(now);
        true
    }
}

async fn run_process(
    kind: RunnerKind,
    prompt: &str,
    cwd: &Path,
    timeout: Duration,
) -> std::result::Result<String, RunnerFailure> {
    use tokio::io::AsyncWriteExt;

    let runner = match kind {
        RunnerKind::Codex => "codex",
        RunnerKind::Claude => "claude",
    };
    let out_path = unique_temp("parler-worker-out", "txt");
    let err_path = unique_temp("parler-worker-err", "log");
    let err_file = std::fs::File::create(&err_path).map_err(|error| RunnerFailure::Failed {
        runner,
        detail: error.to_string(),
    })?;
    let mut command = tokio::process::Command::new(runner);
    command
        .args(runner_args(kind, cwd, &out_path))
        .current_dir(cwd)
        .env_remove("PARLER_SESSION_KEY")
        .stdin(Stdio::piped())
        .stderr(Stdio::from(err_file));
    match kind {
        RunnerKind::Codex => {
            command.stdout(Stdio::null());
        }
        RunnerKind::Claude => {
            let out_file = std::fs::File::create(&out_path).map_err(|error| RunnerFailure::Failed {
                runner,
                detail: error.to_string(),
            })?;
            command.stdout(Stdio::from(out_file));
        }
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            cleanup(&out_path, &err_path);
            return Err(RunnerFailure::NotInstalled(runner));
        }
        Err(error) => {
            cleanup(&out_path, &err_path);
            return Err(RunnerFailure::Failed { runner, detail: error.to_string() });
        }
    };
    let completion = async {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await?;
            stdin.shutdown().await?;
        }
        child.wait().await
    };
    let status = match tokio::time::timeout(timeout, completion).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            cleanup(&out_path, &err_path);
            return Err(RunnerFailure::Failed { runner, detail: error.to_string() });
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            cleanup(&out_path, &err_path);
            return Err(RunnerFailure::TimedOut { runner, secs: timeout.as_secs() });
        }
    };
    if !status.success() {
        let detail = tail(&err_path, MAX_ERROR_CHARS);
        cleanup(&out_path, &err_path);
        if looks_like_auth_failure(&detail) {
            return Err(RunnerFailure::NotAuthed(runner));
        }
        return Err(RunnerFailure::Failed {
            runner,
            detail: if detail.is_empty() { format!("exited with {status}") } else { detail },
        });
    }
    let output = std::fs::read_to_string(&out_path).unwrap_or_default();
    cleanup(&out_path, &err_path);
    let output = output.trim();
    if output.is_empty() {
        return Err(RunnerFailure::Failed {
            runner,
            detail: "produced no final response".into(),
        });
    }
    Ok(clip(output, MAX_RESULT_CHARS))
}

fn runner_args(kind: RunnerKind, cwd: &Path, out_path: &Path) -> Vec<OsString> {
    match kind {
        RunnerKind::Codex => vec![
            "exec".into(),
            "--sandbox".into(),
            "workspace-write".into(),
            "--ephemeral".into(),
            "--ignore-user-config".into(),
            "-C".into(),
            cwd.as_os_str().into(),
            "-o".into(),
            out_path.as_os_str().into(),
            "-".into(),
        ],
        RunnerKind::Claude => vec![
            "-p".into(),
            "--permission-mode".into(),
            "auto".into(),
            "--no-session-persistence".into(),
            "--strict-mcp-config".into(),
            "--mcp-config".into(),
            r#"{"mcpServers":{}}"#.into(),
            "--output-format".into(),
            "text".into(),
        ],
    }
}

fn unique_temp(prefix: &str, ext: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}.{ext}", std::process::id()))
}

fn cleanup(a: &Path, b: &Path) {
    let _ = std::fs::remove_file(a);
    let _ = std::fs::remove_file(b);
}

fn clip(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    output.push('…');
    output
}

fn tail(path: &Path, max_chars: usize) -> String {
    let value = std::fs::read_to_string(path).unwrap_or_default();
    let value = value.trim();
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().skip(value.chars().count() - max_chars).collect()
}

fn looks_like_auth_failure(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("login")
        || value.contains("unauthor")
        || value.contains("not authenticated")
        || value.contains("401")
}

#[cfg(test)]
mod tests {
    use super::*;
    use parler_connector::Config;
    use parler_protocol::{RoomKind, HANDOFF_KIND};
    use std::sync::{Arc, Mutex};

    fn endpoint(name: &str) -> parler_protocol::EndpointRef {
        parler_protocol::EndpointRef { id: format!("id-{name}"), name: name.into(), role: None }
    }

    fn message(from: &str, parts: Vec<Part>) -> StoredMessage {
        StoredMessage {
            seq: 1,
            id: "task-1".into(),
            room: "team".into(),
            from: endpoint(from),
            parts,
            mentions: None,
            reply_to: None,
            ts: 1,
        }
    }

    fn options(source: WorkSource, all_messages: bool) -> WorkOptions {
        WorkOptions {
            source,
            all_messages,
            allow_from: HashSet::new(),
            max_per_hour: 20,
            timeout: Duration::from_secs(10),
            once: true,
        }
    }

    #[test]
    fn targeted_handoff_is_work_and_plain_text_requires_opt_in() {
        let handoff = HandoffRef {
            next: "inspect origin/main".into(),
            summary: Some("local main is empty".into()),
            to: Some("worker".into()),
            bundle: None,
        };
        let targeted = message("alice", vec![handoff.to_part()]);
        match select_message(
            &targeted,
            SigStatus::Valid,
            "id-worker",
            "worker",
            None,
            &options(WorkSource::Room, false),
        ) {
            Selection::Work(item) => assert_eq!(item.instruction, "inspect origin/main"),
            Selection::Ignore(reason) => panic!("targeted handoff was ignored: {reason:?}"),
        }

        let plain = message("alice", vec![Part::text("inspect origin/main")]);
        assert!(matches!(
            select_message(
                &plain,
                SigStatus::Valid,
                "id-worker",
                "worker",
                None,
                &options(WorkSource::Room, false),
            ),
            Selection::Ignore(IgnoreReason::NotAddressed)
        ));
        let mut hub_mentioned = plain.clone();
        hub_mentioned.mentions = Some(vec!["worker".into()]);
        assert!(matches!(
            select_message(
                &hub_mentioned,
                SigStatus::Valid,
                "id-worker",
                "worker",
                None,
                &options(WorkSource::Room, false),
            ),
            Selection::Ignore(IgnoreReason::NotAddressed)
        ));
        assert!(matches!(
            select_message(
                &plain,
                SigStatus::Valid,
                "id-worker",
                "worker",
                None,
                &options(WorkSource::Room, true),
            ),
            Selection::Work(_)
        ));
    }

    #[test]
    fn unsigned_disallowed_and_lifecycle_messages_never_execute() {
        let plain = message("alice", vec![Part::text("change the code")]);
        assert!(matches!(
            select_message(
                &plain,
                SigStatus::Unsigned,
                "id-worker",
                "worker",
                None,
                &options(WorkSource::Room, true),
            ),
            Selection::Ignore(IgnoreReason::Untrusted)
        ));

        let mut restricted = options(WorkSource::Room, true);
        restricted.allow_from.insert("someone-else".into());
        assert!(matches!(
            select_message(
                &plain,
                SigStatus::Valid,
                "id-worker",
                "worker",
                None,
                &restricted,
            ),
            Selection::Ignore(IgnoreReason::Disallowed)
        ));

        let done = TaskRef {
            status: TaskStatus::Done,
            task: Some("prior".into()),
            note: None,
            result: None,
            tokens: None,
            elapsed_ms: Some(10),
        };
        let result = message("alice", vec![Part::text("finished"), done.to_part()]);
        assert!(matches!(
            select_message(
                &result,
                SigStatus::Valid,
                "id-worker",
                "worker",
                None,
                &options(WorkSource::Room, true),
            ),
            Selection::Ignore(IgnoreReason::Lifecycle)
        ));
    }

    #[test]
    fn explicit_result_handoff_executes_once_but_will_not_be_bounced_back() {
        let done = TaskRef::new(TaskStatus::Done);
        let handoff = HandoffRef {
            next: "integrate the result".into(),
            summary: None,
            to: Some("worker".into()),
            bundle: None,
        };
        let result = message("alice", vec![Part::text("result"), done.to_part(), handoff.to_part()]);
        match select_message(
            &result,
            SigStatus::Valid,
            "id-worker",
            "worker",
            None,
            &options(WorkSource::Room, true),
        ) {
            Selection::Work(item) => assert!(item.had_task_ref),
            Selection::Ignore(reason) => panic!("explicit return handoff was ignored: {reason:?}"),
        }
    }

    #[test]
    fn prompt_forces_execution_instead_of_summary() {
        let item = WorkItem {
            task_id: "42".into(),
            sender_id: "UA".into(),
            sender_name: "alice".into(),
            instruction: "inspect origin/main".into(),
            summary: Some("local main is empty".into()),
            context: None,
            bundle: None,
            had_task_ref: false,
        };
        let prompt = build_prompt(&item, "team", WorkSource::Room);
        assert!(prompt.contains("Execute the task now"));
        assert!(prompt.contains("do not merely summarize"));
        assert!(prompt.contains("inspect origin/main"));
        assert!(prompt.contains(HANDOFF_MARKER));
    }

    #[test]
    fn runner_result_extracts_only_a_valid_addressed_final_continuation() {
        let output = concat!(
            "finished the implementation\n",
            "PARLER_HANDOFF {\"to\":\"translator\",\"next\":\"translate the docs\",",
            "\"summary\":\"implementation is green\"}"
        );
        let result = parse_runner_result(output);
        assert_eq!(result.body, "finished the implementation");
        let handoff = result.continuation.expect("valid final marker should become a handoff");
        assert_eq!(handoff.to.as_deref(), Some("translator"));
        assert_eq!(handoff.next, "translate the docs");

        for invalid in [
            "body\nPARLER_HANDOFF not-json",
            "body\nPARLER_HANDOFF {\"next\":\"broadcast this\"}",
            "PARLER_HANDOFF {\"to\":\"translator\",\"next\":\"missing body\"}",
        ] {
            let result = parse_runner_result(invalid);
            assert!(result.continuation.is_none());
            assert_eq!(result.body, invalid);
        }
    }

    #[test]
    fn rate_gate_is_rolling_and_zero_disables_it() {
        let start = Instant::now();
        let mut gate = RateGate::new(2);
        assert!(gate.admit(start));
        assert!(gate.admit(start + Duration::from_secs(1)));
        assert!(!gate.admit(start + Duration::from_secs(2)));
        assert!(gate.admit(start + Duration::from_secs(3600)));

        let mut unlimited = RateGate::new(0);
        for _ in 0..100 {
            assert!(unlimited.admit(start));
        }
    }

    #[test]
    fn runners_are_argv_built_and_do_not_load_parler_mcp() {
        let cwd = Path::new("/tmp/work space");
        let out = Path::new("/tmp/result.txt");
        let codex = runner_args(RunnerKind::Codex, cwd, out);
        assert!(codex.iter().any(|arg| arg == "--ignore-user-config"));
        assert!(codex.iter().any(|arg| arg == "workspace-write"));
        assert!(codex.iter().any(|arg| arg == cwd.as_os_str()));

        let claude = runner_args(RunnerKind::Claude, cwd, out);
        assert!(claude.iter().any(|arg| arg == "--strict-mcp-config"));
        assert!(claude.iter().any(|arg| arg == r#"{"mcpServers":{}}"#));
    }

    struct FakeRunner {
        prompts: Arc<Mutex<Vec<String>>>,
        result: &'static str,
    }

    impl Runner for FakeRunner {
        fn name(&self) -> &'static str {
            "fake"
        }

        fn run<'a>(
            &'a self,
            prompt: &'a str,
            _cwd: &'a Path,
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = std::result::Result<String, RunnerFailure>> + Send + 'a>> {
            Box::pin(async move {
                self.prompts.lock().unwrap().push(prompt.to_string());
                Ok(self.result.to_string())
            })
        }
    }

    async fn start_hub() -> String {
        let store = parler_hub::Store::open(None).unwrap();
        let state = Arc::new(parler_hub::HubState::new(
            store,
            "parler://test".into(),
            "Worker Test Hub".into(),
            parler_hub::HubMode::Private,
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = parler_hub::serve(listener, state).await;
        });
        format!("ws://{addr}")
    }

    async fn agent(hub: &str, name: &str) -> MeshAgent {
        let cfg = Config {
            hub_url: hub.into(),
            identity: parler_auth::new_identity().unwrap(),
            name: name.into(),
            role: None,
            attention: Default::default(),
        };
        MeshAgent::connect(&cfg).await.unwrap()
    }

    fn text_and_kinds(messages: &[StoredMessage]) -> String {
        messages
            .iter()
            .flat_map(|message| &message.parts)
            .map(|part| match part {
                Part::Text(text) => text.clone(),
                Part::Extension { kind, .. } => kind.clone(),
                Part::Data(_) => "data".into(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn room_handoff_wakes_runner_and_posts_result_plus_one_return_turn() {
        let hub = start_hub().await;
        let mut alice = agent(&hub, "alice").await;
        let mut worker = agent(&hub, "worker").await;
        let invite = alice
            .invite(RoomKind::Channel, Some("team".into()), None, None)
            .await
            .unwrap();
        worker.join(&invite.code).await.unwrap();
        let handoff = HandoffRef {
            next: "inspect origin/main and report risks".into(),
            summary: Some("local main looks empty".into()),
            to: Some("worker".into()),
            bundle: None,
        };
        alice
            .send(
                Target::Room { room: invite.room.clone() },
                vec![handoff.to_part()],
                Some(vec!["worker".into()]),
                None,
            )
            .await
            .unwrap();

        let prompts = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner { prompts: prompts.clone(), result: "inspected the real branch" };
        let report = run(
            &mut worker,
            &invite.room,
            &options(WorkSource::Room, false),
            &runner,
        )
        .await
        .unwrap();
        assert_eq!(report.handled, 1);
        assert!(prompts.lock().unwrap()[0].contains("inspect origin/main and report risks"));

        let (messages, _) = alice.pull(&invite.room, None, None).await.unwrap();
        let rendered = text_and_kinds(&messages);
        assert!(rendered.contains("inspected the real branch"));
        assert!(rendered.contains(parler_protocol::TASK_KIND));
        assert!(rendered.contains(HANDOFF_KIND), "result should hand one turn back to alice");
    }

    #[tokio::test]
    async fn explicit_runner_continuation_routes_the_next_turn_without_an_automatic_bounce() {
        let hub = start_hub().await;
        let mut alice = agent(&hub, "alice").await;
        let mut worker = agent(&hub, "worker").await;
        let mut translator = agent(&hub, "translator").await;
        let invite = alice
            .invite(RoomKind::Channel, Some("pipeline".into()), None, None)
            .await
            .unwrap();
        worker.join(&invite.code).await.unwrap();
        translator.join(&invite.code).await.unwrap();
        alice
            .send(
                Target::Room { room: invite.room.clone() },
                vec![
                    HandoffRef {
                        next: "finish the implementation".into(),
                        summary: None,
                        to: Some("worker".into()),
                        bundle: None,
                    }
                    .to_part(),
                ],
                Some(vec!["worker".into()]),
                None,
            )
            .await
            .unwrap();

        let runner = FakeRunner {
            prompts: Arc::new(Mutex::new(Vec::new())),
            result: concat!(
                "implementation complete\n",
                "PARLER_HANDOFF {\"to\":\"translator\",\"next\":\"translate the docs\",",
                "\"summary\":\"implementation is green\"}"
            ),
        };
        let worker_id = worker.id.clone();
        run(
            &mut worker,
            &invite.room,
            &options(WorkSource::Room, false),
            &runner,
        )
        .await
        .unwrap();

        let (messages, _) = translator.pull(&invite.room, None, None).await.unwrap();
        let result_handoffs: Vec<HandoffRef> = messages
            .iter()
            .filter(|message| message.from.id == worker_id)
            .flat_map(|message| message.parts.iter().filter_map(HandoffRef::from_part))
            .collect();
        assert_eq!(result_handoffs.len(), 1);
        assert_eq!(result_handoffs[0].to.as_deref(), Some("translator"));
        assert_eq!(result_handoffs[0].next, "translate the docs");
        assert!(!text_and_kinds(&messages).contains(HANDOFF_MARKER));
    }

    #[tokio::test]
    async fn service_request_runs_and_result_is_dmed_to_signed_sender() {
        let hub = start_hub().await;
        let mut requester = agent(&hub, "requester").await;
        let mut worker = agent(&hub, "worker").await;
        requester
            .register(parler_protocol::Visibility::Private, Vec::new(), Vec::new(), None)
            .await
            .unwrap();
        let room = worker.serve("review").await.unwrap();
        requester
            .send_text(Target::Service { service: "review".into() }, "review the branch")
            .await
            .unwrap();

        let prompts = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner { prompts, result: "review complete" };
        let mut opts = options(WorkSource::Service, false);
        opts.allow_from.insert(requester.id.clone());
        let report = run(&mut worker, &room, &opts, &runner).await.unwrap();
        let dm = report.last_response_room.expect("worker should return a DM room");
        assert_ne!(dm, room);
        let (messages, _) = requester.pull(&dm, None, None).await.unwrap();
        let rendered = text_and_kinds(&messages);
        assert!(rendered.contains("review complete"));
        assert!(rendered.contains(parler_protocol::TASK_KIND));
        assert!(!rendered.contains(HANDOFF_KIND), "a service result does not bounce room turns");
    }

    #[tokio::test]
    async fn service_result_falls_back_to_queue_when_sender_has_no_directory_card() {
        let hub = start_hub().await;
        let mut requester = agent(&hub, "requester").await;
        let mut worker = agent(&hub, "worker").await;
        let room = worker.serve("review").await.unwrap();
        requester
            .send_text(Target::Service { service: "review".into() }, "review the branch")
            .await
            .unwrap();

        let runner = FakeRunner {
            prompts: Arc::new(Mutex::new(Vec::new())),
            result: "review complete without directory registration",
        };
        let mut opts = options(WorkSource::Service, false);
        opts.allow_from.insert(requester.id.clone());
        let report = run(&mut worker, &room, &opts, &runner).await.unwrap();
        assert_eq!(report.last_response_room.as_deref(), Some(room.as_str()));

        let (messages, _) = requester.pull(&room, None, None).await.unwrap();
        let rendered = text_and_kinds(&messages);
        assert!(rendered.contains("review complete without directory registration"));
        assert!(rendered.contains(parler_protocol::TASK_KIND));
    }
}
