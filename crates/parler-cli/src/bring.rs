//! `parler bring <agent>` — a one-line second opinion from another AI agent, no copy-paste.
//!
//! v1 runs the target agent as a plain subprocess ("pipe mode"): feed it a context recap on
//! stdin, capture only its final answer, and hand it back. The caller (the CLI command or the
//! `parler_bring` MCP tool) decides what to do with the review — print it, or post it into a
//! session room so it lands in the conversation via `parler_recv`.
//!
//! Why pipe mode: it is deterministic and needs no protocol change — the agent never joins the
//! hub, so there is no identity to mint and no join gate to resolve. Driving a second agent as a
//! full MCP participant is a deliberate later step (see `docs/research/parler-bring-spec.md`).

use std::time::Duration;

/// Agents `bring` knows how to drive. v1: codex only. The requested agent is validated against
/// this fixed list before any command is built, and we always spawn via argv (never a shell
/// string), so a bring target is never interpolated into a shell and only these exact programs
/// can be launched.
pub const SUPPORTED_AGENTS: &[&str] = &["codex"];

/// Wall-clock bound on a single review. A real review is multi-minute; without a hard cap a
/// wedged agent would hang forever (and, from the MCP tool, leak a background process).
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// True if `bring` can drive `agent`.
pub fn is_supported(agent: &str) -> bool {
    SUPPORTED_AGENTS.contains(&agent)
}

/// Why a review failed — each maps to a remedy the user can act on (no debug dumps; #111).
pub enum BringError {
    /// The agent binary isn't on `PATH`.
    NotInstalled(String),
    /// The agent ran but looks unauthenticated (needs a login).
    NotAuthed(String),
    /// The review exceeded the timeout and was killed.
    TimedOut(u64),
    /// The agent exited non-zero, or something else went wrong.
    Failed { detail: String },
}

impl BringError {
    /// A single actionable line: what went wrong and the exact fix.
    pub fn remedy(&self) -> String {
        match self {
            BringError::NotInstalled(agent) => format!(
                "'{agent}' isn't installed (not on PATH). Install the {agent} CLI, then retry."
            ),
            BringError::NotAuthed(agent) => {
                format!("'{agent}' isn't logged in. Run `{agent} login`, then retry.")
            }
            BringError::TimedOut(secs) => format!(
                "the review didn't finish within {secs}s and was stopped. Retry, or raise \
                 --timeout-secs for a bigger review."
            ),
            BringError::Failed { detail } => format!("the review failed: {detail}"),
        }
    }
}

/// Build the prompt handed to the reviewing agent: a second-opinion instruction plus the context.
/// A caller-supplied `instruction` replaces the default preamble.
pub fn build_prompt(instruction: Option<&str>, context: &str) -> String {
    let preamble = instruction.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(
        "You are a senior engineer giving an independent second-opinion review. Read the context \
         below and reply with specific, actionable feedback: correctness risks, edge cases, and \
         concrete suggestions. Be concise and do not restate the context back.",
    );
    format!("{preamble}\n\n--- context ---\n{context}")
}

/// Run `agent` on `prompt` and return its review text. `agent` must be in [`SUPPORTED_AGENTS`]
/// (the caller validates first; this asserts it too).
pub async fn run_review(
    agent: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String, BringError> {
    match agent {
        "codex" => run_codex(prompt, timeout).await,
        other => Err(BringError::Failed {
            detail: format!("don't know how to bring '{other}'"),
        }),
    }
}

/// Drive `codex exec` in read-only pipe mode. The recap goes in on stdin; codex writes only its
/// final message to a `-o` file (all its header/prompt-echo/token chatter goes to stderr, which we
/// keep only for diagnostics). `--ignore-user-config` makes the run deterministic — the user's
/// personal model, reasoning effort and MCP servers can't change the result or add latency.
async fn run_codex(prompt: &str, timeout: Duration) -> Result<String, BringError> {
    use tokio::io::AsyncWriteExt;

    let out_path = unique_temp("parler-codex-out", "txt");
    let err_path = unique_temp("parler-codex-err", "log");
    let err_file = match std::fs::File::create(&err_path) {
        Ok(f) => f,
        Err(e) => return Err(BringError::Failed { detail: e.to_string() }),
    };

    let mut cmd = tokio::process::Command::new("codex");
    cmd.arg("exec")
        .arg("--sandbox")
        .arg("read-only") // a review must never touch the working tree
        .arg("--skip-git-repo-check") // may run outside a git repo
        .arg("--ignore-user-config") // deterministic: ignore the user's model/MCP/reasoning
        .arg("-o")
        .arg(&out_path) // final agent message only → this file (no stdout parsing)
        .arg("-") // read the prompt from stdin
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::from(err_file));

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = std::fs::remove_file(&err_path);
            return Err(BringError::NotInstalled("codex".into()));
        }
        Err(e) => {
            let _ = std::fs::remove_file(&err_path);
            return Err(BringError::Failed { detail: e.to_string() });
        }
    };

    // Feed the recap, then close stdin so codex stops waiting for more input.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            cleanup(&out_path, &err_path);
            return Err(BringError::Failed { detail: e.to_string() });
        }
        Err(_) => {
            // Timed out: kill the child and reap it so we don't leak a process.
            let _ = child.start_kill();
            let _ = child.wait().await;
            cleanup(&out_path, &err_path);
            return Err(BringError::TimedOut(timeout.as_secs()));
        }
    };

    if !status.success() {
        let stderr_tail = tail(&err_path, 800);
        cleanup(&out_path, &err_path);
        if looks_like_auth_failure(&stderr_tail) {
            return Err(BringError::NotAuthed("codex".into()));
        }
        let detail = if stderr_tail.is_empty() {
            format!("codex exited with {status}")
        } else {
            stderr_tail
        };
        return Err(BringError::Failed { detail });
    }

    let review = std::fs::read_to_string(&out_path).unwrap_or_default();
    cleanup(&out_path, &err_path);
    let review = review.trim().to_string();
    if review.is_empty() {
        return Err(BringError::Failed {
            detail: "codex produced no output".into(),
        });
    }
    Ok(review)
}

/// A unique temp path — avoids pulling in a temp-file crate at runtime (it's a dev-dep only).
fn unique_temp(prefix: &str, ext: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}.{ext}", std::process::id()))
}

fn cleanup(a: &std::path::Path, b: &std::path::Path) {
    let _ = std::fs::remove_file(a);
    let _ = std::fs::remove_file(b);
}

/// The last `max` characters of a file's contents, trimmed — for surfacing a failure cause.
fn tail(path: &std::path::Path, max: usize) -> String {
    let s = std::fs::read_to_string(path).unwrap_or_default();
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    s.char_indices()
        .nth(s.chars().count().saturating_sub(max))
        .map(|(i, _)| s[i..].to_string())
        .unwrap_or_else(|| s.to_string())
}

fn looks_like_auth_failure(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("login") || s.contains("unauthor") || s.contains("not authenticated") || s.contains("401")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_list_gates_agents() {
        assert!(is_supported("codex"));
        assert!(!is_supported("claude"));
        assert!(!is_supported("rm -rf /"));
    }

    #[test]
    fn build_prompt_uses_default_preamble_then_context() {
        let p = build_prompt(None, "review src/auth.rs");
        assert!(p.contains("second-opinion"));
        assert!(p.contains("--- context ---"));
        assert!(p.trim_end().ends_with("review src/auth.rs"));
    }

    #[test]
    fn build_prompt_honors_custom_instruction() {
        let p = build_prompt(Some("Only check for SQL injection."), "ctx");
        assert!(p.starts_with("Only check for SQL injection."));
        assert!(!p.contains("second-opinion"));
        assert!(p.contains("ctx"));
    }

    #[test]
    fn empty_instruction_falls_back_to_default() {
        let p = build_prompt(Some("   "), "ctx");
        assert!(p.contains("second-opinion"));
    }

    #[test]
    fn remedies_name_a_fix() {
        assert!(BringError::NotInstalled("codex".into()).remedy().contains("Install"));
        assert!(BringError::NotAuthed("codex".into()).remedy().contains("codex login"));
        assert!(BringError::TimedOut(300).remedy().contains("300s"));
        assert!(BringError::Failed { detail: "boom".into() }.remedy().contains("boom"));
    }

    #[test]
    fn auth_failure_sniff() {
        assert!(looks_like_auth_failure("Error: 401 Unauthorized"));
        assert!(looks_like_auth_failure("please run codex login"));
        assert!(!looks_like_auth_failure("model produced 12789 tokens"));
    }
}
