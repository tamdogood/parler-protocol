# Spec: `parler bring` — a one-line second opinion from another agent

Date: 2026-07-09 · Status: v1 (pipe mode) SHIPPED · Owner: UX-redesign lane (`tasks/todo.md`,
"the wire, not the window")

## Problem

The single most common thing a developer does with two AI agents is get one to review the other's
work — the exact workflow in Darren Bounds' widely-shared post (Claude Code builds, Codex reviews
read-only, findings go back). Today that is copy-paste: generate a review prompt, switch windows,
paste it into the second tool, paste the findings back. Parler's whole thesis is *don't shuttle
text between windows*. `parler bring` collapses that loop into one line.

This is deliberately the **funnel**, not the niche. Solo, one machine, two agents — a place where a
subprocess genuinely beats a protocol. But the motion a user learns here (`bring` a second agent
into the work) is the same one that pays off at the hackathon/teammate moment, which is where the
wire beats the window.

## Two possible transports

- **Pipe mode (v1, shipped).** `parler` opens/uses a session, runs the second agent as a plain
  subprocess fed the context on stdin, and posts its answer back into the session. The reviewing
  agent never joins the hub — no identity to mint, no join gate to resolve, no protocol change.
  Deterministic and low-risk.
- **MCP-joiner mode (v2, deferred).** The second agent self-bootstraps as a real Parler agent via
  `PARLER_SESSION_KEY` and participates through MCP tool calls (join → pull → reply) itself.

## Spike evidence (2026-07-09, machine-verified on macOS, `codex-cli 0.142.5`)

- `codex` is installed and logged in; `codex exec --sandbox read-only` exists exactly as the
  reference workflow used it.
- Prompt goes in on **stdin** (trailing `-` forces it); the **final answer is the only thing on
  stdout** — all header/prompt-echo/token-footer chatter goes to **stderr**. So pipe mode needs
  **zero output parsing**. `-o <file>` writes just the final message, which we use.
- `codex exec` **does** connect to MCP servers in headless one-shot mode (observed it dialing a
  configured server mid-run), so MCP-joiner mode is *mechanically* possible — but whether a
  one-shot exec **reliably** decides to call join→pull→reply in order was **not** demonstrated, and
  that reliability is the real risk. Deferred to its own v2 spike with a pass/fail measurement.

Verdict: ship pipe mode as v1; do not gate v1 on the joiner. Kill criterion for v2 is reliability
of the tool-calling loop across N runs, measured independently.

## v1 design (as shipped)

Surfaces (both thin adapters over `crates/parler-cli/src/bring.rs`):

- **CLI** `parler bring <agent> [--context … | --context-file <path|->] [--instruction …]
  [--room <room>] [--quiet] [--timeout-secs N]`. Runs the agent, prints the review; with `--room`,
  also posts it into that session.
- **MCP** `parler_bring { agent, context }`. Uses the active session (opens one seeded with the
  context if there is none), then **spawns the bundled `parler bring … --context-file - --room
  <room> --quiet` detached** and returns immediately. The review arrives as a normal message; the
  host reads it with `parler_recv`.

Key decisions and why:

- **Async return.** A real review is multi-minute; even trivial codex runs took seconds at high
  reasoning. Blocking the MCP call would hit the host's tool-call timeout, so `parler_bring` never
  awaits the review — it launches the work and returns a "reviewing now, read it with parler_recv"
  line. The review lands via the normal message path.
- **Shell out to ourselves, not a shared `MeshAgent`.** Every MCP tool is a synchronous
  `Result<String>` and a detached `tokio::spawn` can't hold `&mut state.agent`. Spawning the
  bundled binary's own `parler bring` (the pattern `parler connect` and the desktop app already
  use) gives the review its own hub connection with the same identity — two connections per id are
  fine (the hub keys subscribers as `id → Vec<connection>`). One implementation does the real work.
- **Context over stdin, never argv.** The recap can be large; passing it as a command-line arg
  risks `ARG_MAX`. The MCP tool pipes it to the child's stdin (`--context-file -`). We always spawn
  via argv (no shell string), so nothing from the context is ever shell-interpreted.
- **Security surface.** The bring target is validated against a fixed whitelist (`SUPPORTED_AGENTS
  = ["codex"]`) before any command is built, so only that exact program can be launched. `codex`
  runs `--sandbox read-only` (never touches the tree) and `--ignore-user-config` (the user's model,
  reasoning effort, and MCP servers can't change the result or add latency/failures). A hard
  timeout bounds a wedged run; on timeout the child is killed and reaped.
- **Remedies, not dumps (#111).** Failure maps to one actionable line: not installed → install the
  CLI; looks unauthenticated → `codex login`; timed out → retry or raise `--timeout-secs`; else the
  tail of stderr.
- **Failure is never silent in the async path.** The detached MCP flow runs with stderr nulled, so
  on failure `bring` posts "⚠ second opinion from <agent> failed: <remedy>" into the room
  (best-effort) before exiting — the host's next `parler_recv` surfaces the fix instead of a
  phantom "review never arrived" dead end (the #100 trap).
- **Tool-list budget.** `parler_bring`'s schema is minimal; the `tool_specs_stay_lean` test ceiling
  was raised with a documented justification (matching the `parler_send_file` precedent), not
  bypassed.

## Known v1 limitations (documented, not bugs)

- **No kill-on-session-close.** A review spawned from the MCP tool is bounded by its own timeout,
  not cancelled when the session closes. Acceptable for v1; the timeout caps the worst case.
- **Attribution.** The review is posted by the host's own identity, prefixed "🔎 second opinion
  from codex", rather than as a distinct agent. Distinct identity is a v2 (MCP-joiner) property.
- **codex only.** The whitelist is one entry by design; adding an agent is a whitelist entry plus a
  runner arm.

## Not in v1

MCP-joiner mode; cross-machine bring; kill-on-close; multiple review agents. See the parent plan.
