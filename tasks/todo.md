# Task: Real-time push delivery (sub-second) — 2026-06-28

**User ask:** implement the roadmap item *"Real-time push delivery (sub-second; today delivery is
pull + durable cursor)"* (README.md:554).

**Design principle:** push is a **best-effort latency layer over the durable cursor**, never a new
delivery guarantee. A dropped/missed push is always recoverable by `Pull` (the per-(room,agent)
cursor remains the source of truth), so the hub keeps **no** per-subscriber durability — just live
fan-out. **Additive + backward-compatible**: new optional frames; an old client never subscribes and
behaves exactly as today; a new client against the *deployed* hub gets an `Error` to `subscribe` and
falls back to polling.

- [x] **Protocol** (`hub.rs`): `ClientFrame::Subscribe` (standing intent), `ServerFrame::Subscribed`
  (ack), `ServerFrame::Delivery { message }` (unsolicited; not echoed to author; does not advance the
  cursor) + round-trip tests.
- [x] **Store**: `room_member_ids(room)` (the fan-out recipient set).
- [x] **Hub**: per-connection bounded mpsc (`PUSH_BUFFER=256`) + `subscribers` registry on `HubState`
  (`subscribe`/`unsubscribe`/`fanout` keyed by agent id, conn-id tagged); `handle_socket` `select!`s
  socket-recv ⨉ push ⨉ idle-deadline; `Send` fans out best-effort (`try_send`, drop-on-full,
  prune-on-closed); deregister on disconnect.
- [x] **Connector**: `MeshTransport::subscribe`/`next_delivery` (default no-op so other transports
  compile); `HubClient` buffers `Delivery` in an `inbox` + demuxes it from replies (incl. `recv_binary`);
  `MeshAgent` wrappers.
- [x] **CLI**: `parler recv --watch` — subscribe + block on `next_delivery`, pull-on-wake (advances+
  dedups the cursor AND heartbeats the idle timer); 2 s polling fallback when push is unsupported.
- [x] **MCP**: auto-`subscribe` after connect (`McpState.push`); opt-in `wait_secs` long-poll on
  `parler_recv` (pull → wait → re-pull). `wait_secs` absent = unchanged behavior.
- [x] **Tests**: e2e `push_delivery_is_sub_second` + `unsubscribed_agent_is_never_pushed` (mesh_e2e)
  and `recv_wait_secs_long_polls_for_a_push` (mcp) + `push_delivery_frame_round_trips` (protocol).
- [x] **Docs**: README roadmap box ✓ + Good-first-issues; `docs/agent-mesh.md` Deferred→live + a
  `--watch` Stop-hook; `docs/discovery.md` + hub/server.rs module doc.
- [x] Gate: `scripts/verify.sh --rust-only` → **VERIFY: PASS**. `[HUMAN] web:` none (Rust/CLI/protocol).

**Verdict:** shipped, additive, backward-compatible. The deployed hub (parler-hub.fly.dev) keeps
working with old clients; a new client against an old hub gets an `Error` to `subscribe` → returns
`false` → stays pull-based. The elegant core: **push is a best-effort latency layer over the durable
cursor** — the hub holds no per-subscriber durability, a full/closed channel just drops the push, and
the recipient always recovers the message via `Pull`. So at-least-once + ordering are unchanged; only
latency improves (poll-interval → sub-second). Proven by the `push ⟂ cursor` assertion (a pull after
two pushes still returns the full backlog).

---

# Task: SQLite design + audit + agent-memory research (2026-06-28)

**User ask:** design/audit the hub's SQLite store so it stays scalable + corruption-safe as the
public hub grows; ensure messages are recorded correctly and easily retrieved; research the latest
agent-memory findings; decide whether to build a vector DB; **ensure agents can transmit big code
changes efficiently**; record everything in a doc.

- [x] Map the store (`parler-hub/src/store.rs`) — schema, indexes, concurrency, durability
- [x] Audit message recording (seq/cursor atomicity, at-least-once, corruption surface)
- [x] Audit big-message / code path (blobs on disk, WS-binary, dedup, RAM, GC)
- [x] Research agent memory (Letta/Mem0/Zep/Graphiti; episodic/semantic/procedural)
- [x] Research SQLite-at-scale + sqlite-vec hybrid vs dedicated vector DB
- [x] Write `docs/storage-and-memory.md` (design + audit + research + phased roadmap)

**Verdict:** store is **corruption-safe today**; the single shared connection is the throughput
ceiling; message/fact/blob growth is **unbounded** (needs retention). Code transfer is architected
right (content-addressed blobs off the SQLite path) but uploads are fully buffered in RAM and blobs
never GC. Recommend FTS5 now + **`sqlite-vec` hybrid later with client-supplied embeddings** — do
**not** stand up a separate vector database. Full write-up: `docs/storage-and-memory.md`.

## Implementation (user: "implement all the phases")
- [x] **P0 config & integrity** — per-connection pragmas (`synchronous=NORMAL`, cache/mmap/temp_store,
  `busy_timeout=5s`, `foreign_keys`), `auto_vacuum=INCREMENTAL`, `idx_members_agent`, `quick_check()`
- [x] **P1 durability & growth** — `prune_messages`/`prune_facts`/`gc_blobs`/`sweep_expired`/
  `incremental_vacuum` + `blobs.last_fetched`; background **janitor** (spawn_blocking) + CLI/env flags;
  Litestream opt-in scaffold (`deploy/litestream.yml` + deploy docs)
- [x] **P2 concurrency unlock** — 1 writer + read-only WAL connection **pool** (`w()`/`r()`); hot reads
  fan out; `pull` reads on a reader, advances cursor on the writer. *S4 (`rooms.last_seq`) skipped on
  purpose — taxes the hot send path to speed a cold read; the COUNT is index-backed.*
- [◑] **P3 big-blob efficiency** — blob GC + LRU landed (P1); chunked/streaming upload (B1) = scoped
  cross-crate follow-up (current single-frame path works to the 25 MiB cap)
- [⏳] **P4 semantic memory** — designed; needs the `sqlite-vec` dep + a client embedding source (none
  exists yet); land as a focused follow-up so the deployed protocol isn't half-changed

**Verification:** 44 tests green — hub **22** (incl. `file_backed_pool_reads_see_writes`, retention,
`quick_check`, `janitor_pass`), connector e2e **15** (all delivery modes/sessions/reconnect/idle/code
handoff), CLI/MCP **7**; `cargo clippy -p parler-hub -D warnings` clean; `--release` binary builds.
The file-backed pool test caught (and I fixed) that `quick_check` must run on the writer because
FTS5 index validation needs write access — a good example of the read/write split being exercised.
All additive + backward-compatible; **no `cargo fmt`** (hand-formatted repo). Not committed.

---

# Feature: Code Handoff — git-bundle artifact passing (2026-06-27, BUILT)

**User ask:** investigate [ottogin/agenthub](https://github.com/ottogin/agenthub) and borrow the good
stuff. Conclusion: Parler is the *communication* plane (Slack); agenthub is the *artifact* plane
(GitHub). The gap worth filling is that agents can pass messages/facts but **not work artifacts**.
Borrowed the **git-bundle transport** (not agenthub's commit-DAG/GitHub metaphor). Full spec:
`docs/code-handoff.md`.

Design in one line: a handoff = a content-addressed **blob** (sha256 of a git bundle, on the hub's
disk, bound to its room) + an ordinary room message carrying a `Part::Extension { kind:
"com.parler.bundle", ... }`. Bytes move over the **already-authenticated WebSocket as binary frames**
(no new HTTP channel, no new dep, no capability tokens). `send`/`recv`/cursor/wake all work unchanged.

## Phase 1 — blob handoff (MVP)
- [x] `parler-protocol::hub`: `PutBlob`/`GetBlob` (`ClientFrame`); `BlobReady`/`BlobStored`/`BlobIncoming` (`ServerFrame`)
- [x] `parler-protocol::hub`: `BUNDLE_KIND` const + `BundleRef::{to_part,from_part}` (+ round-trip test)
- [x] `parler-hub::store`: `blobs` + `blob_rooms` tables (metadata; bytes on disk); `BlobMeta`/`put_blob_meta`/`blob_meta`/`blob_readable_by` (+ test)
- [x] `parler-hub::server`: `PutBlob` (resolve `Target` + member + size → `BlobReady`) → consume one Binary frame (verify sha256+len) → persist → `BlobStored`
- [x] `parler-hub::server`: `GetBlob` (member-of-any-bound-room check → `BlobIncoming` + Binary frame)
- [x] `parler-hub`: `HubState::new` + `{blob_dir,max_blob_bytes}` + flags/env; `serve` creates the dir
- [x] `parler-connector::client`: `recv_binary` + `MeshTransport::{upload_blob,download_blob}`
- [x] `parler-connector::agent`: `push(target, bundle, meta, note)`, `fetch_blob(id)`, `BundleMeta`, `PushReceipt`
- [x] `parler-cli`: `push` (git bundle create → upload → post message), `fetch` (bytes only), `apply` (verify+fetch into `refs/parler/*`, never auto-merge)
- [x] `parler-cli`: `recv` renders a `com.parler.bundle` part (📦, full blob id in the apply hint)
- [x] `parler-cli::mcp`: `parler_push`, `parler_fetch` (NO apply)
- [x] e2e test: push → recv (sees bundle part) → fetch_blob → bytes match → non-member denied
- [x] content-address helper `parler_auth::content_id` (single source of truth for hub + connector)

## Phase 2 — defense (borrowed from agenthub)
- [x] `max_blob_bytes` enforced (default 25 MiB, `--max-blob-bytes`/env) at PutBlob + on the received frame
- [x] per-agent in-memory fixed-window rate limits (`RateLimits`: 240 sends/min, 120 blobs/hour) on `HubState`

## Phase 3 — frontier (deferred; possible scope creep)
- [ ] index latest bundle per room (tip/summary/author); `parler frontier --room R`; surface in `rooms`/website

## Review — 2026-06-27
Built Phase 1 + Phase 2. Decisions: **WS-binary** transport (no new dep/HTTP/token surface),
**single-frame** blobs, **25 MiB** cap, Phase 3 deferred.
- **Tests:** `--no-fail-fast` across touched crates green — protocol **24** (+blob frames, +BundleRef
  round-trip), hub **10** (+blob meta/room binding), connector e2e **7** (+`code_handoff_*`) & discovery
  **5**. Only failure is the pre-existing `parler-auth` `auth_live` (needs a vendored `nats-server`).
- **Live, real git:** two `parler` agents over a real hub — `push` a real git bundle → peer `recv`s
  the 📦 handoff → `apply` lands the **exact tip** in a fresh repo (both commits present) → non-member
  `fetch` denied → blobs persisted content-addressed under `<db>.blobs/`.
- **Clippy:** clean except a **pre-existing** `large_enum_variant` on `ServerFrame::Card` (DirectoryEntry),
  unrelated to this feature; new variants are tiny.
- **Additive / backward-compatible:** new frames + one extension kind; old clients render an unknown
  bundle part gracefully. Docs: `docs/code-handoff.md` (full spec, "as built"), `docs/agent-mesh.md`,
  `README.md` updated.

---

# Feature: The first public hub — deploy + wss:// (2026-06-27)

**User ask:** create the first server anyone can publish their agents onto, so it's the first live
example (the website was showing "Can't reach the hub" against `127.0.0.1:7070`). Confirmed host:
**Fly.io** (+ a portable Caddy recipe).

**Key finding:** the only real code blocker was that `tokio-tungstenite` was declared with **no TLS
feature**, so `wss://` dials failed at runtime — exactly why the roadmap's TLS box was unchecked. The
hub already binds `0.0.0.0`, serves a CORS-open REST API, and is fully env-configurable; the rest was
deploy plumbing + docs.

### Built
- [x] **TLS client** — `tokio-tungstenite` now `features = ["rustls-tls-webpki-roots"]` (bundled CA
  roots; reuses the rustls already pulled by async-nats). `client.rs` already normalized
  `https://→wss://`; now it actually connects. Build green.
- [x] **Hub landing page** (`GET /`) — was a 404; now a small dark self-documenting page (hub
  name/mode/counts + the 3-command publish snippet derived from `public_url` + API/repo links, and an
  optional `PARLER_HUB_WEB` link to the directory site). +2 unit tests (url helper, escaping/snippet).
- [x] **`deploy/` kit** — `Dockerfile` (glibc builder → distroless/cc, builds `parler-hub`),
  root `.dockerignore`, `fly.toml` (volume + http_service + `/health` check, always-on),
  `docker-compose.yml` + `Caddyfile` (auto-TLS self-host = the documented TLS recipe), `README.md`.
- [x] **Wiring + docs** — `web/.env.example` (prod HTTPS hub + Vercel note, dev fallback kept);
  README "Deploy a public hub" section + TLS roadmap box ticked; `docs/discovery.md` transport note
  + "Try it" point at `deploy/`.

### Review — 2026-06-27
- **Tests:** `cargo test --workspace` green except the *pre-existing* `auth_live` test (needs a
  vendored `nats-server`, unrelated). `parler-hub` now **13** tests (+`display_hub_url`,
  +`landing_page…`, +`open_creates_missing_parent_dir`); `connector` e2e (5+6) + `ws_url_normalization`
  still green.
- **Live publish smoke (`ws://`):** booted `parler hub --public`, `init`+`register --public` an agent
  → `/api/directory` returns it `verified:true`, `/api/hub` shows `agents:1/public:1`, `/` renders the
  publish guide. (`.context/smoke-public-hub.sh`.)
- **Container run-check:** `docker build` → **39.9 MB** distroless image; `docker run` with **no
  volume** boots a `public` hub, auto-creates `/data`, and serves `/health` + `/api/hub` + `/`.
- **Root-cause fix found by the run-check:** `Store::open` didn't create the DB's parent dir, so a
  fresh `/data` (or any new `--db` dir) errored `unable to open database file`. Fixed in `store.rs`
  (mirrors `Config::save`'s `create_dir_all`) + a regression test.

### Left to the user (outward-facing; needs their account)
- `fly deploy --config deploy/fly.toml` under their Fly account, then set
  `NEXT_PUBLIC_HUB_API=https://<app>.fly.dev` in Vercel. (I prepared everything to a one-command deploy
  but didn't provision under their account.)

---

# Feature: Agent Discovery — directory + signed cards + Next.js site (2026-06-27)

**User ask:** the best discovery hub — agents register with a uuid + a public/private visibility
(public = discoverable by any agent; private = same-hub only), Slack-like, with a strong security
protocol, plus a Next.js + shadcn dark-theme website (Resend styling) to browse a hub or the public
directory. Confirmed: one hub binary in public/private mode; private-hub viewing via a short-lived
directory token; ship a runnable demo. Plan: `~/.claude/plans/recursive-hatching-hearth.md`.

### Built
- [x] **Protocol** (`parler-protocol::hub`): `Visibility{public,private}` (default private),
  `DiscoverScope{hub,public}`, `DirectoryEntry`, frames `Register/Discover/Lookup/MintDirectoryToken`
  + `Registered/Directory/Card/DirectoryToken`, and `canonical_card_bytes` (RFC-8785-style).
- [x] **Auth**: `parler_auth::{sign,verify}` (nkey Ed25519), reused by hub + connector + tests.
- [x] **Hub store**: `directory` + `directory_tokens` tables; `register_card`, `discover`
  (scope/tag/skill/status filters), `lookup_card`, token mint/validate; presence now self-reported
  and **decayed to offline by staleness** (`PRESENCE_STALE_MS`) instead of forced on disconnect.
- [x] **Hub server**: WS ops (verify signature, bind `card.id == authed id`); read-only REST
  `/api/hub`, `/api/directory`, `/api/agents/:id` with `tower-http` CORS + bearer-token gating for
  `scope=hub`; `--name`/`--public` flags + `HubMode`.
- [x] **Connector + CLI + MCP**: `MeshAgent::{register,discover,lookup,mint_directory_token}`
  (signs the card with the local seed); CLI `register/discover/card/token`; MCP `parler_register/
  parler_discover/parler_card`.
- [x] **Website** (`web/`): Next.js 15 + Tailwind v4 + shadcn-style, Resend dark theme — nav/hero,
  hub header, scope toggle, search + filters, signed agent cards with status + verified badges, a
  detail sheet, and a token-unlock dialog. Builds clean; screenshot-verified against a live hub.
- [x] **Demo + docs**: `scripts/seed-demo.sh` (public hub + 7 signed agents, 5 public/2 private),
  `docs/discovery.md`, pointer in `docs/agent-mesh.md`.
- [x] **Discovery → conversation bridge** (follow-up): a `register`ed agent is *reachable* — a peer
  can `send --to <id>` cold and the hub opens the DM room (no paste-a-code). `resolve_target` falls
  back to pairing only for agents with no directory card. Verified with a live two-agent round-trip
  (atlas DMs probe by id → probe reads + replies). Tests +2 in `discovery_e2e`.

### Review — 2026-06-27
- **Tests:** `cargo test --workspace --no-fail-fast` = **69 passed / 1 failed**; the single failure
  is the pre-existing `parler-auth` `auth_live` test (needs a vendored `nats-server`, unrelated).
  New: protocol +4 (frames/canonicalization/default), auth +1 (sign/verify), hub +3 (scope split,
  visibility/idempotent register, token expiry), connector +3 e2e (`discovery_e2e`: public-vs-hub
  visibility, forged/tampered/unsigned card handling, token mint).
- **Live demo verified:** `/api/hub` → public hub "Parler Public", 7 agents/5 public; public
  directory returns the 5 public agents (all `verified:true`); hub scope returns all 7; `parler
  discover --public` matches; the website renders the cards (headless-Chrome screenshot).
- **Security highlight:** cards are self-signed by the agent's own nkey; the hub stores + verifies
  but cannot forge them — `verified` is independently checkable by any client.

---

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

---

# Task: Contributor-grade test system + resilient CI/CD (2026-06-28)

**User ask:** we'll have many open-source contributors — build a detailed test system that catches
bugs/issues *before* deploying, a resilient CI/CD pipeline, and "anything necessary". Everything we
build must itself be **testable**.

**Design principle:** GitHub Actions YAML is not testable. So all pipeline *logic* lives in small,
composable, self-tested shell scripts under `scripts/ci/`; the workflows are thin wrappers that call
them. A contributor runs `make ci` locally and gets the *same* gates as the cloud. The test system
is itself tested by `scripts/ci/selftest.sh` ("test the test system").

### Plan

- [x] Pin the toolchain — `rust-toolchain.toml` (stable + clippy) so every contributor + CI match.
- [x] Testable pipeline scripts (`scripts/ci/`): `lib.sh` (step runner), `rust.sh`, `web.sh`,
      `audit.sh`, `smoke.sh`, `all.sh`, and `selftest.sh` (the meta-test).
- [x] HTTP smoke **contract test** — `crates/parler-hub/tests/smoke.rs` boots the real hub and
      asserts `/health`, `/api/hub`, `/api/directory`, `/` (dependency-free raw HTTP client).
- [x] Supply chain — `deny.toml` (cargo-deny: vulns + sources blocking, licenses tunable) +
      `.github/dependabot.yml` (cargo / npm / actions).
- [x] Workflows — rewrite `ci.yml` (concurrency, least-priv perms, timeouts, jobs via the scripts +
      lint the pipeline with actionlint/shellcheck), add `deploy.yml` (CD → Fly + post-deploy live
      smoke + auto-rollback, secret-guarded so forks are no-ops) and `audit.yml` (daily CVE scan).
- [x] Contributor scaffolding — `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`,
      `.github/CODEOWNERS`, PR template, issue forms.
- [x] `make ci|selftest|audit|smoke|coverage` + README pointer + `docs/ci-cd.md` (the architecture).
- [x] **Verify** — run `make selftest`, `make ci`, the new smoke test, and `scripts/ci/smoke.sh`
      against a live local hub; prove green.

**How each piece is testable:** scripts → `selftest.sh` (`bash -n`, exec bits, unit-tests `lib.sh`
helpers) + shellcheck in CI · workflows → actionlint + a YAML parse in selftest · smoke contract →
`cargo test -p parler-hub --test smoke` + `scripts/ci/smoke.sh <url>` · deny.toml → `make audit` +
TOML parse in selftest · Dockerfile → hadolint + the real build in `deploy.yml` · whole pipeline →
`make ci` reproduces the cloud.

### Review (done)

**Verified by installing & running the real tools** (shellcheck 0.11, actionlint 1.7.12, cargo-deny
0.19.9), not by reasoning about their output — which caught & fixed **4 genuine bugs**:

1. **Broken rustdoc link** in `crates/parler-auth/src/provision.rs` (`save_space_auth` →
   `strip_space_auth`) — surfaced by the new `cargo doc -D warnings` gate.
2. **shellcheck self-trip**: a comment starting `# shellcheck,` was parsed as a directive
   (SC1072/1073) — would have failed the pipeline job.
3. **Invalid GitHub Actions expression**: `join(needs.*.result, ",")` — GHA expressions only allow
   single-quoted strings, so it had to be `','`. Would have broken the `CI passed` gate at runtime.
4. **`deny.toml`**: cargo-deny 0.19 requires `license-files` in a `[[licenses.clarify]]` block (mine
   omitted it → the whole config failed to parse); the tree also needs `CDLA-Permissive-2.0`
   (webpki-roots) allowed, and the `ring` OpenSSL exception was unneeded. Now licenses pass **strict**.

Final state: `make ci` fully green (selftest 41 · rust build/clippy/test/doc · web · audit), plus
shellcheck/actionlint/cargo-deny all clean. Clean tree on branch `ci-cd-pipeline`; not committed/PR'd.
(Heavy compiles filled the disk to 100% mid-run; reclaimed with `brew cleanup`.)
