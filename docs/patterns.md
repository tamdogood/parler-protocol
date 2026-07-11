# Multi-agent patterns — recipes over Parler Protocol verbs

The primitives in [communication.md](communication.md) compose into the standard multi-agent
workflows. This page is a cookbook: the same patterns ACP documents for *composing agents*
(chaining, routing, parallel fan-out), written with Parler Protocol's verbs. Every recipe works from
both the `parler` CLI and the `parler_*` MCP tools.

The one building block is **request → work → reply on a room**. A service queue makes it many-to-one;
a session makes it a shared conversation. Task status ([task-lifecycle.md](task-lifecycle.md)) makes
the work observable while it runs.

## Chaining — each agent builds on the last

Sequential hand-off, where one agent's output is the next one's input. Use a **turn handoff** so the
next agent continues without a human re-prompting:

```bash
# writer finishes, hands the draft to the editor with the context it needs
parler handoff --room pipeline --for editor \
  --summary "draft in refs/parler/draft (parler apply <blob>)" \
  --next "tighten the draft and hand it to translator"
# editor watches the room and continues the moment it's handed the turn
parler recv --room pipeline --watch
```

Each stage `handoff --for <next>`; the final stage posts the result. Attach real artifacts (a code
bundle, a file) with `--bundle` / `push` / `send-file` instead of pasting them into chat.

## Routing — a dispatcher picks the specialist

A router agent reads a request and forwards it to the right worker. Workers `serve` named queues; the
router `send --service` the one it chose:

```bash
# specialists register as workers
parler serve rust-review          # (on the rust reviewer)
parler serve docs-review          # (on the docs reviewer)

# the router classifies, then dispatches to the matching queue
parler send --service rust-review "review crates/parler-hub/src/server.rs"
```

Don't know who serves what? `discover` finds agents by tag/skill/role first, then you `send`. When
the queued **`offers` card field** lands, `discover --offers` will filter straight to hireable queues.

## Parallel fan-out — many workers at once, then gather

Dispatch independent sub-tasks to several workers, let them run concurrently, and collect the replies
on one room. A shared **session** is the natural gather point — every reply lands in the same
conversation, already correlated:

```bash
# open a session, seed it with the shared context, hand the key to N workers
parler session open --topic release-audit --context "audit v0.4 across security, perf, docs"
# each worker joins the same key and reports back into the room
parler task working --room release-audit --task security --note "scanning auth paths"
parler task done    --room release-audit --task security --note "no criticals" --result <blob>
# the coordinator pulls only what's new; task status shows which sub-tasks are still open
parler recv --room release-audit --watch
```

Because delivery is durable and pull-based, a coordinator that steps away resumes exactly where it
left off — no reply is missed while it was busy.

## Long-running work — status while it runs

For a task that takes minutes, don't leave the requester guessing. Post status as you go; the terminal
update is a **signed receipt**:

```bash
parler task accepted --service build --task 91
parler task working  --service build --task 91 --note "compiling"
parler task awaiting --service build --task 91 --note "need a signing key — approve?"
parler task done     --service build --task 91 --result <artifactBlob> --tokens 5200 --elapsed-ms 180000
```

See [task-lifecycle.md](task-lifecycle.md) for the status model and how receipts feed directory
telemetry.

## Second opinion — an independent review, inline

Pull another agent into your *current* conversation for a one-shot review — no copy-paste, its answer
lands in your session:

```bash
parler bring codex --context "does this auth flow look right? src/auth.rs"
```

→ From MCP this is `parler_bring`. See [communication.md](communication.md) for the boundary (the host
owns *when* an agent acts; Parler Protocol carries the intent and the context).

## Where to go deeper

| For… | Read |
|------|------|
| Every capability, one map | [communication.md](communication.md) |
| Sessions, DMs, channels, service queues, handoff | [agent-mesh.md](agent-mesh.md) |
| Task status + signed receipts | [task-lifecycle.md](task-lifecycle.md) |
| Find/verify/DM an agent with no pairing | [discovery.md](discovery.md) |
