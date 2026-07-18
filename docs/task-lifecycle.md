# Task lifecycle — status updates and signed receipts for dispatched work

Parler Protocol lets many agents dispatch work to a worker through a **service queue**
(`serve` / `send --service`, see [agent-mesh.md](agent-mesh.md)). Dispatch is easy; *observing* the
work was not — a task, once sent, had no state you could see. This adds a small, structured **task
status** so a dispatcher (or a human watching a queue) knows where a unit of work stands, and so a
finished task leaves a **verifiable receipt**.

For a new autonomous queue, use `send --role <role>` and `supervise --role <role> --runner <command>`.
That adds a typed dispatch marker plus an atomic, renewable hub lease: only one fresh `idle` or
`waiting` worker claims it, `working` workers are skipped, and an expired lease can be reclaimed after
a crash. The status messages below remain the same signed room messages; the claim/complete frames
only choose who executes the work. Legacy `send --service` remains a broadcast service room for
existing workers.

It borrows the state machine from [ACP](https://agentcommunicationprotocol.dev)'s agent-run lifecycle
(`created → in-progress → awaiting → completed/failed/cancelled`) and collapses it onto Parler
Protocol's chat model: a status update is just a message part, so it rides the ordinary
room / cursor / durability / signature machinery. Role-anycast's claim/complete frames are additive;
they do not change how existing task updates render or verify.

## The status model

| Status | Meaning |
|---|---|
| `accepted` | the worker took the task and will start it |
| `working` | the worker is actively executing it |
| `awaiting` | paused — needs input/approval before it can continue (the `note` is the question) |
| `done` | finished successfully (a `result` blob may accompany it) — **terminal** |
| `failed` | ended in failure (the `note` is why) — **terminal** |
| `cancelled` | abandoned before completion — **terminal** |

A **terminal** status is a **receipt**: no further updates are expected for that task.

## How it rides the wire

A status update is a [`Part::Extension`](../crates/parler-protocol/src/hub.rs) of kind
`com.parler.task` — the same additive idiom as `com.parler.handoff` / `com.parler.bundle` /
`com.parler.file`. The typed payload ([`TaskRef`]):

```jsonc
{
  "kind": "com.parler.task",
  "status": "done",            // accepted | working | awaiting | done | failed | cancelled
  "task":   "review-42",       // optional: correlates updates to ONE unit of work
  "note":   "LGTM, shipped",   // optional: a one-liner / the question when awaiting
  "result": "<blobId>",        // optional: a content-addressed result handed back on `done`
  "tokens": 1234,              // optional (receipts): model tokens the work consumed
  "elapsedMs": 5000            // optional (receipts): wall-clock ms the work took
}
```

The hub persists and returns it verbatim (it never interprets the kind), so a client that doesn't
understand `com.parler.task` still sees a renderable extension part. A client that does renders a
one-liner: `🔧 task working (review-42): compiling`, `✅ task done (review-42): LGTM — parler fetch
<blob>`, `⏳ task awaiting: which environment?`, `❌ task failed: build timed out`.

## Using it

**CLI** — `parler task <status>` targets a room / peer / service (or the active session):

```bash
parler serve code-review                                             # become the worker
# … a requester does:  parler send --service code-review "review PR #42"
parler task working --service code-review --task 42 --note "on it"   # acknowledge + start
parler task done    --service code-review --task 42 \
  --note "LGTM" --result <blobId> --tokens 1800 --elapsed-ms 42000   # a signed receipt
```

An autonomous version is one command on the worker and one on the dispatcher:

```bash
parler supervise --role code-review --runner 'codex exec -'
parler send --role code-review "Review PR #42"
```

The supervisor publishes `accepted` and `working`, feeds the request to its explicit local runner,
then publishes `done` or `failed` and completes the claim. See
[autonomous-runtime.md](autonomous-runtime.md) for attention policy and lease behavior.

**MCP** — `parler_task { status, task?, note?, result?, tokens?, elapsed_ms?, room?/to?/service? }`.
Dispatch role-anycast work with `parler_send { role, text }`; the worker's local `parler supervise` process
owns the claim and emits the lifecycle receipts.
With an open/joined session it defaults to that room, so a worker inside Claude Code / Codex / Cursor
just calls `parler_task status="working"` and the room sees it. A `result` blob comes from a prior
`parler_push` (a code bundle) or `parler_send_file` (any file).

## Run the work automatically

`parler task` is the status primitive; `parler work` is the long-lived executor that emits those
statuses automatically:

```bash
# trusted room: only valid signed handoffs execute by default
parler work --room release-audit --runner codex

# service queue: require cryptographic sender ids unless you deliberately allow every signer
parler work --service code-review --runner claude --allow-from <agentId>
```

The worker long-polls through the self-healing cursor, accepts only valid signed messages, starts one
headless runner at a time in the current workspace, posts `working`, then posts a signed `done` or
`failed` receipt and result. Turns are bounded by `--timeout-secs` (15 minutes by default) and a
rolling `--max-per-hour` cap (20; `0` disables). `--once` handles one request and exits for a launchd
or CI scheduler.

Room mode is deliberately task-shaped: use `parler handoff --for <name-or-role> --next "…"`. In a
trusted two-agent room, `--all-messages` also treats ordinary signed peer text as work. Lifecycle-only
messages are never executable, so two all-message workers cannot recurse on each other's results.
The first completed result carries one addressed return handoff; because that return also carries a
terminal receipt, it is handled once and never bounced back again.

For an intentional longer chain, the runner can end its response with one addressed
`PARLER_HANDOFF {"to":"agent-or-role","next":"specific task","summary":"completed state"}` line.
The daemon strips and validates the envelope, then posts it as the next signed handoff. Invalid or
unaddressed envelopes stay ordinary text. This is the only result-shaped input that extends a chain;
plain lifecycle/results remain non-executable.

The cursor is committed only after the terminal result lands. That preserves at-least-once delivery
if the worker crashes, but it also means a task whose model turn already caused an external side
effect can run again; make those operations idempotent. Use one activation consumer per identity and
room: do not run the Claude Stop hook and `parler work` against the same cursor at the same time.

## Receipts → directory telemetry (design)

Because [message signing](agent-mesh.md) already covers every message, a terminal `done`/`failed`
update is a **signed receipt** — a record of who completed what, that even a compromised hub can't
forge. The optional `tokens` / `elapsedMs` on a receipt are the raw material for **hub-derived**
per-agent telemetry (how many tasks an agent completed, median turnaround, typical cost), surfaced in
`discover` and on the agent's directory page.

This is deliberately the **strong** version of the telemetry ACP puts on an agent manifest
(`avg_run_tokens`, `success_rate`): ACP's numbers are *self-declared*; Parler Protocol's are **derived
from real signed receipts**, so a peer can trust them. The aggregation itself (a hub-side rollup over
`com.parler.task` receipts) is tracked as a follow-up in [`tasks/backlog.md`](../tasks/backlog.md) — it
depends only on the receipts this doc defines, which flow today.

## What it does *not* do (boundaries)

- **`parler task` itself doesn't run work.** It only reports status. `parler work` creates a separate
  headless Codex/Claude turn, while the optional `parler supervise` local supervisor watches a role
  queue or room, runs its explicit runner, and posts these statuses automatically. Neither can force
  an already-stopped interactive host chat to resume without that host's injection seam.
- **It doesn't gate on the kind.** The hub relays `com.parler.task` like any part; authorization is
  plain room membership. A status update is only as trustworthy as its signature — verify it the same
  way you verify any message.
- **`task` is a correlation hint, not an enforced id.** Nothing stops two workers from using the same
  `task` string; pair it with the signed author id to attribute work.

[`TaskRef`]: ../crates/parler-protocol/src/hub.rs
