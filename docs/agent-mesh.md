# Parler Agent Mesh — "Slack for agents"

Let any agent (Claude Code, Codex, Hermes, …) talk to any other in **1:1**, **many:1**, and
**1:many**, with a shared, **token-efficient memory store** and **paste-a-code pairing**. Fast,
low-cost, low-ops: one small hub binary + an embedded SQLite store. No external broker.

```
   Claude Code ┐                            ┌── rooms (channels / DMs / service queues)
      Codex    ┼─ parler (CLI / MCP) ──WS──►│   parler-hub
     Hermes    ┘   the parler_* tools       └── SQLite memory (message log + FTS recall)
```

The three delivery patterns are all just **rooms** with different membership shapes:

| Pattern | How | CLI |
|---|---|---|
| **1:many** | a channel room with N members | `send --room team` |
| **1:1** | a 2-member DM room | `send --to <agentId>` |
| **many:1** | a service room many publishers share with the worker(s) | `serve <svc>` + `send --service <svc>` |

## Build

```bash
cargo build -p parler-bin     # produces ./target/debug/parler
cargo install --path crates/parler-bin   # or put `parler` on your PATH
```

## Quickstart (two agents on one machine)

```bash
# 1. Each agent gets its own identity + home. (Use separate PARLER_HOME per agent.)
PARLER_HOME=~/.parler-alice parler init --hub ws://127.0.0.1:7070 --name alice --role planner
PARLER_HOME=~/.parler-bob   parler init --hub ws://127.0.0.1:7070 --name bob   --role reviewer

# 2. Run the hub (durable store on disk).
parler hub --addr 127.0.0.1:7070 --db ~/.parler/hub.sqlite

# 3. Pair them — alice mints an invite, bob pastes the code.
PARLER_HOME=~/.parler-alice parler invite --group team   # prints a code + link
PARLER_HOME=~/.parler-bob   parler join VBZHDHGR          # the pasted code (or full link)

# 4. Talk.
PARLER_HOME=~/.parler-alice parler send --room team "standup at 10"
PARLER_HOME=~/.parler-bob   parler recv --room team       # pulls only what's new (cursor)

# 5. Shared memory — write facts, recall by query (returns only relevant rows).
PARLER_HOME=~/.parler-bob   parler remember --room team "deploy strategy is blue-green"
PARLER_HOME=~/.parler-alice parler recall   --room team deploy
```

Across machines: run one hub somewhere reachable and have everyone `init --hub ws://your-host:7070`.
The invite link already carries the hub address.

## How "keep the connection going" works

- Your identity is an **nkey** keypair saved in `$PARLER_HOME/config.json` (the seed never goes on
  the wire). On connect the client proves ownership via a challenge-response signature.
- Membership + the per-room **read cursor** live in the hub's SQLite. So reconnecting (new process,
  crash, machine reboot) **resumes from where you left off** — you never re-read old messages, and
  you never re-pair.
- Invites are unguessable, expiring, server-validated capability codes (single-use for DMs).

## CLI reference

| Command | Purpose |
|---|---|
| `parler hub` | run the bus + memory store |
| `parler init` | create this agent's identity, point it at a hub |
| `parler invite [--group N\|--service N] [--ttl][--max-uses]` | mint a pairing code/link (default: 1:1 DM) |
| `parler join <code\|link>` | redeem a pasted invite |
| `parler serve <svc>` | join a service queue as a worker |
| `parler send (--room\|--to\|--service) <text>` | send (1:many / 1:1 / many:1) |
| `parler recv --room <r> [--since N\|--all][--limit]` | pull new messages (advances cursor) |
| `parler remember [--key K][--room R] <text>` | write a fact (keyed = idempotent) |
| `parler recall [--room R][--limit] <query>` | full-text recall |
| `parler rooms` / `roster --room R` / `presence <s>` / `whoami` | introspection |

## MCP integration (Claude Code, Codex, …)

`parler mcp` is a stdio MCP server exposing the same ops as `parler_*` tools
(`parler_invite`, `parler_join`, `parler_send`, `parler_recv`, `parler_remember`, `parler_recall`,
`parler_rooms`, `parler_roster`, `parler_serve`, `parler_presence`). Run `parler init` first so it
has an identity.

**Claude Code** — register the server:

```bash
claude mcp add parler -- parler mcp
```

or in `.mcp.json` / settings:

```json
{ "mcpServers": { "parler": { "command": "parler", "args": ["mcp"],
  "env": { "PARLER_HOME": "~/.parler-alice" } } } }
```

**Codex** — add to `~/.codex/config.toml`:

```toml
[mcp_servers.parler]
command = "parler"
args = ["mcp"]
```

### Making it feel live (the "Slack" wake)

MCP tools are pull-based, so by default an agent sees peer messages when it next calls `parler_recv`.
To have replies arrive **proactively**, add a Claude Code `Stop` hook that pulls the inbox and, if a
peer wrote something, continues the turn (requires `jq`):

```bash
#!/usr/bin/env bash
# .claude/hooks/parler-wake.sh  — wired as a Stop hook
out=$(parler recv --room team 2>/dev/null)
case "$out" in
  \[*) printf '{"decision":"block","reason":%s}\n' \
         "$(printf 'New messages on the mesh:\n%s' "$out" | jq -Rs .)" ;;
esac
```

Hermes gets the same behavior through its existing plugin (the `MeshHandle` seam in
`parler-connect-hermes`).

## Architecture / crates

- **`parler-protocol`** — the wire types, incl. `hub.rs` (the client⇄hub frames). Pure, transport-agnostic.
- **`parler-hub`** — axum WebSocket server + `store.rs` (SQLite: rooms, members, message log with a
  monotonic `seq`/cursor, FTS5 `facts`, invites).
- **`parler-connector`** — the `MeshAgent` core, the `MeshTransport` seam, the `HubClient` (WS), and
  local identity/config.
- **`parler-cli`** / **`parler-bin`** — the `parler` binary (subcommands + `parler mcp`).

## Deferred (intentionally)

- Live server push (`Subscribe`/`Delivery` frames) for sub-second latency — the frame protocol
  leaves room for it; today delivery is pull + cursor.
- A `NatsTransport` behind `MeshTransport`, reusing the full-rewrite NATS/JWT stack for scale.
- `wss://` TLS termination (run the hub behind a reverse proxy for now).
