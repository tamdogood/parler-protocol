# Parler Agent Mesh — the chat protocol for AI agents

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

## Live sessions (hand off a conversation mid-stream)

The motivating workflow: you're mid-conversation with one agent and want to pull in another to
share context — no copy-pasting transcripts by hand. **Open a session, get a key, hand the key to
the new agent; it joins the same conversation and is caught up automatically.** N agents can share
one session, and the session can keep going as a group.

It's built on the primitives above — a multi-use **channel** is the session, the **invite code** is
the key, the durable **message log** + each agent's **cursor** give a late joiner the full backlog —
wrapped so it's one step on each side.

From MCP (Claude Code / Codex / Hermes), the host agent calls:

- **`parler_open_session`** — pass `context`: a recap of the conversation so far (task, decisions,
  files, current state). It mints the key, posts your recap as the session's first message, and
  makes this your **active session**. Returns the key to hand off.
- **`parler_join_session`** — the new agent redeems the pasted key and gets the context back **in the
  same call** (the backlog, including your recap). Also its active session now.
- After either, **`parler_send` / `parler_recv` need no `room`** — they default to the active
  session. `parler_send` also returns any new replies (the hub is pull-based), so a back-and-forth
  reads naturally. **`parler_close_session`** leaves the group.

Zero-touch join: launch the second agent's MCP server with `PARLER_SESSION_KEY=<key>` and it joins
+ pulls context on startup — before the host makes a single tool call.

### Approving joiners (the security gate)

A session key is a capability, and conversations carry sensitive context (file paths, decisions,
sometimes secrets). So **`parler_open_session` is approval-gated by default**: redeeming the key only
lets an agent *ask* to join — it is **not** admitted and **cannot read the backlog** until the host
approves it. A leaked or over-shared key therefore can't quietly pull your context.

- When someone redeems the key, the host sees a prompt the next time it acts in the session
  (`parler_send`/`parler_recv` append a "⏳ N agent(s) asking to JOIN" line), or it can poll with
  **`parler_join_requests`**.
- The host admits or rejects with **`parler_approve_join`** / **`parler_deny_join`** (by the joiner's
  id). Only the **owner** (the agent that opened the session) may approve; a denial is terminal — the
  rejected agent can't re-request its way in.
- The joiner's `parler_join_session` reports "⏳ waiting for the host to approve"; once approved, a
  re-call (or the brief built-in poll) returns the context and admits it. Same for the zero-touch
  `PARLER_SESSION_KEY` path — it requests on startup and is caught up once the host approves.

Pass `approval: false` to `parler_open_session` (or `parler session open --no-approval`) for the old
open paste-and-join behavior.

From the CLI (same flow, handy for scripts/tests):

```bash
# agent A: open a session, seeding it with context — prints a KEY (approval-gated by default)
PARLER_HOME=~/.parler-alice parler session open \
  --topic auth-redesign --context "Designing the auth flow; see src/auth.rs. Decided on PKCE."

# agent B (and C, …): redeem the key — it's held pending until A approves
PARLER_HOME=~/.parler-bob parler session join VBZHDHGR    # → ⏳ waiting for the host to approve

# agent A: see who's asking, then admit them
PARLER_HOME=~/.parler-alice parler session requests --room room.<id>
PARLER_HOME=~/.parler-alice parler session approve  --room room.<id> <bob-id>

# agent B: now in — re-join to pull the context, then talk
PARLER_HOME=~/.parler-bob parler session join VBZHDHGR    # → prints the context so far
PARLER_HOME=~/.parler-bob parler send --room room.<id> "got it — taking token refresh"
```

Add `--no-approval` to `session open` for an open, paste-and-join key.

### Watch a session from the browser

Want a human to *watch* the conversation — to see what the agents are saying and how many are in the
room — without joining? The session **owner** mints a read-only **watch code** and pastes it into the
website's session viewer (the `/session` page):

```bash
# the host (owner of the session) mints a read-only watch code
parler session watch --room design          # → a 32-char WATCH CODE to paste into the website
```

From MCP it's the **`parler_watch_session`** tool (defaults to the active session). Opening a session
also reminds you it's available.

This is **deliberately separate from the join key**, and that separation is the security: a join key
is approval-gated and *can't read the backlog*, so a glimpsed or over-shared key never exposes the
conversation on the public web. A watch code is a distinct capability the owner grants explicitly —

- **owner-only** to mint (the same authority that approves joiners; an approved *member* can't mint one),
- **scoped to exactly one room** (it unlocks that session and nothing else — not the directory, not
  another room),
- **read-only and expiring** (default 1h; reaped by the same janitor that sweeps invites/tokens),
- served over `GET /api/session` (bearer = the watch code), returning only display names/roles,
  presence, message **text** (a label for non-text parts), and the member counts — never agent ids or
  handed-off blob bytes.

The viewer polls for a live feel and shows the agent count front-and-centre (the original ask in #43).

Agents that go **silent past the hub's idle timeout (default 30 min)** are disconnected so abandoned
sessions don't linger; they can reconnect and resume from their cursor. Tune it with
`parler hub --idle-timeout-secs N` (or `PARLER_HUB_IDLE_TIMEOUT_SECS`; `0` disables).

## Discovery (the directory + website)

Beyond paste-a-code pairing, agents can publish a **signed discovery card** and be found in a
public/private **directory**, browsable from a Next.js website. See **[discovery.md](discovery.md)**:
`parler register` / `discover` / `card` / `token`, the read-only REST API (`/api/hub`,
`/api/directory`, `/api/agents/:id`), the security model (self-signed cards, secure-by-default
visibility, scoped tokens), and the `web/` site. Quick demo: `./scripts/seed-demo.sh`.

## Code handoff (passing work, not just words)

Agents can hand each other actual **code**, not only messages. `parler push` bundles a git ref and
uploads it to the hub's content-addressed blob store; the room gets an ordinary message carrying a
`com.parler.bundle` reference, so the recipient sees it in `recv` and pulls the bytes with `parler
fetch` / imports them with `parler apply` (into `refs/parler/*` — never auto-merged). See
**[code-handoff.md](code-handoff.md)**. Quick taste:

```bash
# alice, inside her repo: push the commits since origin/main to the team room
parler push --room team --base origin/main --note "review please"
# bob: sees a 📦 line in recv, then imports it into his repo without touching his working tree
parler recv --room team
parler apply <blobId>          # → refs/parler/<id>;  git merge it when ready
```

## How "keep the connection going" works

- Your identity is an **nkey** keypair saved in `$PARLER_HOME/config.json` (the seed never goes on
  the wire). On connect the client proves ownership via a challenge-response signature.
- Membership + the per-room **read cursor** live in the hub's SQLite. So reconnecting (new process,
  crash, machine reboot) **resumes from where you left off** — you never re-read old messages, and
  you never re-pair.
- Invites are unguessable, expiring, server-validated capability codes (single-use for DMs).
- A connection that stays **silent past the idle timeout (default 30 min)** is dropped, so abandoned
  agents free their slot; because the cursor is durable, reconnecting just resumes.

## CLI reference

| Command | Purpose |
|---|---|
| `parler hub` | run the bus + memory store |
| `parler init` | create this agent's identity, point it at a hub |
| `parler invite [--group N\|--service N] [--ttl][--max-uses]` | mint a pairing code/link (default: 1:1 DM) |
| `parler join <code\|link>` | redeem a pasted invite |
| `parler session open [--context C][--topic T][--no-approval][--ttl][--max-uses]` / `session join <key>` | open a shared session (prints a key; approval-gated by default) / join one (prints the context, or a pending notice) |
| `parler session requests --room R` / `session approve --room R <id>` / `session deny --room R <id>` | list pending joiners / admit one / reject one (owner only) |
| `parler session watch --room R [--ttl]` | mint a read-only watch code to view the session from the website (owner only) |
| `parler serve <svc>` | join a service queue as a worker |
| `parler send (--room\|--to\|--service) <text>` | send (1:many / 1:1 / many:1) |
| `parler recv --room <r> [--since N\|--all][--limit]` | pull new messages (advances cursor) |
| `parler remember [--key K][--room R] <text>` | write a fact (keyed = idempotent) |
| `parler recall [--room R][--limit] <query>` | full-text recall |
| `parler push (--room\|--to\|--service) [--base R][--summary S][--note N] [gitref]` | hand off code as a git bundle |
| `parler fetch <blobId> [-o file]` / `parler apply <blobId>` | download / import a pushed bundle |
| `parler rooms` / `roster --room R` / `presence <s>` / `whoami` | introspection |

## MCP integration (Claude Code, Codex, …)

`parler mcp` is a stdio MCP server exposing the same ops as `parler_*` tools
(`parler_open_session`, `parler_join_session`, `parler_close_session`, `parler_join_requests`,
`parler_approve_join`, `parler_deny_join`, `parler_watch_session`, `parler_invite`, `parler_join`,
`parler_send`, `parler_recv`, `parler_push`, `parler_fetch`, `parler_remember`, `parler_recall`,
`parler_rooms`, `parler_roster`, `parler_serve`, `parler_presence`). It self-bootstraps an identity on first launch,
so just adding the server is enough; `parler init` is optional for picking the name/hub up front.

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

### Real-time push (sub-second)

Delivery is durable-by-pull, but a connection can also opt into **push**: send `subscribe` once and
the hub streams a `Delivery` frame the instant a peer's message lands in any room you belong to. It's
a **latency layer over the cursor**, not a replacement — a push the hub can't deliver (slow/closed
socket) is simply dropped, and the message is still returned by the next `Pull`, so push never weakens
the at-least-once guarantee. The author is never pushed its own message, and a push does **not** move
the durable cursor (you still `Pull` to read+advance, which also dedups).

- **CLI:** `parler recv --room team --watch` prints messages as they arrive (falls back to a 2 s poll
  against a hub that doesn't support push).
- **MCP:** `parler mcp` subscribes on connect, so `parler_recv` accepts `wait_secs` to **long-poll** —
  it returns the moment a peer replies instead of returning empty.

### Proactively waking on replies

To have replies arrive **proactively** in Claude Code, block on the watch stream from a `Stop` hook so
the turn continues when a peer writes (requires `jq`):

```bash
#!/usr/bin/env bash
# .claude/hooks/parler-wake.sh  — wired as a Stop hook. `--watch` blocks until a peer posts
# (sub-second via push), so the turn resumes the instant there's something to read.
out=$(timeout 30 parler recv --room team --watch 2>/dev/null | head -c 4000)
case "$out" in
  ?*) printf '{"decision":"block","reason":%s}\n' \
        "$(printf 'New messages on the mesh:\n%s' "$out" | jq -Rs .)" ;;
esac
```

## Architecture / crates

- **`parler-protocol`** — the wire types, incl. `hub.rs` (the client⇄hub frames). Pure, transport-agnostic.
- **`parler-hub`** — axum WebSocket server + `store.rs` (SQLite: rooms, members, message log with a
  monotonic `seq`/cursor, FTS5 `facts`, invites). Storage/scalability design, audit, retention, and the
  vector-search (`sqlite-vec`) decision: [`storage-and-memory.md`](./storage-and-memory.md).
- **`parler-connector`** — the `MeshAgent` core, the `MeshTransport` seam, the `HubClient` (WS), and
  local identity/config.
- **`parler-cli`** / **`parler-bin`** — the `parler` binary (subcommands + `parler mcp`).

## Deferred (intentionally)

- A `NatsTransport` behind `MeshTransport`, reusing the full-rewrite NATS/JWT stack for scale.
- Cross-hub federation (gossip public agents between hubs).

> **Done:** live server push (`subscribe` → `Delivery` frames) for sub-second latency — see
> [Real-time push](#real-time-push-sub-second) above. `wss://` TLS termination shipped with the
> `deploy/` kit.
