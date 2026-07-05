# CLAUDE.md

Guidance for Claude Code in this repository. **Start with [`AGENTS.md`](AGENTS.md)** — it is the
shared onboarding map (what Parler Protocol is, the crate/architecture layout, build/test commands, and the
`docs/` index). This file only adds the Claude-specific rules on top.

## The rules that bite

- **Never run `cargo fmt`.** This repo is hand-formatted and has no rustfmt gate; a repo-wide format
  reflows every file. Match surrounding style by hand.
- **`make ci` is the gate.** It mirrors the cloud pipeline exactly. Run it (or `CI_SKIP_WEB=1 make
  ci` while iterating on Rust) until green before calling a task done. `clippy -D warnings` is hard.
- **Respect the protocol contract.** A change to `parler-protocol` ripples into `parler-hub`,
  `parler-connector`, `parler-cli`, and `web/`. Update and test all of them, not just one crate.
- **Don't weaken the security model** (self-signed cards, seed never leaves device, private-by-
  default, join-secret for public-URL private hubs). See the security section in `AGENTS.md`.

## How to work here

- **Follow the contract.** [`docs/engineering-guidelines.md`](docs/engineering-guidelines.md)
  governs every change — the `code-standards` skill walks it step by step. Review diffs with the
  `parler-review` skill or the `code-reviewer` agent (`.claude/agents/code-reviewer.md`); both
  execute [`docs/code-review-guidelines.md`](docs/code-review-guidelines.md).
- **Plan first** for any non-trivial task (3+ steps); write the plan to `tasks/todo.md`. If it goes
  sideways, stop and re-plan rather than pushing through.
- **Use subagents** for research, exploration, and parallel analysis to keep this context clean —
  one focused task per subagent.
- **Verify before done.** Prove it works (tests, logs, real run). Would a staff engineer approve it?
- **Capture lessons.** Read `tasks/lessons.md` at session start; append the pattern after any
  correction from the user so the same mistake doesn't repeat.

Simplicity first · find root causes, no temporary fixes · touch only what's necessary.
