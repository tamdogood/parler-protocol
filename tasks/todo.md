# Feature: Agent Mesh — "Slack for agents" (focused build)

**2026-06-27 — user redirected scope.** Not a full Cotal copy. Deliver a focused feature: any agent
(Claude Code / Codex / Hermes) talks to any other in **1:1, many:1, 1:many**; an **efficient memory
backend**; and **paste-a-code pairing** ("tell my agent → it hands me a link/code → I paste it to the
other agent → the connection persists"). Must be **fast, low-cost, low-ops**.

### Architecture (proposed — confirm before building)
- **`parler-hub`** (new): one small binary = message bus + memory store.
  - WebSocket transport (axum); rooms + DMs + presence; the 3 delivery modes reuse `parler-protocol`
    `Route` (Multicast = 1:many, Unicast = 1:1, Anycast/inbox = many:1).
  - **Memory** = embedded SQLite (rusqlite, bundled, FTS5): append-only message log per room +
    `facts` table w/ full-text recall + per-agent read cursors (agents fetch only new/relevant → low token cost).
  - **Pairing**: `invite` mints a token signed with the hub nkey (reuse `parler-auth`) → returns
    `parler://<hub>/join?c=…` or a short code; `join` redeems → durable member cred → auto-reconnect.
  - No external NATS / JWT operator chain in the MVP (those stay as a future pluggable transport).
- **`parler-connector`** (build out the stub): the `MeshAgent` client **core**, exposed through thin adapters.
  - `MeshTransport` trait: `HubClient` (WebSocket, MVP) now; `NatsTransport` (reuse existing work) later.
  - **CLI** (`parler` binary) **and** **MCP** (hand-rolled JSON-RPC-over-stdio — no heavy SDK) wrap the SAME core.
  - **Wake** = Claude Code `Stop` hook (pull inbox → continue the turn) + the Hermes `MeshHandle` seam
    already waiting in `parler-connect-hermes/serve.rs`. Hermes via its Python plugin.
  - **Durable connection**: persisted nkey creds (`~/.parler/`) + hub-side per-(agent,room) cursor ⇒ reconnect resumes.

### Phases
- [x] **P1 Hub core** — axum WS server; nkey challenge-response identity; rooms/membership/presence;
  the 3 delivery modes (room/dm/service) over WS; SQLite persistence + per-(agent,room) cursors.
- [x] **P2 Pairing** — invite mint/redeem (capability codes + links), durable membership, reconnect/resume.
- [x] **P3 Memory** — message log + FTS5 `facts`; `remember`/`recall` with scope (room vs private); cursors.
- [x] **P4 Client (CLI + MCP)** — `MeshAgent` core + `MeshTransport` + `HubClient`; the `parler` CLI
  (`hub`/`init`/`invite`/`join`/`serve`/`send`/`recv`/`remember`/`recall`/`rooms`/`roster`/`presence`/
  `whoami`) **and** `parler mcp` (hand-rolled stdio MCP server, 10 `parler_*` tools) over the SAME core.
- [~] **P5 Wake + polish** — quickstart docs done (`docs/agent-mesh.md`, incl. a drop-in Claude Code
  `Stop`-hook + MCP config). *Still open:* wiring the Hermes `MeshHandle` seam to the live client;
  optional live server push (`Subscribe`/`Delivery`); a demo traffic generator.

### Review — 2026-06-27
Built the focused "Slack for agents" feature end-to-end (no full Cotal/NATS copy).
- **New/changed crates:** `parler-protocol::hub` (shared frames); new `parler-hub` (server + SQLite/FTS
  store); built out `parler-connector` (MeshAgent/HubClient/Config), `parler-cli` (the `parler` binary +
  `mcp` module), `parler-bin`.
- **Model:** everything is a *room*; the 3 patterns are membership shapes. Pull + durable cursor (no live
  push yet) ⇒ stateless-per-message hub, trivially durable, reconnect-resumes.
- **Tests:** `cargo test` green for the feature crates — protocol 18, hub 6 (store/server unit incl. FTS
  recall + invite limits + cursor), connector 1 + **6 e2e** (`mesh_e2e.rs`: 1:1 / 1:many / many:1 /
  memory scope / reconnect-resume / non-member-denied). Real-process smoke test passed: 2 agents pair via
  a code, broadcast+receive, recall a fact, and the MCP server answers initialize/tools.list/tools.call.
- **Pre-existing failure (not this work):** `parler-auth/tests/auth_live.rs` needs a `nats-server` binary
  that isn't vendored here (`.context/bin/nats-server`); unrelated to the mesh feature.

> The waves below are the **original full-parity rewrite plan**, now **deprioritized** per the redirection.

---

# Parler — build tracker

Full-parity Rust rewrite of [Cotal](https://github.com/Cotal-AI/Cotal). Plan:
`~/.claude/plans/system-instruction-you-are-working-tender-wolf.md`. Reference clone:
`.context/cotal-ref/`. Local `nats-server`: `.context/bin/nats-server`.

## Wave 0 — Foundation
- [x] Cargo workspace + 15 crate skeletons (`crates/parler-*`), shared workspace deps, `.gitignore`
- [x] `parler-protocol`: wire types (`types.rs`) + subject grammar (`subjects.rs`), rebranded `cotal`→`parler`
- [x] Protocol tests: SPEC §12 subject vectors, matchers, collapse, mentions, member-key, envelope round-trip (15 passing)
- [ ] `parler-protocol`: `schemars` schema gen → `spec/parler.schema.json` + validation test
- [x] `parler-auth`: nkeys identity (`identity.rs`) — id/seed/creds parse
- [x] `parler-auth`: NATS decentralized JWT v2 issuance (operator→account→user) + creds format
- [x] `parler-auth`: six profile ACLs + `nats-server` config render
- [x] **De-risk:** boot real `nats-server` with minted JWTs; connect with minted user creds; assert allow/deny ✅ (tests/auth_live.rs)

## Wave 1 — Core (`parler-core`)
- [ ] connection (creds/open) + stream & KV provisioning (exact policies from `streams.ts`)
- [ ] presence (KV heartbeat + stale→offline sweep + roster + watch)
- [ ] three delivery modes (multicast/unicast/anycast) with subject-derived authenticated kind
- [ ] explicit ack-on-surface; dedup by id across paths
- [ ] channels registry + history backfill (`historical=true`, watermark ack-drop)
- [ ] Plane-3 durable membership + fan-out/reader/dlv + ACL re-auth
- [ ] per-module integration tests vs live broker

## Wave 2 — Surfaces & connectors (parallel)
- [ ] `parler-connector`: MeshAgent + 17 `parler_*` tools + orientation/relay/control/launch
- [ ] `parler-manager`: control-plane handler + PTY runtime + roster + spawn/despawn + MAX_AGENTS
- [ ] `parler-delivery`: daemon (fan-out + trusted reader + single-flight lease)
- [ ] `parler-cli`: all subcommands + YAML manifest engine + MeshView model
- [ ] `parler-console`: ratatui TUI (+ plain stream)
- [ ] `parler-web`: axum HTTP+SSE dashboard (+ static assets)
- [ ] `parler-connect-claude` (rmcp MCP + hooks + transcript)
- [ ] `parler-connect-opencode` (Rust sidecar + JS plugin shim)
- [x] `parler-connect-hermes`: bridge protocol + serial ack-on-surface state machine + launch recipe + Python plugin (11 tests); live mesh via the `MeshHandle` seam, pending `parler-connector`
- [x] `parler-core` Runtime/Terminal/Launch contracts (the host-integration traits cmux/tmux/manager share)
- [x] `parler-cmux` driver (8 tests: CLI wrapper, temp-script gen, layout, id/ref parsing)
- [ ] `parler-tmux` driver (mirror of cmux over the tmux CLI)
- [ ] `parler-bin`: compose all subcommands into the `parler` binary

## Wave 3 — Integration & polish
- [ ] Full conformance suite (14 §12 MUSTs + interop scenario)
- [ ] Port the ~50 `*.smoke.ts` integration tests
- [ ] `demo` traffic generator
- [ ] Benchmarks vs Node (`criterion` + e2e RTT/throughput/memory) → `docs/benchmarks.md`
- [ ] docs / examples / Docker / release packaging

## Review
- 2026-06-24: Foundation + auth landed. `cargo test --workspace` green = **24 tests**
  (15 `parler-protocol` + 8 `parler-auth` unit + 1 live broker integration).
  - `parler-protocol`: untagged `Route` + `#[serde(flatten)]` emits exactly one of
    `channel`/`to`/`toService`; SPEC §12 subject-parse vectors pass.
  - `parler-auth`: hand-rolled NATS JWT v2 (operator/account/user) since `nats-jwt` lacks operator +
    JetStream limits. **Top risk retired**: `tests/auth_live.rs` boots the real `nats-server`, mints
    creds, and the broker enforces the agent ACL (declared-channel publish delivered; undeclared
    rejected) and account JetStream (manager creates the CHAT stream).
  - **Next:** `parler-core` endpoint (port the 133 KB `endpoint.ts`) — connection + stream/KV
    provisioning + presence + the three delivery modes, then the §12 interop scenario as the
    foundation-slice e2e (task #5).
- 2026-06-24: cmux + hermes parity. `cargo test --workspace` = **43 tests** green (added 8 cmux + 11
  hermes + the parler-core contracts). Added the `parler-core` host-integration contracts
  (Runtime/AgentHandle/Terminal/Launch) — Rust uses explicit construction, not the TS global Registry.
  - `parler-cmux`: full cmux CLI driver + Runtime + TerminalLayout; pane temp-script + layout JSON
    + workspace id/ref parsing all tested without a live cmux.
  - `parler-connect-hermes`: the bridge **wire protocol** + the serial **ack-on-surface** state
    machine (incl. the in-flight-eviction edge case) + the **launch** recipe, all tested; the
    **Python plugin** ported faithfully under `plugin/parler/` (adapter/hooks/tools/bridge_client,
    rebranded). The live mesh plugs into the `MeshHandle` trait in `serve.rs` once `parler-connector`
    lands; the unix-socket server is compiled glue around the tested state machine.
