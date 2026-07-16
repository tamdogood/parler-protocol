# Autonomous agents, attention, and role queues

Parler Protocol can deliver a durable message while the receiving model host is idle. A cursor proves
that an agent can read a message later; the receiver still needs a host injection seam or an explicit
runner to start a model turn. This document covers the autonomous paths that remove the need for a
human to enter the other chat and press Enter, including normal visible Claude Code, Codex, and
OpenCode sessions.

## Wake paths

1. **Visible supported-host conversation.** `parler conversation [KEY] --host
   codex|claude|opencode` keeps that host's normal UI attached. Signed peer messages become turns in
   the same visible conversation, human-typed turns are shared back, and no headless process or Enter
   press is involved.
2. **Connected Claude Code wake hook.** Outside the explicit visible conversation adapter, `parler
   connect` can install a short-lived Stop hook for compatible low-level MCP sessions. It receives a
   policy-approved batch and emits Claude Code's continuation response.
3. **Managed headless worker.** `parler work` is a separate local process that runs a bounded Codex
   or Claude turn for each signed task.
4. **Optional local supervisor.** `parler supervise` is a separate local process with an explicit runner
   command. It stays connected, receives work, runs the command, observes the child, and posts signed
   task updates. This is the portable fully autonomous option when a host has no injection API.
5. **Manual pull.** `parler recv` and `parler_recv` remain valid for a human-directed conversation;
   they do not claim to wake an idle host.

The hub stays out of process supervision. It persists messages, presence, role registrations, and
short task leases; it never spawns a child or executes a peer's command.

## Visible-host safety and turn flow

```bash
parler conversation --host claude --topic audit --resume last  # create in Claude Code
parler conversation KEY@HUB --host opencode                 # join from OpenCode
parler conversation KEY@HUB                                 # Codex is the default
```

The adapter accepts only validly signed peer messages and ignores its own messages. An ordinary
peer message fans out once; the automatic reply carries a task-result part, so it does not trigger
another automatic reply. A model can request one intentional continuation by ending with an
addressed `PARLER_HANDOFF` marker, which the adapter validates and converts to a signed handoff.

The user keeps the selected host's configured permission policy. Claude Code's invocation hooks only
wake the normal session and never handle `PermissionRequest` or `PreToolUse`. OpenCode leaves
permission requests on its attached TUI channel. Codex's bridge declines approval and elicitation
requests for an injected turn rather than granting them. A peer can request work, but cannot grant
itself more filesystem/network authority or impersonate human input.

Codex currently labels app-server WebSocket mode experimental. The command probes `codex` and the
loopback server at startup and fails with an update/troubleshooting message when that interface is
unavailable; it never substitutes a hidden headless worker.

Claude Code uses documented command-hook exec form and `asyncRewake`; OpenCode uses its documented
localhost server API and `attach` TUI. Both are invocation-scoped, validate host/session identifiers,
and fail with an update message if the required interface is unavailable. OpenCode API bodies are
bounded, its existing `OPENCODE_SERVER_PASSWORD` policy is preserved, and neither adapter opens its
loopback endpoint beyond `127.0.0.1`.

Each conversation terminal adds a private terminal-instance key to the workspace identity scope.
Two visible agents in the same directory therefore remain two cryptographic roster members instead
of collapsing into one shared identity. `PARLER_AGENT_SESSION` explicitly overrides that private
scope when a terminal host does not expose a stable identifier.

## Attention is local policy

```bash
parler attention open
parler attention dnd
parler attention focus
parler attention quiet --room team
parler attention muted --room noisy-room
parler attention inherit --room team
```

The MCP equivalent is `parler_attention`: use `mode=open|dnd|focus` globally, or
`room=<name>, mode=quiet|muted|inherit` for a room override.

| Policy | Wakes now | Other traffic |
|---|---|---|
| `open` | all peer messages | received normally |
| `dnd` | DMs, addressed handoffs, matching role work | held behind the durable cursor |
| `focus` | addressed handoffs and matching role work | held behind the durable cursor |
| `quiet` room | directed work in that room | ambient traffic is held |
| `muted` room | nothing | deliberately consumed without a host wake |

Only the global mode is mirrored into presence. Quiet and muted room lists never leave the receiver.
A held batch is not acknowledged, so opening attention later replays its durable context. A directed
message behind held ambient traffic can wake once while the batch remains held; the connector suppresses
repeat injection during that temporary re-read window. A non-held wake is acknowledged only after its
host injector accepts it; a failed injection stays durable and is retried.

## One connector contract

`parler-connector` exposes `ConnectorRuntime` so host integrations use one four-step contract:

```text
host lifecycle event  → lifecycle() → presence (with global attention)
host tool call        → send()      → signed, durable room message
hub pull              → receive()   → attention-filtered batch + cursor decision
host wake seam        → inject()    → host-native next model turn
idle host window      → listen_until() → push wake + durable Pull + inject
```

The Codex, Claude Code, and OpenCode conversation adapters are three implementations of `inject`; the
connected Claude Stop hook is a fourth, shorter-lived adapter. An integration keeps `listen_until`
outstanding while its host is idle and ready for work; it returns after one accepted injection so the
host can serialize that model turn, then listens again at the next idle boundary. Push is only the
doorbell: every candidate is recovered and authorized through durable Pull, a missed push is picked up
on the bounded recheck, and failed injection remains retryable. The contract does not invent an
injection capability where a different host has none. Such a host can still offer send/receive tools
and use the local supervisor for continuous operation.

The concrete parity requirements, resource bounds, and extension steps are in
[`visible-host-adapters.md`](visible-host-adapters.md).

## Presence stays live while the host is live

Presence is self-reported lifecycle plus connection freshness. The hub treats a row as offline after
five minutes without a fresh signal; it does not inspect whether a model process happens to be using
CPU. Protocol `Ping` now refreshes only the presence timestamp, preserving `working`/`waiting`, the
activity label, and attention. `parler mcp` also republishes its last lifecycle once a minute for
compatibility with older hubs, and `parler conversation` reports `working` during a turn and
`waiting` between turns. A connected, quiet agent therefore no longer appears offline merely because
it has not called `parler_presence` recently.

## Role-addressed anycast

`--service` remains backwards-compatible broadcast delivery: every service member can pull it. Use
`--role` when exactly one available worker should execute a task:

```bash
# worker machine: register the role and start a local autonomous runner
parler supervise --role code-review --runner 'codex exec -'

# dispatcher: send one typed, role-addressed request
parler send --role code-review "Review the current diff for correctness and security."
```

The request carries a signed `com.parler.dispatch` part. Each `parler supervise --role` worker reads the
ready-role index and asks the hub to claim the request. The claim succeeds for one worker only when
that worker has fresh `idle` or `waiting` presence; `working` workers do not receive new work. The
winner renews a bounded lease, publishes `accepted` / `working` / `done` or `failed` task messages,
then marks the claim terminal. A crashed worker's lease expires and another available worker can claim
the task, so execution is deliberately at-least-once.

`parler roster --room svc.code-review` shows status, attention, and `serving:<role>`. The MCP
`parler_send` tool accepts `role` for the same anycast request; it cannot be combined with `room`,
`to`, or `service`.

## Local supervisor scope

`parler supervise` is opt-in. It does not infer a runner, install a daemon, or execute a command received
from another agent. You provide `--runner`; only that locally authored command is passed to the shell.
Peer task content travels through stdin and `PARLER_*` environment values, never shell interpolation.

```bash
parler supervise --role deploy --runner './scripts/deploy-agent' --timeout-secs 900
parler supervise --room team --runner 'codex exec -' --once
```

The room form is a self-coordinating **body agent**: it continuously receives validly signed,
policy-approved peer
messages from one joined room, runs the configured local agent, and posts the result back. The role
form adds atomic claims. Output is capped, child streams are drained, leases are bounded, and a
timed-out child is stopped and reported failed. Use your usual operating-system process manager when
you want restart-on-crash behavior.
