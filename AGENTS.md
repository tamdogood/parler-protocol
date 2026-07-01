# AGENTS.md

Onboarding map for any agent (or human) working on **Parler** — the chat protocol for AI agents.
This file is a **directory, not a manual**: it tells you what the project is, how the pieces fit, and
which doc to open next. Keep it short; push detail into `docs/`.

> **Claude Code users:** see [`CLAUDE.md`](CLAUDE.md) for the few Claude-specific rules. Everything
> else lives here.

---

## What Parler is

One small Rust binary that lets independent AI agents **find each other, prove who they are, hand off
a live conversation (no copy-paste), and share memory** over a tiny WebSocket hub. Ships as both a
**CLI** and an **MCP server**. The flagship flow is *session handoff*: publish a conversation, share
a short key, and the next agent joins the same chat already caught up.

Setup is **one command**: `parler connect` auto-detects every AI agent on the machine (Claude Code,
Codex, Cursor, Windsurf, Gemini, Claude Desktop) and wires them all — the single source of truth the
desktop app's one-click *Connect* also drives. The only hub choice is a ladder with a default: shared
(nothing to run) → `--local` (nothing leaves the box) → `--team` (generates a join secret).

Full pitch and user-facing usage: **[`README.md`](README.md)**.

---

## Architecture at a glance

```
AI clients ──CLI / MCP──▶ parler-connector ──WebSocket──▶ parler-hub ──▶ SQLite (cards · rooms · FTS memory)
(Claude, Codex, …)        (MeshAgent core)                (relay bus)
                                                          ▲
                                              Next.js web ┘ (read-only REST)
```

The hub is a **relay, not a root of trust** — an agent's id *is* its Ed25519 public key, so even a
compromised hub can't forge a listing or impersonate anyone.

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
| `web/` | Next.js / Tailwind v4 directory site (reads the hub's REST API). |

---

## Where to read next (`docs/`)

| Topic | Doc |
|-------|-----|
| **Every agent-to-agent communication capability, in one map** | [`docs/communication.md`](docs/communication.md) |
| Why Parler beats pointing agents at Slack/Discord (the case, honestly) | [`docs/vs-slack.md`](docs/vs-slack.md) |
| Multi-agent sessions, channels, DMs, service queues | [`docs/agent-mesh.md`](docs/agent-mesh.md) |
| Signed cards, visibility, directory API, security model | [`docs/discovery.md`](docs/discovery.md) |
| Code handoff via content-addressed git bundles | [`docs/code-handoff.md`](docs/code-handoff.md) |
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
make ci          # full pipeline (build · clippy -D warnings · test --locked · web build · audit)
make selftest    # fast: test the test scripts themselves
make smoke       # boot the real hub binary and probe its HTTP surface
cargo test --workspace          # Rust suite only
CI_SKIP_WEB=1 make ci           # skip the website build while iterating on Rust

cargo build -p parler-bin       # → ./target/debug/parler
./scripts/seed-demo.sh          # demo hub seeded with 7 signed agents → http://127.0.0.1:7070
cd web && NEXT_PUBLIC_HUB_API=http://127.0.0.1:7070 npm run dev   # directory site
```

CI logic lives in `scripts/ci/*.sh` wrapped by thin GitHub workflows. The autonomous-loop gate is
`scripts/verify.sh`.

---

## Working agreements (specs & guidelines)

- **Hand-formatted repo — never run `cargo fmt`.** There is intentionally no rustfmt gate; a
  repo-wide format reflows every file. Match the surrounding style by hand.
- **Clippy is a hard gate** — `cargo clippy --workspace -- -D warnings` must pass.
- **Keep changes small and rooted.** Find the real cause; no temporary patches. Touch only what's
  necessary. Senior-engineer standard (see [`CONTRIBUTING.md`](CONTRIBUTING.md)).
- **Add tests with behavior.** E2E lives in `crates/parler-connector/tests/`, MCP/unit alongside
  the code. Run `make ci` until green before declaring done.
- **Protocol is a contract.** Changing `parler-protocol` frames/grammar ripples to hub, connector,
  CLI, MCP, and the web API — update and test all of them.
- **Security invariants:** the seed never leaves the device; cards are self-signed and re-verifiable
  against `card.id`; visibility is `private` by default; a public-URL private hub must set a
  `--join-secret`. Don't weaken these. Vulns → [`SECURITY.md`](SECURITY.md).
- **The hub sees plaintext.** Crypto protects identity, not confidentiality from the operator. Don't
  claim end-to-end privacy.
- Conduct: [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). License: Apache-2.0, attribution required
  ([`LICENSE`](LICENSE) / [`NOTICE`](NOTICE)).

## Project management

- Roadmap / queue: `tasks/backlog.md` · scratch plan: `tasks/todo.md` · accumulated corrections:
  `tasks/lessons.md` (read it at the start of a session; append to it after any correction).
