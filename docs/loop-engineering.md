# Loop engineering — running Parler Protocol autonomously

This doc is two things: a short briefing on **loop engineering** (the practice), and the **operating
manual** for the autonomous loop wired into this repo. If you just want to run it, jump to
[Run it](#run-it).

## What loop engineering is

"Loop engineering" (coined mid-2026, the successor to *prompt* engineering for the agentic era) is the
discipline of designing the cycle an AI agent runs — not just the one-shot prompt. An **agentic loop**
is a trigger plus a *verifiable goal*: the agent acts, observes the result, reasons about it, and
repeats until the goal is met, without a human re-prompting at each step. The canonical loop is:

> **Plan → Search → Modify → Verify → Repair → Summarize**

The power is in *closing* the loop. A failing test isn't just an error — it's new context for the next
turn. A type error isn't a blocker — it's a signal that an assumption was wrong. The thing that
separates a demo from a production agent is treating the loop as an engineering artifact with four
deliberately-designed parts:

| Part | Question it answers | In this repo |
| --- | --- | --- |
| **Prompt** (reason) | What should the agent do each turn? | `.claude/commands/work-next.md` |
| **Tools** (act) | What can it do? | Claude Code's file/bash/git tools |
| **Feedback** (observe) | How does it know if it worked? | `scripts/verify.sh` (the gate) |
| **Guardrails** (stop) | When must it stop? | stopping criteria below |

The single highest-leverage piece is **feedback quality**: a fast, deterministic, single-command gate.
If the agent can't trust its own "am I done?" signal, nothing else matters — "a loop with nothing to
push back is the agent agreeing with itself on repeat."

## How it's wired here

```
tasks/backlog.md   ──▶  /work-next  ──▶  scripts/verify.sh  ──▶  commit + check off
   (the queue)         (one item)        (the gate: must         (git-backed state,
                                          end VERIFY: PASS)        so a crash resumes)
        ▲                                        │
        └──────────  tasks/todo.md (log) + tasks/lessons.md (memory)  ◀── learnings
```

- **`tasks/backlog.md`** — the forward queue of small, shippable, prioritized items. The loop pulls the
  top unchecked item. *Distinct from* `tasks/todo.md`, which is the append-only log of finished work.
- **`scripts/verify.sh`** — the feedback signal. The loop runs it as **`--rust-only`** (build ·
  clippy `-D warnings` · test). The full form additionally runs the web `next build` and mirrors
  `.github/workflows/ci.yml` exactly — that's for Tam's manual pre-push check, **not** the loop. It
  prints `VERIFY: PASS` / `VERIFY: FAIL (<stage>)`. **It never runs `cargo fmt`** — this repo is
  hand-formatted.
- **The `web/` app is human-driven and out of the loop's scope.** The loop never edits `web/` or runs
  the web build; Tam drives the site by hand. A backlog item that needs UI work does only its
  Rust/CLI/protocol part and leaves a `[HUMAN] web: …` note. (This also keeps the loop off the
  disk-constrained `npm ci` path.)
- **`.claude/commands/work-next.md`** — the per-iteration prompt. Does exactly one item: orient → plan
  (split if too big) → baseline → modify → verify → land (commit on the current branch) → stop.
- **`tasks/lessons.md`** — the self-improvement memory; read at the top of every iteration, appended to
  after any correction. This is how the loop stops repeating mistakes.

### Stopping criteria & guardrails (the part that makes it safe)

- **One item per iteration.** The loop re-enters for the next; it never tries to clear the board.
- **No-progress guard.** If the same `verify.sh` failure survives two fix attempts, the item is marked
  `[BLOCKED] <reason>` and the loop stops thrashing.
- **Never commit a red tree**, never relax `-D warnings` to pass, never `cargo fmt`.
- **Additive only.** The hub is deployed live; non-additive wire-protocol changes go to the backlog's
  Icebox and need a human. Same for benchmarks/decisions parked there.
- **Git-backed state.** Each landed item is its own commit on the working branch, so an interrupted
  loop resumes cleanly from the backlog's checkboxes.

## Run it

One verified item, interactively:

```
/work-next
```

Autonomously, hands-off (Claude self-paces the interval between iterations):

```
/loop /work-next
```

Or on a fixed cadence — e.g. every 10 minutes:

```
/loop 10m /work-next
```

You can also point it at a specific item instead of the top of the queue:

```
/work-next streaming blob upload
```

The loop keeps going until the "Now" section of `tasks/backlog.md` is empty or every remaining item is
`[BLOCKED]`. Add work by appending items to `tasks/backlog.md` (keep them small and give each a clear
*Done when:*). Stop a running loop the way you stop any `/loop`: interrupt it.

## Maintaining the loop

- **Feed the queue.** The loop is only as good as `backlog.md`. Write items the size of one PR with an
  explicit done-condition; split anything bigger.
- **Tend the gate.** If CI changes, change `scripts/verify.sh` to match — the two must stay identical or
  the feedback signal lies.
- **Grow the memory.** When you correct the agent, make sure the rule lands in `tasks/lessons.md` so the
  next iteration already knows it.

### Sources

- [What Is Loop Engineering? — MindStudio](https://www.mindstudio.ai/blog/what-is-loop-engineering-ai-coding-agents)
- [Loop Engineering: design coding-agent loops that run while you sleep — explainx.ai](https://explainx.ai/blog/loop-engineering-coding-agents-claude-code-guide-2026)
- [Agentic Loops: From ReAct to Loop Engineering — Data Science Dojo](https://datasciencedojo.com/blog/agentic-loops-explained-from-react-to-loop-engineering-2026-guide/)
- [What Is the AI Agent Loop? — Oracle](https://blogs.oracle.com/developers/what-is-the-ai-agent-loop-the-core-architecture-behind-autonomous-ai-systems)
