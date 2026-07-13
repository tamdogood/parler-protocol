# AGENTS.md

Onboarding map for any agent (or human) working on **Parler Protocol** — the chat protocol for AI agents.
This file is a **directory, not a manual**: it tells you what the project is, how the pieces fit, and
which doc to open next. Keep it short; push detail into `docs/`.

> **Claude Code users:** see [`CLAUDE.md`](CLAUDE.md) for the few Claude-specific rules. Everything
> else lives here.

---

## What Parler Protocol is

One small Rust binary that lets independent AI agents **find each other, prove who they are, hand off
a live conversation (no copy-paste), and share memory** over a tiny WebSocket hub. Ships as both a
**CLI** and an **MCP server**. The flagship flow is *session handoff*: publish a conversation, share
a short key, and the next agent joins the same chat already caught up.

Setup is **one command**: `parler connect` auto-detects every AI agent on the machine (Claude Code,
Codex, Cursor, Windsurf, Gemini, Claude Desktop, OpenCode, VS Code, Cline) and wires them all — the single source of truth the
desktop app's one-click *Connect* also drives. The only hub choice is a ladder with a default: shared
(nothing to run) → `--local` (nothing leaves the box) → `--team` (generates a join secret).
Agent-hosted MCP and terminal commands scope identity per workspace/session, so parallel terminals
join rooms as distinct cryptographic members instead of reusing one flat config.

Full pitch and user-facing usage: **[`README.md`](README.md)**.

---

## Architecture at a glance

```
AI clients ──CLI / MCP──▶ parler-connector ──WebSocket──▶ parler-hub ──▶ SQLite
(Claude, Codex, …)        (MeshAgent core)                (relay bus)     cards · rooms/DMs/sessions
                                                          ▲              FTS+vector memory · blobs
                                              Next.js web ┘ (read-only REST + A2A cards + session viewer)
```

The hub is a **relay, not a root of trust** — an agent's id *is* its Ed25519 public key (ownership
proven by challenge-response on connect), so even a compromised hub can't forge a listing or
impersonate anyone. Setup is `parler connect`; the flagship flow is *session handoff* (share a key,
the next agent joins the same chat caught up); the desktop app (`desktop/`, Electron) wraps the same
binary for one-click Connect and a local hub.

- Diagram source: [`docs/architecture.mmd`](docs/architecture.mmd)
- Message-flow sequence: [`docs/sequence.mmd`](docs/sequence.mmd)

### Crate layout (`crates/*`, Cargo workspace)

| Crate | Role |
|-------|------|
| `parler-protocol` | Wire frames + types; transport-agnostic standard. `canonical_card_bytes` for signing. |
| `parler-auth` | nkey/Ed25519 identity, `sign`/`verify`, NATS JWT issuance (NATS path is deferred). |
| `parler-hub` | WebSocket bus + embedded SQLite store (directory, rooms, FTS5 memory) + REST API. |
| `parler-connector` | The `MeshAgent` client core + `MeshTransport` seam + WS `HubClient`. Shared by CLI & MCP. |
| `parler-cli` | `parler` subcommands (incl. `parler connect`, the one-command agent wiring) **and** the `parler mcp` stdio server — thin adapters over `MeshAgent`. |
| `parler-bin` | The umbrella `parler` binary. |

---

## Where to read next (`docs/`)

| Topic | Doc |
|-------|-----|
| **The engineering contract — how every change is written** (hard gates, invariants, definition of done) | [`docs/engineering-guidelines.md`](docs/engineering-guidelines.md) |
| **How every change is reviewed** (verified findings, severity ladder, checklists) | [`docs/code-review-guidelines.md`](docs/code-review-guidelines.md) |
| **Every agent-to-agent communication capability, in one map** | [`docs/communication.md`](docs/communication.md) |
| Multi-agent patterns (chaining, routing, parallel fan-out) as recipes over Parler verbs | [`docs/patterns.md`](docs/patterns.md) |
| Task lifecycle — status updates + signed receipts for dispatched work | [`docs/task-lifecycle.md`](docs/task-lifecycle.md) |
| Why Parler Protocol beats pointing agents at Slack/Discord (the case, honestly) | [`docs/vs-slack.md`](docs/vs-slack.md) |
| Multi-agent sessions, channels, DMs, service queues | [`docs/agent-mesh.md`](docs/agent-mesh.md) |
| Share a live session with your teammates (hackathons, group projects) | [`docs/team-sessions.md`](docs/team-sessions.md) |
| Signed cards, visibility, directory API, security model | [`docs/discovery.md`](docs/discovery.md) |
| A2A interoperability — project signed cards into A2A Agent Cards | [`docs/a2a-interop.md`](docs/a2a-interop.md) |
| Code handoff via content-addressed git bundles | [`docs/code-handoff.md`](docs/code-handoff.md) |
| File transfer between agents (any file, same content-addressed blob transport) | [`docs/file-transfer.md`](docs/file-transfer.md) |
| Storage internals, scaling ceilings, retention, sqlite-vec roadmap | [`docs/storage-and-memory.md`](docs/storage-and-memory.md) |
| CI/CD design (logic lives in testable scripts, not YAML) | [`docs/ci-cd.md`](docs/ci-cd.md) |
| Running the project autonomously (`/loop /work-next`) | [`docs/loop-engineering.md`](docs/loop-engineering.md) |
| Deploy a public hub (Fly.io + VPS/Caddy) | [`deploy/README.md`](deploy/README.md) |
| Run a private hub for your team (one command) | [`deploy/private/README.md`](deploy/private/README.md) |

---

## Build, test, run

Toolchain is pinned (`rust-toolchain.toml`, stable + clippy). The `Makefile` mirrors CI exactly —
`make ci` locally == the cloud pipeline.

```bash
make ci          # full pipeline (build · clippy -D warnings · test --locked · audit)
make selftest    # fast: test the test scripts themselves
make smoke       # boot the real hub binary and probe its HTTP surface
cargo test --workspace          # Rust suite only

cargo build -p parler-bin       # → ./target/debug/parler
./scripts/seed-demo.sh          # demo hub seeded with 7 signed agents → http://127.0.0.1:7070
```

CI logic lives in `scripts/ci/*.sh` wrapped by thin GitHub workflows. The autonomous-loop gate is
`scripts/verify.sh`.

---

## Working agreements (specs & guidelines)

The full contract is [`docs/engineering-guidelines.md`](docs/engineering-guidelines.md) (authoring)
and [`docs/code-review-guidelines.md`](docs/code-review-guidelines.md) (reviewing) — read them once
per session, whatever tool you are. The bullets below are the load-bearing summary.

- **Hand-formatted repo — never run `cargo fmt`.** There is intentionally no rustfmt gate; a
  repo-wide format reflows every file. Match the surrounding style by hand.
- **Clippy is a hard gate** — `cargo clippy --workspace -- -D warnings` must pass.
- **Keep changes small and rooted.** Find the real cause; no temporary patches. Touch only what's
  necessary. Senior-engineer standard (see [`CONTRIBUTING.md`](CONTRIBUTING.md)).
- **Add tests with behavior.** E2E lives in `crates/parler-connector/tests/`, MCP/unit alongside
  the code. Run `make ci` until green before declaring done.
- **Review before landing.** Self-review your diff against
  [`docs/code-review-guidelines.md`](docs/code-review-guidelines.md). In Claude Code the
  `parler-review` skill or the `code-reviewer` agent (`.claude/agents/code-reviewer.md`) runs it;
  other tools follow the doc by hand.
- **Protocol is a contract.** Changing `parler-protocol` frames/grammar ripples to hub, connector,
  CLI, MCP, and the web API — update and test all of them.
- **Docs track code — no drift.** Any user-facing change (CLI commands/flags, MCP tools, wire
  protocol, setup/config, REST API, security model) isn't done until `README.md`, `AGENTS.md`, and
  `docs/` match it — grep the changed name/flag/behavior across all of them and update
  every hit in the same PR. A phantom `parler_*` tool reference in the docs fails
  `test_docs_reference_only_real_tools` (part of `make ci`); the rest is on you to keep honest.
- **Security invariants:** the seed never leaves the device; cards are self-signed and re-verifiable
  against `card.id`; visibility is `private` by default; a public-URL private hub must set a
  `--join-secret`. Don't weaken these. Vulns → [`SECURITY.md`](SECURITY.md).
- **The hub sees plaintext.** Crypto protects identity, not confidentiality from the operator. Don't
  claim end-to-end privacy.
- **Writing a blog post?** Use the shared `write-blog` skill in
  [`.claude/skills/write-blog/`](.claude/skills/write-blog/) (invoke it in Claude Code, or read its
  `SKILL.md`). It enforces the house voice (no em dashes), picks a non-cannibalizing SEO angle, and
  runs the humanizer pass. `bash .claude/skills/write-blog/check.sh <file>`
  scans a draft for style fails. (The site itself is maintained in its own repo.)
- Conduct: [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). License: Apache-2.0, attribution required
  ([`LICENSE`](LICENSE) / [`NOTICE`](NOTICE)).

## Project management

- Roadmap / queue: `tasks/backlog.md` · scratch plan: `tasks/todo.md` · accumulated corrections:
  `tasks/lessons.md` (read it at the start of a session; append to it after any correction).
