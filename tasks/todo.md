# Task: Communication metrics — estimated tokens + activity (hub & session viewer) — 2026-07-03

**User:** "build a feature into the agent hub, and session viewer so that we can keep track of how many
tokens the users' agents have spent communicating within the hub ... better insight ... also add any
other metrics that you think can be useful for the users to know about their agents activity."

**Design:** the hub relays plaintext and is **not** an LLM, so it can't read a model's real tokenizer —
it **estimates** tokens from message text (~4 chars/token, Unicode-scalar counted), always shown as an
estimate ("≈"). This measures the cost of the inter-agent communication the hub actually carries —
exactly "tokens spent communicating within the hub." Store an estimated `tokens` per message (new
column via the existing `add_column_if_missing`, backfilled once) so aggregates are cheap SQL, not a
re-parse per poll. **No new wire frame / protocol type** — purely additive helpers + REST fields.

## Steps
- [x] Explore protocol, hub store/server, session viewer, web types
- [x] `parler-protocol`: `estimate_tokens(&str)` + `estimate_message_tokens(&[Part])` pure helpers + 2 tests
- [x] `store.rs`: migrate `messages.tokens` (+ one-time backfill); `append_message(..., tokens)`;
      `RoomStats`/`AgentStat` + `room_stats(room)`; store test
- [x] `server.rs`: `Metrics.tokens_total`; `estimatedTokensTotal` in `/api/hub`; count+store tokens in
      `Send`; `stats` object in `/api/session`; smoke + e2e assertions
- [x] `web`: `types.ts` (`HubStats`/`HubSummary.stats?`, `SessionStats`/`SessionView.stats`); metrics
      strip in `session-viewer.tsx`; hub-wide tokens in `hub-header.tsx`
- [x] `desktop`: mirrored the same metrics strip + types in the app's session viewer
- [x] `docs/agent-mesh.md`: "Activity metrics" subsection
- [x] Verify: `CI_SKIP_WEB=1 make ci` (rust) + `web` build + desktop typecheck + **live binary round-trip**

## Review — DONE & VERIFIED (2026-07-03) ✅

Agents' **communication cost is now visible** in the hub summary and the session viewer (web + desktop),
led by the headline the user asked for: *how many tokens have my agents spent talking through the hub.*

**Design honesty:** the hub is a **relay, not an LLM**, so it can't know a model's real tokenizer — it
**estimates** at ~4 chars/token (`estimate_message_tokens`, Unicode-scalar counted), computed once at
append time and **stored per message** so every aggregate is a cheap SQL sum instead of a re-parse on
each 4s viewer poll. Everything is labelled `≈` with a footnote. **No new protocol frame/type** — pure
additive helpers + REST fields, so no wire-contract ripple.

**What shipped:**
- **`parler-protocol`** — `estimate_tokens` / `estimate_message_tokens` pure helpers (skip the sig part;
  count text + string leaves of data/extension parts). 2 unit tests.
- **`parler-hub` store** — `messages.tokens` column (in the DDL *and* `add_column_if_missing`, which now
  returns whether it added it → one-time `backfill_message_tokens` for pre-existing rows).
  `append_message` returns `(id, seq, tokens)`. New `room_stats(room)` → totals, activity span, and a
  per-agent breakdown **grouped by display name/role (no agent id leaks)**, chattiest first. Store test.
- **`parler-hub` server** — `Metrics.tokens_total` (bumped from the same estimate the row stored);
  `estimatedTokensTotal` on `/api/hub`; a `stats` object on `/api/session`. Smoke test + the watch e2e
  now assert the new fields (incl. the no-id-leak invariant on the per-agent rows).
- **`web`** — `HubStats`/`SessionStats` types; a metrics strip (est. tokens · messages · tokens/msg ·
  active-for + a "who's talking" per-agent bar chart) above the roster; hub-wide "≈ N tokens relayed"
  on the hub header.
- **`desktop`** — the same strip + types mirrored into the app's session viewer.
- **`docs/agent-mesh.md`** — an "Activity metrics" subsection.

**Verified:**
- `CI_SKIP_WEB=1 make ci` → "all gates passed" (build · clippy -D warnings · test --locked · doc · deny).
- Web `next build` green (all routes prerender; `/hub` 13.2→14.3 kB). Desktop `npm run typecheck` green.
- **Live binary round-trip** (real `parler` CLI ⇄ real `parler-hub`): opened a session, sent 3 messages,
  minted a watch code → `GET /api/session` returned `stats.estimatedTokens: 71`, `messages: 4`, the
  activity span, and a per-agent row for alice (planner) **with no `id` field**; `GET /api/hub` showed
  `estimatedTokensTotal: 82`, `messagesTotal: 4`. Fresh hub reports `estimatedTokensTotal: 0`.

**Not committed** — working tree, ready for review/PR. **Deferred (noted):** desktop Local-Hub stats
panel could also surface `estimatedTokensTotal` (the session viewer, the actual ask, is done); agent-
reported *real* usage would need the MCP host to plumb token counts, which it doesn't expose today.

---

# Task: Full-app audit (arch · security · setup UX) + Wave 1 remediation — 2026-07-03

**User:** "act as a senior engineer and architect, audit my entire app and draft a plan so that I can
use other agents to work on improving it ... full audit ... vulnerabilities ... improve UX, making the
set up so easy and straightforward to use."

**What I did:** three parallel Explore audits (architecture/code-quality, security, setup UX), then
**verified every load-bearing claim against the source** before trusting it. Discarded one false
"critical" — the architecture audit's "panics on network input" (`hub.rs:862/868…`) is `#[test]`
code, not network-reachable. Full plan:
`~/.claude/plans/system-instruction-you-are-working-calm-newt.md`; workstreams filed in
`tasks/backlog.md` (Waves 1–4, "Full-app audit remediation" epic).

**Verdict:** security posture verified **strong** — no critical/high vulns. Confirmed solid:
constant-time join-secret compare, hub/watch token scope wall, fully parameterized SQL, server-side
blob-hash + path safety, seed-never-on-wire, the `Invite` self-join fix, sandboxed Electron
(`contextIsolation`/`execFile`), checksum'd installer. Real work is (1) two narrow hardening fixes,
(2) known scaling ceilings that ship off-by-default, (3) first-run UX confidence ("did it work?").

## Review — Wave 1 DONE & VERIFIED (2026-07-03) ✅
- **[SEC-1] Atomic 0600 secret writes** — new `parler_auth::write_private_file` (temp file created
  `0600` + atomic `rename`) replaces the `write`-then-`set_permissions` window for the nkey seed
  (`config.rs::save`) and the hub join secret (`secret.rs`). No instant where the seed sits at the
  default umask; overwriting an old world-readable config now *replaces* (not widens) it.
- **[SEC-2] Redacting `Debug`** — `Identity` + `ConfigFile` hand-write `Debug` printing
  `seed: "<redacted>"` (the id, a public key, is still shown). `{:?}`/logs can't leak the seed.
- **[UX-1] Installer PATH self-heal** — `install.sh` smoke-tests `parler --version` (catches a
  wrong-arch/truncated download) and, on a PATH miss, prints an exact `>> ~/.zshrc` line for the
  detected shell + a full-path `parler connect` fallback — no more silent "command not found".
- **[UX-5] Detection dead-end** — `parler connect` prints the path it checked ("looked in ~/.codex")
  + `parler connect <id> --print` for non-standard install locations.
- **Tests:** +3 `parler-auth` unit tests (0600-on-create, replace-world-readable-with-0600, `Debug`
  redaction). **Gate:** `CI_SKIP_WEB=1 make ci` → "all gates passed"; `shellcheck scripts/install.sh`
  clean. **Not committed** — left in the working tree for review.

## Review — Waves 2–4 DONE & VERIFIED (2026-07-03) ✅
Continued straight into the rest of the plan (user: "continue implement other waves").

**Wave 2 — first-run confidence:**
- **[UX-2] Reachability probe** — a bare `parler connect` now dials each wired hub once (`probe_hubs`,
  3s timeout) and prints `✓ hub reachable` / `⚠ hub not reachable yet …` (localhost → "start it:
  parler hub --local"); `--verify` still does the deeper wait. Failures surface at wire time, not after.
- **[UX-4] Online visibility** — `parler mcp` announces the freshly minted identity and appends a
  trimmed `~/.parler/mcp.log` (connected / connect-FAILED / auto-register outcome); `parler doctor`
  shows a "Recent MCP activity" tail (`recent_log`). +1 test (`breadcrumb_log_roundtrips_and_trims`).
- **[docs] Signing is flagged-not-rejected** — new note in `docs/discovery.md` security model.

**Wave 3 — scale & resilience:**
- **[ARCH-2] Retention on by default** — `Retention::default()` bounds messages (30d), unkeyed facts
  (500/author-room), idle blobs (14d); `main.rs` maps an explicit `0`/negative to keep-all; new
  `idx_messages_ts` for the age scan; guard test `retention_defaults_are_enabled`.
- **[ARCH-1] Arc fanout** — push channel + `Subscriber` now carry `Arc<ServerFrame>`, so a fan-out to N
  members shares one frame; the `retain` also counts pushes and keeps a `Full` subscriber (cursor
  recovers it). Push e2e green.
- **[ARCH-4] Version echo** — `Challenge` gained an optional `version`; the client warns on a major
  mismatch (`warn_on_protocol_mismatch`). Additive/optional — old hubs and clients unaffected.

**Wave 4 — hardening & observability:**
- **[ARCH-3] parking_lot** — hub/store mutexes are non-poisoning; `.lock()` returns the guard (13 sites).
  Dep was already in-tree, so cargo-deny stays green (recorded the lock edge with one non-`--locked` build).
- **[ARCH-6] Metrics** — `Metrics` counters (connections/messages/pushes-total + live-connection gauge)
  under `/api/hub` `"stats"`; smoke test asserts them.
- **[SEC-3/backlog] Hardened challenge nonce** — `parler-auth:v1:<hub>:<exp>:<rand>` (domain-separated,
  hub-bound via 12-hex `sha256(public_url)`, 60s TTL), validated on `Hello` step 2; zero client change;
  unit test covers expired/foreign/malformed.
- **Deferred on purpose:** W4c god-file split + W3d self-healing reconnect — each wants its own PR (a
  big pure-refactor / a new resilience feature shouldn't ride with this batch).

**Gate:** `CI_SKIP_WEB=1 make ci` → "all gates passed" (build · clippy -D warnings · test --locked ·
cargo-deny). New/updated tests all green. **Not committed** — working tree, ready for review/PR.

---

# Task: Desktop app = same simplicity as the CLI (single source of truth) — 2026-07-01

**User:** "make sure the macOS app has the same simplicity as the CLI and the whole thesis of the
app; make it intuitive and easy to use."

**Thesis (AGENTS.md/README + connect.rs module doc):** setup is ONE command — `parler connect`
auto-detects *every* agent and wires them all; it is "the single source of truth ... the desktop app
shells out to `parler connect --json`." Flagship flow = session handoff.

**Gaps found (app diverges from the CLI/thesis):**
1. Not one click — `ConnectScreen` wires one host at a time; no "connect everything".
2. App re-implements wiring in `mcp.ts` instead of driving `parler connect` → **Codex missing
   entirely** + agents get no per-agent `PARLER_HOME`/`PARLER_NAME` (they collide on one identity).
3. No `parler connect --remove`, so the app kept its own removal path (last straggler).
This is the **documented follow-up** of the 2026-07-01 "One good way to set up Parler" task
("Full CLI-delegation + Codex-in-GUI left as a follow-up (needs ... a connect `remove`/enriched
`--list`)").

**Plan:**
- [ ] CLI: `parler connect --remove [hosts…]` (symmetric unwire) + add `hub` to `--list --json`
      (so the app can classify local/public) + tests. `CI_SKIP_WEB=1 make ci` green.
- [ ] App main `mcp.ts`: thin driver over bundled `parler` — detect (`--list --json`),
      `connect`/`connectAll` (`connect [host] --hub … [--join-secret] --json`), `disconnect`
      (`connect --remove host --json`). Codex + per-agent identity fixed for free.
- [ ] IPC/types/preload: add `agents.connectAll(target)`.
- [ ] `ConnectScreen`: primary one-click **"Connect all detected agents"**; per-host demoted.
- [ ] `onboarding`: "Connect all my agents" (not just Claude Code).
- [ ] Verify: `make ci` (Rust) + desktop `npm run typecheck` + `npm run build`.

## Review — DONE & VERIFIED (2026-07-01) ✅

Made the macOS app's setup **literally the CLI in a button**: the app now shells out to the bundled
`parler connect --json` for all agent wiring, and Connect leads with a one-click "Connect all detected
agents." This closes the documented follow-up and makes the AGENTS.md/connect.rs thesis ("the desktop
app shells out to `parler connect --json`") true for the first time.

**What changed (thesis + simplicity):**
- **One click wires everything.** New `agents.connectAll(target)` runs `parler connect --hub …
  [--join-secret …] --json` (no host args = auto-detect + wire all). ConnectScreen's hero is now
  "Connect all detected agents (N)"; per-host rows demoted to "Or connect one at a time." Onboarding
  step 2 is "Connect your agents" → one "Connect all" button (was Claude-Code-only).
- **Single source of truth.** `desktop/src/main/mcp.ts` went from ~260 lines of re-implemented
  detection/JSON-writing to a thin driver: `detectHosts` → `connect --list --json`,
  `connect`/`connectAll` → `connect [host] --json`, `disconnect` → `connect --remove host --json`.
- **Fixed real divergences for free:** the app now supports **Codex** (was missing entirely) and each
  wired agent gets its own `PARLER_HOME`/`PARLER_NAME` (they used to collide on one unnamed identity).
- **CLI gained the missing inverse:** `parler connect --remove [hosts…]` (symmetric unwire, preserves
  the user's other MCP servers in JSON + TOML), and `--list --json` now reports each host's wired
  `hub` so the app can classify local vs shared. +6 unit tests.

**Files:** Rust — `crates/parler-cli/src/connect.rs` (`--remove`/`run_remove`/`unwire`/`remove_json`/
`remove_toml`/`configured_hub`/`parse_hub_from_text` + `hub` in `--list`), `src/lib.rs` (`--remove`
arg). Desktop — `mcp.ts` (rewrite), `shared/types.ts` + `channels.ts` + `preload/index.ts` +
`main/ipc.ts` (`connectAll` + `hubArgs`), `screens/connect.tsx` + `screens/onboarding.tsx`, README.

**Verified:**
- `CI_SKIP_WEB=1 make ci` → "all gates passed" (build · clippy -D warnings · test --locked · doc ·
  cargo-deny). 12/12 connect unit tests.
- **Live CLI cycle** (isolated `$HOME`): `connect cursor codex --hub ws://127.0.0.1:7071 --json` →
  both `wired`; `--list --json` reports `hub: ws://127.0.0.1:7071` for both; Codex TOML kept the
  user's `# comment` + `[mcp_servers.other]`; `--remove codex --json` → `removed` (parler table gone,
  other kept); second `--remove` → `not-configured` (idempotent).
- Desktop `npm run typecheck` (main + renderer) clean; `npm run build` bundles all three.
- **Not GUI-tested** (needs a display + `npm run build:binaries`): the Electron UI itself — the app
  calls the same bundled binary the live CLI cycle exercised, just renamed `parler-cli`.

---

# Task: A2A interoperability — Agent Card discovery bridge — 2026-07-01

**Why:** Competitor deep-dive found A2A (Google → Linux Foundation, 150+ orgs, v1.0) is the de-facto
agent-discovery standard. Our cards are "A2A-inspired" but we speak zero A2A on the wire, so the A2A
ecosystem can't find a Parler agent. Highest-leverage competitive gap. Ship the discovery half first:
project our already-signed directory cards into A2A AgentCard JSON at the standard well-known location.

**Scope (purely additive at the hub's HTTP edge — no protocol-frame change, no connector/CLI ripple):**
- [x] `docs/a2a-interop.md` — design doc (vision · phase 1 = discovery · phase 2 = message endpoint)
- [x] `GET /.well-known/agent-card.json` — the hub's own A2A entry-point card
- [x] `GET /a2a/directory` — public (hub-scope w/ token) agents as A2A cards
- [x] `GET /a2a/agents/:id` — one agent projected to an A2A card (respects visibility)
- [x] `a2a_card()` projection: Parler card → A2A v0.3 AgentCard, with a `parler` extension carrying the
      verifiable nkey id + native signature (A2A clients ignore unknown fields; Parler-aware clients
      re-verify offline — the "hub can't forge a card" guarantee, carried onto the A2A surface)
- [x] Unit test for the projection; smoke tests for the well-known + directory endpoints
- [x] Link the doc from AGENTS.md + docs/communication.md
- [x] `CI_SKIP_WEB=1 make ci` green (clippy -D warnings is a hard gate)

**Out of scope (documented as phase 2):** inbound A2A `message/send`/`message/stream` (JSON-RPC) →
room post; proper A2A JWS signatures (needs the agent's seed, which lives on the agent, not the hub).

## Review — DONE & VERIFIED (2026-07-01) ✅

Shipped the A2A **discovery bridge**: the hub now projects our already-signed directory cards into
A2A AgentCard JSON at the standard well-known location, so the A2A ecosystem (150+ orgs) can discover
a Parler agent. Purely additive at the hub's HTTP edge — **no `parler-protocol` change**, so zero
ripple into connector/CLI/web.

- **3 routes** (`crates/parler-hub/src/server.rs`): `GET /.well-known/agent-card.json` (hub
  self-card / entry point), `GET /a2a/directory` (agents as A2A cards; `scope=hub` reuses the existing
  directory-token gate), `GET /a2a/agents/:id` (one card; private cards gated like `/api/agents/:id`).
- **`a2a_card()` projection** + proxy-aware `request_base_url()` (honors `X-Forwarded-Proto` for
  Fly/Caddy). Carries a `parler` extension (`id` = Ed25519 pubkey + native `signature` → offline
  re-verifiable), and **deliberately does not fake an A2A JWS `signatures` field** (would need the
  agent's seed).
- **Tests:** 3 unit (`a2a_card_projects_core_and_parler_fields`,
  `a2a_card_synthesizes_a_skill_from_tags_when_none_given`, `request_base_url_is_proxy_aware_and_falls_back`)
  + 2 HTTP smoke (`a2a_well_known_card_is_served`, `a2a_directory_is_a_json_array`).
- **Doc:** `docs/a2a-interop.md` (vision · phase 1 shipped · phase 2 = message endpoint + Task
  lifecycle + team assembly). Linked from AGENTS.md + docs/communication.md.

**Verified:** `CI_SKIP_WEB=1 make ci` → "all gates passed" (build · clippy -D warnings · test --locked
· cargo doc -D warnings · cargo-deny). 45 hub lib tests + 6 smoke tests green. **Live end-to-end** vs
`scripts/seed-demo.sh` (public hub, real signed agents): well-known card is proxy-aware
(`https://parler-hub.fly.dev/a2a/directory`, `publicAgents:5`); `/a2a/directory` returns 5 projected
cards each carrying `parler.id` + `verified:true` + a real `signature`, and **no faked `signatures`
field**; the public↔hub scope split (5 public vs 7 with a directory token) proves private agents are
excluded from the world-readable view; unknown agent → 404.

**Phase 2 (documented, not built):** inbound A2A `message/send`/`message/stream` → room post; the
`Task` lifecycle it maps onto; agent-produced A2A JWS; an outbound A2A client.

---

# Task: One good way to set up Parler — kill the fragmentation — 2026-07-01

**User:** "too technical & hard to set up even for an SE; too cumbersome for little reward. I don't
know when to set up for Claude vs Hermes vs Codex — everything's fragmented. I just need ONE good way
to set it up for all agents. Public vs private hub is also too technical and I'm lost." → implement
all the recommendations, veteran security/network/Rust standard.

**Root causes (grounded in the code):** (1) "zero setup" actually needs the Rust toolchain
(`cargo install --path`); no prebuilt binary. (2) 5 hosts, 5 config locations, only Claude has a
one-liner; Codex/Gemini/Hermes have no automated path anywhere. (3) The only auto-wiring logic lives
in the Electron app (`desktop/src/main/mcp.ts`) and knows just Claude/Cursor/Claude-Desktop — the CLI
has no `connect`. (4) "public vs private hub" forces an infra decision (hubs, join secrets, wss)
before you've said hello.

## 1. `parler connect` — the one command (single source of truth)
- [ ] `crates/parler-cli/src/connect.rs`: host registry + JSON/TOML/claude-cli writers, tests.
- [ ] Auto-detect & wire Claude Code, Codex, Cursor, Windsurf, Gemini, Claude Desktop; per-host
      identity home (`~/.parler/agents/<id>`) assigned automatically (kills PARLER_HOME juggling).
- [ ] Hub *ladder*, not a fork: default **shared**; `--local` (loopback, nothing leaves); `--team`
      (generates a join secret, prints teammate line). Reuse `parler_hub::random_secret()`.
- [ ] Unknown host (`parler connect hermes`) + `--print` → portable paste-snippet. `--json` (desktop),
      `--list`. Idempotent writes that never clobber other MCP servers.
- [ ] Honest one-line confidentiality note; `parler hub --local` sugar for a clean instruction.

## 2. A real install (no Rust toolchain)
- [ ] `scripts/install.sh` (curl | sh) + `.github/workflows/release-cli.yml` (macOS arm64/x64 + Linux)
      + `packaging/homebrew/parler.rb`.

## 3. Reframe docs — README Quickstart → install + `parler connect` + hub ladder; delete Options A/B/C.

## 4. Desktop = same code path — `mcp.ts` delegates to bundled `parler connect --json` (Codex/Gemini free).

## Gate: `CI_SKIP_WEB=1 make ci` green (build·clippy -D warnings·test·doc·deny). Never `cargo fmt`.

## Review — DONE & VERIFIED (2026-07-01) ✅

Collapsed setup from "install Rust → hand-edit per-agent configs → choose public/private hub" to
**install once → `parler connect`**, and reframed the hub model as a ladder with a default.

- **`parler connect`** (`crates/parler-cli/src/connect.rs`, +8 unit tests) — one command auto-detects
  and wires Claude Code, Codex, Cursor, Windsurf, Gemini, Claude Desktop; unknown host (`connect
  hermes`) / `--print` → portable snippet; `--list`, `--json` (desktop). **Idempotent, never clobbers
  other MCP servers** (JSON merge + format-preserving `toml_edit` for Codex), `0600` on every file we
  author, absolute exe path baked in, per-host identity home (`~/.parler/agents/<id>`). Hub ladder:
  default **shared** → `--local` (loopback, nothing leaves; `parler hub --local` sugar added) →
  `--team` (mints + prints a join secret via `parler_hub::random_secret`, detects LAN IP). Added
  `--join-secret`/`--hub` (fills a real gap: a secret-gated hub had no client-side secret flag).
- **Real install** — `scripts/install.sh` (checksum-verified `curl|sh`, no Rust), `release-cli.yml`
  (macOS arm64/x64 + Linux x64, uploads to the Release), `packaging/homebrew/parler.rb`.
- **Docs + website** — README Quickstart → two-line install + `parler connect` + the ladder table
  (deleted Options A/B/C); reframed Connect + env-var sections; security caveat leads with one plain
  sentence; AGENTS.md + docs/agent-mesh.md; website examples/download/faq + both blog components/mirrors.
- **Desktop** — extended detection+wiring to Windsurf + Gemini (safe data addition to the existing
  JSON path); README notes `parler connect` is the shared brain. *Full CLI-delegation + Codex-in-GUI
  left as a follow-up (needs GUI smoke test + a connect `remove`/enriched `--list`).*

**Verified:** `CI_SKIP_WEB=1 make ci` → "all gates passed" (build·clippy -D warnings·test·doc·
cargo-deny incl. new `toml_edit`); 8/8 connect tests; live binary smoke — `--list`, `--print`, real
Codex TOML write (twice: idempotent, `# comment` + `model` + `[mcp_servers.other]` preserved, clean
`[mcp_servers.parler]`, `0600`), `--local`/`--team` messaging (secret + LAN teammate line); `cd web &&
npm run build` green. **Not GUI-tested:** the desktop app (Electron; change is a typed data addition).

---

# Task: Parler Desktop — SIMPLIFY for 10/10 UX (declutter) — 2026-06-30

**User:** too many features/buttons; clunky & cluttered. Simplify as much as possible, still intuitive.

**Model:** *the app IS your local hub.* It auto-runs in the background; you connect agents to it and
watch them. Public hub becomes an advanced option inside Connect, not a global axis.

**Cuts (6 screens → 3 + gear):**
- Nav = **Agents · Connect · Sessions** + a Settings **gear** in the footer. Remove **Dashboard**
  (redundant) and **Local Hub** from the nav (fold into Settings → "Manage local hub", still there for
  power users). Remove the **global Local/Public titlebar switch** (app = local hub).
- **Agents** (home): your hub's agents show with **zero friction** — app auto-mints a directory token
  (`parler token`) so the private hub's full roster is visible without a paste. Just search + cards.
  Drop scope toggle, sort, grid/list toggle, tag facets, token-gate.
- **Connect**: local by default; "public hub" demoted to a small advanced toggle. One action per host.
  Manual snippet collapsed under "Other MCP hosts".
- **Sessions**: keep watch viewer but **remove Timeline Replay** (play/scrub/speed buttons) — chat
  only. "Open a session" = recap + Open; topic/no-approval under a small "Options" disclosure.
- **Settings**: minimal + one collapsible "Local hub (advanced)" (start/stop, mode, port, secret,
  logs, data folder). Drop the default-connect-target setting.
- **Onboarding**: 2 steps (Welcome → Connect first agent to the auto-started local hub). Drop hub-choice.
- **Titlebar**: clean drag bar + tiny status pill → Settings.

**Files:** main: +`parler token` mint + `hub.directoryToken` IPC (cached). renderer: rewrite
`session-viewer` (chat-only), simplify `directory`/`connect`/`sessions`/`settings`/`onboarding`/
`sidebar`/`titlebar`/`App`; delete `dashboard`, replace `directory-screen`→`agents-screen`.

**Verify:** typecheck + build + headless boot clean; `dist` DMG launches. Then update PR.

## Review — DONE & VERIFIED (2026-06-30) ✅
Halved the surface (6 nav destinations → **3 + a Settings gear**) and cut per-screen clutter, around
one model: *the app is your local hub; connect agents and watch them.*
- **Removed:** Dashboard, Local Hub from nav (now Settings → "Manage"), the global Local/Public
  titlebar switch, and the Session viewer's **Timeline Replay** (play/scrub/speed). Nav = Agents ·
  Connect · Sessions + gear.
- **Agents (home):** your hub's full private roster shows with **zero paste** — app auto-mints a
  directory token (`parler token`) and reads `scope=hub`; falls back to public if the hub's down.
  Just search + status chips; dropped scope/sort/grid-list/tag-facets/token-gate. Friendly empty state.
- **Connect:** local by default, public demoted to a one-line link; one action per host; manual
  snippet collapsed. **Sessions:** recap + Open (topic/approval under "Options"); chat-only viewer.
  **Settings:** trimmed; hub advanced kept behind "Manage". **Onboarding:** 3 steps → 2.
- **Main:** added `hub.directoryToken` IPC (cached, cleared when hub leaves `running`) + `parler token`
  driver; hardened event forwarding against a torn-down window (fixed an `Object has been destroyed`
  teardown rejection).
- **Verified:** typecheck + build clean; headless boot loads renderer with **no console errors and no
  rejection**; in-app hub smoke → running/healthy/api 200; **directory-token smoke** → `parler token`
  parsed, `/api/directory?scope=hub` 401 without / 200 + roster with; `dist` DMG rebuilt, packaged app
  launches clean. Deleted `dashboard.tsx` + `directory-screen.tsx`.

## Review — (superseded by the section above)

---

# Task: Parler Desktop — macOS Electron app (hub + directory + session viewer) — 2026-06-30

**User ask:** a downloadable macOS app that (1) serves everything the website does (agent
**directory** + **session viewer**), (2) can run a full **private hub locally** (WS bus + SQLite DB +
blobs) with one toggle, (3) makes **connecting an agent** (Claude Code / Cursor / any MCP host) a
one-click action against either the local hub or the public hub, (4) matches the website's dark
"Resend obsidian terminal" theme exactly, and (5) streamlines setup: download → connect an agent in
under a minute. Later: a "Download for macOS" button on the website.

## Key architectural decisions (confirm before build)
1. **Location:** new self-contained `desktop/` project at repo root (sibling to `web/`, `crates/`).
2. **Renderer stack:** Electron + **Vite + React 19 + TS + Tailwind v4**, reusing the exact `@theme`
   tokens + fonts from `web/app/globals.css` (fonts bundled locally for offline). Port the key
   components (SessionViewer w/ chat+timeline replay, directory, agent-card, status-dot, hub-header,
   ui/*). *Not* embedding the Next.js SSR app — a native SPA fits IPC + offline + OS integration.
3. **Native binaries:** bundle compiled `parler-hub` + `parler` as electron-builder `extraResources`;
   `desktop/scripts/build-binaries.sh` cargo-builds them per mac arch. Main process spawns/supervises
   the hub and shells out to `parler` for privileged actions.
4. **Distribution:** electron-builder → DMG (arm64 + x64). Signing/notarization stubbed + documented
   (needs Apple Developer ID); unsigned works locally with a quarantine note.

## Main process (Electron)
- [ ] Hub supervisor — spawn bundled `parler-hub` (free port, default 7071), persistent
      `userData/hub.sqlite` + `.blobs`, name `${user}'s Hub`, optional `--public`, auto-generated
      persisted join secret via `--join-secret-file`. Health-poll `/health`; restart w/ backoff;
      graceful SIGTERM on quit; stream stdout/stderr to a log buffer.
- [ ] MCP host integration — detect `claude` CLI (+ Cursor config path); one-click
      `claude mcp add parler -- <bundled parler> mcp` with `PARLER_HUB` (local|public) +
      `PARLER_JOIN_SECRET` when private; render snippets + JSON for other hosts; detect connected state.
- [ ] Privileged actions via bundled `parler` (open session, mint watch, approvals, whoami).
- [ ] Typed IPC (`preload.ts`, contextIsolation on): hub start/stop/status/url, mcp list/connect,
      session open/mint-watch, clipboard, openExternal, settings get/set.
- [ ] Tray/menu-bar item w/ live hub status + quick toggle. Settings store (JSON in userData).

## Renderer (screens)
- [ ] Onboarding (first run): welcome → private-hub vs public-hub → connect first agent (1 click).
- [ ] Dashboard: hub status card (up/down, URL, agents, DB size, uptime) + quick actions.
- [ ] Local Hub: start/stop, live `/api/hub` stats, DB path+size, blob usage, live logs, connect
      snippet, public/private toggle, reveal/rotate join secret, open data folder.
- [ ] Directory: ported agent directory (public + hub scope), search/filter tag/skill/status.
- [ ] Sessions: (a) Watch viewer (chat + timeline replay, ported); (b) Your sessions — open, show key,
      mint watch code, manage join approvals.
- [ ] Connect Agents: target picker (local|public), detected hosts, one-click connect, snippets.
- [ ] Settings: auto-start hub, hub config, about/version (theme locked dark).

## Packaging & website
- [ ] electron-builder DMG (arm64+x64) + `build-binaries.sh`; signing/notarization config + docs.
- [ ] `.gitignore` desktop `node_modules/dist/release`; keep `make ci` green (no heavy new gate).
- [ ] Website "Download for macOS" button → GitHub Release DMG (wiring; release upload needs CI).

## Delivery phases (each runnable)
1. **Foundation** — scaffold, theme port, window chrome, dashboard shell, hub supervisor + health.
2. **Features** — directory, session watch viewer, connect-agents wizard, hub controls+logs+settings,
   onboarding.
3. **Packaging** — DMG build, binary build script, signing docs, website download button.

## Review — DONE & VERIFIED (2026-06-30) ✅

Shipped the full app in new `desktop/` (Electron + electron-vite + Vite/React 19/Tailwind v4),
plus website download CTAs. **User chose: ship unsigned for now, full-fledged all phases.**

**Main process (`src/main/`)** — `HubSupervisor` (spawns bundled `parler-hub`: free-port pick from
7071, persistent `userData/hub.sqlite` + `.blobs`, `--join-secret-file`, health poll, crash-restart
w/ cap, log ring, graceful SIGTERM); `mcp.ts` (detect + one-click Claude Code via `claude mcp add`,
config-merge for Cursor/Claude Desktop w/ backup, GUI-PATH resolution); `parler-cli.ts` (drives
bundled `parler` with an **isolated** `userData/parler-home` identity — open session / mint watch /
whoami, output parsed by regex); typed IPC + `preload` (`contextIsolation` on, prod-only CSP header);
`tray.ts` (menu-bar status + start/stop/quit); settings JSON.

**Renderer (`src/renderer/`)** — theme ported 1:1 from `web/app/globals.css` (fonts via `@fontsource`,
offline); screens: Onboarding (3-step), Dashboard, Local Hub (start/stop, live stats, storage, streaming
logs, secret reveal, public/private toggle, port), Directory (faceted, ported AgentCard/Detail),
Sessions (open + full chat/timeline-replay viewer, ported), Connect (target picker + host wiring +
snippets), Settings. Frameless titlebar w/ global Local⇄Public target switch + hub pill.

**Packaging** — electron-builder DMG (arm64), unsigned (ad-hoc/linker-signed → launches on Apple
Silicon), bundles `parler`+`parler-hub`+tray icons as extraResources; generated icons
(`scripts/gen-icons.mjs`), `scripts/build-binaries.sh`, README w/ signing/notarize docs +
`xattr -dr com.apple.quarantine` note.

**Website** — `MAC_DOWNLOAD_URL` in `lib/seo.ts`; hero "Download for macOS" + subline; nav "Download";
new `components/download.tsx` section (`#download`) with on-brand faux-window preview. `next build` green.

**Verification (all green):**
- `npm run typecheck` (main+renderer) clean; `npm run build` bundles main/preload/renderer; fonts offline.
- Headless Electron boot: renderer loads, **zero console errors**, self-quits (env-gated smoke hooks).
- **Full-path hub smoke inside Electron**: supervisor spawned the bundled hub → `phase=running
  healthy=true` → `/api/hub` 200 ("tamnguyen's Hub", private) → clean stop; created hub.sqlite+blobs.
- **Binary/CLI integration smoke** vs real binaries: hub `--join-secret-file` boot, `parler init` w/
  secret, `session open`→parsed room+KEY, `session watch`→parsed token, `/api/session` bearer 200 +
  content, bad token 401, `whoami` parsed.
- `npm run dist` → `release/Parler-0.1.0-arm64.dmg` (102 MB); packaged `.app` bundles both binaries +
  icns + tray png; packaged app **launches clean** headlessly.
- `desktop/` is invisible to `make ci` (not a cargo workspace member, not `web/`), so CI is unaffected.

**Follow-ups (noted, not blocking):** Apple Developer ID signing+notarization (config stubbed, flip
when creds exist); universal/x64 arch (script supports a target triple, needs `rustup target add`);
CI job to build+upload the DMG to a GitHub Release so `MAC_DOWNLOAD_URL` resolves to a real asset;
optional in-app "mint directory token" for one-click private hub-scope directory viewing.

---

# Task: Standalone full-screen Agents Console page (web) — 2026-06-29

**User ask:** from the website, build an *extra standalone page* for the agents hub; on that page add
*more agent-focused features* and make the *existing agents features (the directory) occupy most of
the screen*.

## Design — **Option A** (user-chosen): one `/hub` page, two tabs (Agents + Sessions)
Build on the existing REST surface only (`/api/hub`, `/api/directory`, `/api/session`). Reuse
`AgentCard`, `AgentDetail`, `TokenDialog`, `StatusDot`, design tokens. Agents tab uses a faceted-
search model: fetch the scope+query set once, then filter **status + tags client-side** so all the
live counts stay coherent. Sessions tab = "session hub" = the sessions explainer + the watch viewer.

New:
- [x] `components/agents-console.tsx` — full-width (`max-w-[1600px]`) console: sticky left filter rail
      (scope · status facets w/ counts · tag facets w/ counts · token) + dominant main column.
      New features vs. home Directory: headline live metrics (agents · online · public · verified),
      **sort** (recent/name/status), **grid⇄list toggle**, **"Live activity"** strip, up-to-4-col grid.
- [x] `components/sessions-feature.tsx` — extracted from home `Sessions()` (`showViewerCta` prop).
- [x] `components/session-viewer.tsx` — extracted watch viewer from `app/session/page.tsx`.
- [x] `components/session-hub.tsx` — Sessions tab = `<SessionsFeature/>` + `<SessionViewer/>`.
- [x] `app/hub/page.tsx` — standalone tabbed page (hash-synced: `/hub` agents, `/hub#sessions`).

Modify:
- [x] `app/page.tsx` — use `<SessionsFeature/>`; prune now-unused imports.
- [x] `app/session/page.tsx` — client redirect → `/hub#sessions` (carry any `&k=` watch token).
- [x] `nav-bar.tsx` — add "Hub" link + repoint CTA + session-viewer link to `/hub`.
- [x] `directory.tsx` + `hero.tsx` (home) — link out to `/hub`.
- [x] Verify: `cd web && npm run build && npm run lint` green; grep no stale `/session` links.

## Review — DONE (2026-06-29) ✅ `next build` green (9 routes prerender, /hub 13.2 kB)
Shipped **Option A**: a standalone `/hub` page with **Agents** + **Sessions** tabs, additive (home
page and REST surface untouched — no hub/protocol change).
- **Agents tab** (`components/agents-console.tsx`): full-width `max-w-[1600px]` console so the directory
  dominates the viewport. Sticky left rail (scope · status facets w/ live counts · tag facets w/ counts
  · token) + a main column with: headline metrics (agents · online · public · verified), a **Live
  activity** strip (working/waiting agents + their `activity`), **search**, **sort** (recent/name/
  status), **grid⇄list toggle**, up-to-4-col grid, and a scannable list view. Faceted-search model:
  fetch the scope+query set once, facet status/tags client-side so every count stays coherent.
- **Sessions tab = "session hub"** (`components/session-hub.tsx`): the sessions explainer
  (`sessions-feature.tsx`, extracted from the home `Sessions()`) + the watch viewer
  (`session-viewer.tsx`, extracted from the old `/session` page) on one screen — exactly the requested
  "combine Session viewer with session."
- **Routing/wiring:** `app/hub/page.tsx` (hash-synced tabs: `/hub`, `/hub#sessions`, deep-link
  `/hub#sessions&k=<token>` opens the viewer pre-connected). Old `/session` → client redirect carrying
  the watch token. NavBar gains "Hub" + repoints the CTA; home Directory + Hero link out to `/hub`.
  Viewer hash writes use `replaceState` so tab switches never scroll-jump to the `#sessions` anchor.
- **Verified:** `npm ci && npm run build` clean (type-check passes, no orphan imports); `next start`
  smoke — `/hub` 200 (both tabs render), `/session` 200 (redirect copy), `/` 200; grep shows no stale
  `/session` links.

---

# Task: SEO — make the Parler website discoverable — 2026-06-29

**User ask:** "how to improve SEO for my website to make it more discoverable?" → plan + implement.

## Findings (current state of `web/`)
- Next 15 App Router. Root `layout.tsx` sets only `title` + `description` + `metadataBase`
  (`https://parler-hub.fly.dev`). No OG, no Twitter card, no og:image.
- Blog `[slug]` has `generateMetadata` with `openGraph` but no Twitter card, no canonical, no
  article metadata, no JSON-LD.
- No `sitemap.xml`, no `robots.txt`.
- No structured data anywhere (we have a full FAQ component + an Article — both free rich-result
  wins).
- `/session` is a dynamic, thin, auth-gated viewer page that is currently indexable.

## Plan
- [ ] `web/lib/seo.ts` — single source of truth: `SITE_URL`, site name/description, and the
      `WebSite` + `SoftwareApplication` JSON-LD objects.
- [ ] `web/app/robots.ts` — allow all, declare sitemap, disallow `/session`.
- [ ] `web/app/sitemap.ts` — `/`, `/blog`, and every post from `POSTS` (lastModified = post.date).
- [ ] `web/app/opengraph-image.tsx` — dynamic on-brand 1200×630 OG image (next/og, default font).
- [ ] `web/app/twitter-image.tsx` — re-export the OG image so Twitter gets a card image too.
- [ ] `web/app/layout.tsx` — expand root metadata (openGraph, twitter `summary_large_image`,
      canonical, keywords, authors/creator) + inject WebSite/SoftwareApplication JSON-LD.
- [ ] `web/app/blog/[slug]/page.tsx` — add Twitter card, canonical, article publishedTime/authors;
      inject `BlogPosting` JSON-LD.
- [ ] `web/app/blog/page.tsx` — add openGraph + canonical to the index.
- [ ] `web/components/faq.tsx` — add plain-text answers + emit `FAQPage` JSON-LD.
- [ ] `web/app/session/layout.tsx` — server layout exporting `robots: { index: false }` (page is a
      client component, so it can't export metadata itself).

## Verify
- [ ] `npm run build` in `web/` is green (renders the dynamic OG image, validates metadata).
- [ ] Spot-check generated routes for `/sitemap.xml`, `/robots.txt`, og image.

## Review
Done. `npm run build` green; new routes `/sitemap.xml`, `/robots.txt`, `/opengraph-image`,
`/twitter-image` all prerender. Verified in the built HTML:
- Homepage: canonical + full OG + `twitter:summary_large_image` + auto-injected OG/Twitter image;
  JSON-LD `WebSite` + `SoftwareApplication` + `FAQPage` (Q/A) present.
- Blog post: canonical, Twitter card, `BlogPosting` JSON-LD.
- `robots.txt`: allow `/`, disallow `/session`, sitemap + host declared.
- `/session`: `<meta name="robots" content="noindex, nofollow">`.

New files: `lib/seo.ts`, `app/robots.ts`, `app/sitemap.ts`, `app/opengraph-image.tsx`,
`app/twitter-image.tsx`, `app/session/layout.tsx`.
Edited: `app/layout.tsx`, `app/blog/page.tsx`, `app/blog/[slug]/page.tsx`, `components/faq.tsx`.

Not done (off-page / content — out of code scope): submit sitemap to Google Search Console + Bing,
write more blog posts, earn inbound links, move to a real domain (vs `*.fly.dev`).

---

# Task: Seamless private hub — "docker compose up, agents talk in no time" — 2026-06-29

**User ask:** make the **private** (self-hosted) hub as easy to stand up as the public hub. "As easy
as running a docker to the database, and the agents can just talk to each other in no time." Goal is
adoption — setup must be one command on the operator side and a copy-paste snippet on the agent side,
**symmetric with the public hub** and **without weakening the security model**.

## Today's asymmetry (the gap)
- Public hub onboarding = `claude mcp add parler -- parler mcp` (URL baked in; MCP self-bootstraps).
- Private hub: `deploy/` is titled "Deploy the **public** hub"; both recipes (Fly, VPS+Caddy) assume
  public + a domain + TLS. "Private" is a one-line footnote ("drop `--public`"). There is **no**
  one-command private recipe, and the runtime image is **distroless (no shell)** so a wrapper script
  can't generate a secret. A LAN-reachable private hub *should* set a join secret (security invariant),
  but inventing + distributing one by hand is friction.

## North-star experience (symmetric, one command each side)
```
# Operator, one box:
docker compose -f deploy/private/docker-compose.yml up -d
#   → boot log prints the exact connect line, with the auto-generated secret:
#     PARLER_HUB=ws://localhost:7070 PARLER_JOIN_SECRET=<gen> claude mcp add parler -- parler mcp
# Each agent:
PARLER_HUB=ws://<host>:7070 PARLER_JOIN_SECRET=<gen> claude mcp add parler -- parler mcp
```

## Design decisions
- **Auto-generated, persistent join secret via a file** (the key enabler). New flag
  `--join-secret-file` / env `PARLER_HUB_JOIN_SECRET_FILE`: read the secret from the file; if absent,
  generate a strong one (reuse the hub's existing token generator), persist it `0600`, reuse on later
  boots. Precedence: explicit `--join-secret` value > file > none. **Binary default is unchanged**
  (no secret unless asked) — this is opt-in and only the private compose sets it. Solves seamless +
  secure-by-default + distroless (no shell needed) in one small, testable helper.
- **Mode-aware landing page + boot banner.** The boot banner (stdout = operator-only) prints the
  ready-to-paste connect line *with the real secret*. The `GET /` page is world-reachable, so it must
  **never print the secret** — for a private hub it shows the snippet with a `PARLER_JOIN_SECRET=<your-
  join-secret>` placeholder and points the operator at the boot log / secret file. Map a `0.0.0.0`
  bind → `localhost` for display so the snippet is copy-pasteable on the common same-machine case.
- **`deploy/private/`** — hub-only compose (no Caddy/domain/TLS), private mode, `7070:7070`, named
  volume, `PARLER_HUB_JOIN_SECRET_FILE=/data/join-secret`. Reuses `deploy/Dockerfile`.
- **Out of scope:** `web/` (private directory viewing already works via tokens); a prebuilt GHCR image
  (truest `docker run`, but touches release/CD + registry namespace — offer as a follow-up).

## Steps
- [x] Hub lib: `secret::resolve_join_secret` + `random_secret` (generate-if-absent, persist `0600`,
      reuse). 6 unit tests.
- [x] `main.rs`: `--join-secret-file` arg; precedence (explicit > file > none); private connect banner.
- [x] `server.rs`: `landing_html(requires_secret)` — private copy + `PARLER_JOIN_SECRET=<placeholder>`
      (structurally can't leak the real secret); `0.0.0.0`/`[::]`→`localhost` in `display_hub_url`. Tests.
- [x] `deploy/private/docker-compose.yml` (hub-only, `command: []` ⇒ private, secret-file) + README.
- [x] Docs: README "Option C"; reframed `deploy/README.md`; AGENTS pointer row.
- [x] `CI_SKIP_WEB=1 make ci` green; booted the real binary twice (generate→persist `0600`→reuse +
      banner with the live secret); compose resolves to `command: []`; public compose still `--public`.

## Review
**Done & verified.** Private-hub onboarding is now symmetric with the public hub: one command on the
box, one copy-paste line per agent — and the hub hands you that exact line.

- **Operator:** `docker compose -f deploy/private/docker-compose.yml up -d --build`. Boots PRIVATE,
  auto-generates + persists a join secret (`/data/join-secret`, `0600`, stable across restarts), and
  prints `PARLER_HUB=… PARLER_JOIN_SECRET=… claude mcp add parler -- parler mcp` in its log.
- **Agent:** paste that line. (`parler mcp` already self-bootstraps; client already reads
  `PARLER_JOIN_SECRET`.) Nothing else.
- **Security held / strengthened:** the world-reachable `GET /` never receives the secret (no param —
  shows a placeholder + "find it in the boot log"); the real secret only hits operator stdout/logs +
  the `0600` file. Private hubs now require a secret by default (was an open "drop --public" footnote).
- **Minimal blast radius:** binary default unchanged (feature is opt-in via `--join-secret-file`); no
  new runtime deps (tempfile is dev-only, already in-workspace); reused the shared Dockerfile + landing
  template. `parler-protocol` untouched, so no cross-crate ripple.

**Verification:** `CI_SKIP_WEB=1 make ci` → "all gates passed"; live binary proof (boot1 generated
`Pd9TW…RTgV`, persisted `0600`; boot2 reused the identical secret); `docker compose config` confirms
private=`command:[]`, public=`command:[--public]`.

**Follow-up SHIPPED — prebuilt GHCR image (`docker run …` in seconds, no compile):**
- `.github/workflows/release-image.yml` — multi-arch (amd64+arm64) build→push to
  `ghcr.io/<owner>/parler-hub` on a `v*` tag or manual dispatch. **No secrets, fork-safe** (pushes to
  the runner's own lowercased namespace via `GITHUB_TOKEN` + `packages: write`); tags via
  `docker/metadata-action` (`latest` / semver / `MAJOR.MINOR` / short-SHA). actionlint + selftest clean.
- **Made the image private-by-default** (the right posture for a published image — a bare `docker run`
  must not open a world-joinable hub). `deploy/Dockerfile` `CMD ["--public"]`→`CMD []`; default name
  →"Parler Hub". Kept the live Fly hub public **safely** via the *existing* `PARLER_HUB_PUBLIC` env
  (added `PARLER_HUB_PUBLIC = "true"` to `fly.toml` — verified `=true`→public, bare→private, `--public`
  arg→public on the real binary). Public compose unaffected (already passes `--public` explicitly).
- `deploy/private/docker-compose.yml` now `image: ghcr.io/tamdogood/parler-hub:latest` + `build:`
  fallback (`--build` from a clone). README/deploy/private + docs/ci-cd.md document the `docker run`
  path. Both composes verified via `docker compose config` (private=`command:[]`+secret, public=`--public`).
- Caveat: Docker daemon was down locally so the *image build* itself runs in CI; the Dockerfile delta
  is only the `CMD`/`ENV` lines (build otherwise identical to the proven Fly build) and the binary's
  mode selection is directly proven. `CI_SKIP_WEB=1 make ci` green.

---

# Task: SEO pass — apply the `astro-seo` skill's principles to the Next.js site — 2026-06-30

**User ask:** install `fusengine/agents astro-seo` via skillfish, then "apply this skill to improve SEO
for my website with your best effort." The skill is Astro-specific; the site is Next 15 App Router, so
we apply its *principles* (canonical correctness, RSS, sitemap, BreadcrumbList, feed autodiscovery,
XSS-safe JSON-LD). Existing SEO (PR #55/#56) is already strong (FAQPage/BlogPosting/OG+Twitter/sitemap/
robots), so this is a targeted improvement pass.

## Real bug found
- `/hub` (a `"use client"` page with no metadata) inherited the root layout's `alternates:{canonical:"/"}`
  → the standalone Hub self-reported as a duplicate of `/` and reused the home title/description.

## Plan
- [x] `lib/seo.ts` — `RSS_URL` + `ALT_RSS` feed-autodiscovery constant.
- [x] `app/layout.tsx` — drop root `canonical:"/"` (footgun: every un-overriding route inherited it);
      set site-wide `alternates.types` (RSS).
- [x] `app/page.tsx` — own `metadata` w/ `canonical:"/"` + RSS type.
- [x] `app/hub/layout.tsx` — NEW server layout: hub title/description/canonical `/hub`/OG/Twitter.
- [x] `app/sitemap.ts` — add `/hub`.
- [x] `app/blog/rss.xml/route.ts` — NEW static RSS 2.0 feed (XML-escaped, categories, atom:self).
- [x] `app/blog/page.tsx` — RSS alternate + `Blog` + `BreadcrumbList` JSON-LD.
- [x] `app/blog/[slug]/page.tsx` — RSS alternate + `BreadcrumbList` JSON-LD.
- [x] `components/footer.tsx` — RSS link.

## Verify
- [x] `npm run build` green (15 routes prerender; `/blog/rss.xml` + `/hub` both static).
- [x] Per-route canonicals correct: `/`→`/`, `/hub`→`/hub` (was `/` — the bug), `/blog`→`/blog`,
      post→own URL. `/hub` `<title>`/`og:title` now hub-specific, distinct from home.
- [x] `/blog/rss.xml` well-formed (`xmllint --noout` ✓): escaped titles/deks, categories,
      `atom:self`, RFC-822 dates. RSS `<link rel=alternate>` on home + blog pages; footer link.
- [x] `BreadcrumbList` JSON-LD on blog post + index; `Blog` collection JSON-LD on index.
- [x] Sitemap now lists `/hub`. `/session` still `noindex`; robots.txt unchanged.
- [x] Web CI gate = `scripts/ci/web.sh` (`npm ci` + `next build`); no `next lint` (no ESLint config).

## Review
**Done & verified.** Applied the `astro-seo` skill's *principles* to the Next.js site (skill is
Astro-only, so no Astro code — the checklist transferred: canonical correctness, RSS, sitemap,
BreadcrumbList, feed autodiscovery, XSS-safe JSON-LD via `dangerouslySetInnerHTML`+`JSON.stringify`).

- **Fixed a real canonical bug:** `/hub` (client page, no metadata) inherited the root layout's
  `canonical:"/"` and the home title/description — it self-reported as a duplicate of the homepage.
  Moved the home canonical off the root onto `app/page.tsx`, and gave `/hub` its own server
  `layout.tsx` (title/description/canonical/OG). Root now only advertises the feed site-wide, so no
  route inherits a wrong canonical.
- **Added an RSS 2.0 feed** (`/blog/rss.xml`, `force-static`) with autodiscovery `<link>`s + footer
  link. **Added BreadcrumbList** (posts + index) and a **Blog** collection schema. **Added `/hub`**
  to the sitemap.
- **Minimal blast radius:** `web/` only, no protocol/crate change; existing SEO (FAQPage, BlogPosting,
  OG/Twitter images, keywords) untouched.

New: `app/hub/layout.tsx`, `app/blog/rss.xml/route.ts`. Edited: `lib/seo.ts`, `app/layout.tsx`,
`app/page.tsx`, `app/sitemap.ts`, `app/blog/page.tsx`, `app/blog/[slug]/page.tsx`, `components/footer.tsx`.

Still off-page / out of code scope (same as the 2026-06-29 SEO task): submit sitemap to Google Search
Console + Bing, earn inbound links, a real domain vs `*.fly.dev`, more posts. Nice-to-have not done:
`Organization` logo node (no dedicated square-raster logo asset yet).

### Further pass ("anything else?") — DONE & verified
Recon showed the blog covers are a poor social-card source (aspect 1.14–2.36:1, none = OG's 1.91:1;
raw PNGs up to 3200px / ~400 KB via plain `<img>`), and there was no theme-color/manifest at all.
- **Per-post branded OG + Twitter cards** — `app/blog/[slug]/opengraph-image.tsx` (+ `twitter-image.tsx`
  re-export), 1200×630, title + dek on the root card's aesthetic, next/og default font. Both
  **prerender static** (`generateStaticParams`) so crawlers get a cached image. Dropped the manual
  `images:[post.cover]` from the post's `generateMetadata` so the branded card is the social image;
  the cover stays as the in-page hero + `BlogPosting` `image`. **Visually verified** the rendered PNG.
- **theme-color + web manifest** — `viewport` export (`themeColor:#000`, `colorScheme:dark`) →
  `<meta name=theme-color>`; `app/manifest.ts` → `/manifest.webmanifest` (Next auto-links it).
- **Image sitemap** — blog entries now carry `<image:loc>` (cover) for Google Images.
`next build` green (18 routes prerender). Verified in output: post `og:image`/`twitter:image` → the
branded `/blog/<slug>/opengraph-image` card; `theme-color` + `rel=manifest` present; manifest valid;
sitemap `<image:loc>` present.

Offered, not done (need a judgment call / visual QA): convert covers to `next/image` (Core Web Vitals —
they're 92–388 KB raw PNGs; touches rendering so wants visual QA); `Organization`/`publisher.logo`
(needs a light-bg square logo asset); AI-crawler policy in robots (a product decision).

---

## 2026-06-30 — Desktop app "melts the Mac" on DMG install (crash-restart storm)

**Report:** clicking install via the DMG "created a loop", the Mac turned hot and had to be hard
powered off. `npm run dev` works fine; only the packaged DMG loops.

**Root cause (`desktop/src/main/hub-supervisor.ts`):** an *unbounded* hub restart loop. The restart
counter was reset to 0 on **every** `/health` success (old line 164 `this.restarts = 0`), while the
cap was only checked against that counter. So a hub that becomes healthy → dies → repeats resets its
own budget every cycle and respawns **forever** (~every 800 ms). Each respawn is a full Rust process
(SQLite open, migrations, FTS, sqlite-vec, WS bind) → CPU pegged → fans max → stall. Dev runs one
instance so the hub stays up and never trips; packaging is where a hub flaps — a **second app
instance** (no single-instance lock) spawning a competing hub over the **same SQLite DB** is the most
likely trigger, and a quarantined binary can flap too.

**Fix (3 changes, main-process only):**
- [x] New `desktop/src/main/restart-gate.ts` — a pure, unit-testable rolling-window rate limiter:
      at most `MAX_RESTARTS` (5) respawns per `RESTART_WINDOW_MS` (60 s), then give up + error.
      Removed the reset-on-health line; the window ages attempts out so a long-up hub still recovers.
- [x] `start()` re-entrancy guard (`launching` flag) so concurrent starts (onboarding + auto-start +
      tray + renderer) can't slip past the `child` check during `await findFreePort` and spawn an
      untracked orphan hub. Flag claimed only after the synchronous mkdirs so a throw can't wedge it.
- [x] `index.ts` single-instance lock (`app.requestSingleInstanceLock()` + `second-instance` →
      focus) so a second launch can't run a competing hub over the same DB.

**Verify:**
- [x] Reproduced the loop in a model: OLD policy = 1000 respawns over 1000 flaps (unbounded); NEW
      gate = 5 respawns then gives up. Recovery-after-window and reset-on-stop asserted. All pass.
- [x] `npm run typecheck` green (node + web); `npx electron-vite build` green; confirmed the gate +
      single-instance code inlined into `out/main/index.js`.

**Not in scope (separate, not the melt):** unsigned/quarantined DMG still needs right-click→Open (a
signing/notarization task); this fix makes the app *safe* regardless — worst case is an error state,
never a meltdown. Re-enabling the web download CTAs (hidden in #64) can follow once signing lands.

### Follow-up: full desktop audit (requested "make sure there are no other issues")

Read every main + renderer + shared + preload file and the build scripts. **No other critical/HIGH
issues.** Verified safe: all 3 process-spawn sites are on-demand + `execFile` array args (no shell
injection) with timeouts, only the hub spawn needed rate-limiting (fixed); IPC is a typed enumerated
bridge with `contextIsolation` on / `nodeIntegration` off; `shell.openExternal` only ever gets two
hardcoded https URLs; no `dangerouslySetInnerHTML`/`eval` anywhere (React escapes all hub-supplied
strings); CSP set in packaged builds; timers all bounded + cleaned up; icon generator is
dependency-free.

Fixed two robustness gaps (stability-class, self-contained):
- [x] **React error boundary** (`renderer/src/components/error-boundary.tsx`, wrapped in `main.tsx`)
      — a single throwing component (e.g. a malformed public-hub directory entry missing `card`) was
      white-screening the whole window; now shows a recoverable fallback + Reload.
- [x] **Cap the session viewer's message buffer** (`.slice(-1000)`) — the only unbounded renderer
      array; a long-running watch grew it without limit.

Reported, not fixed (LOW — left for a follow-up so this stays scoped):
- `mcp.ts` `writeConfigServer` renames the live config to `.parler-backup` on every connect, so a
  second connect clobbers the *pristine* pre-parler backup (the user's other servers are still
  preserved in the merge — only the safety snapshot is lost). Guard with `if (!existsSync(backup))`.
- `api.ts` fetch has no timeout; a hung hub connection leaves a spinner (local hub is instant + public
  is fast, so unlikely). Mirror `probeHealth`'s AbortController.
- No main-process `unhandledRejection` logger; the supervisor's `void this.start()` on a mkdir throw
  would reject unhandled (mkdir of userData ~never fails). A log-only handler would surface it.
- `session-viewer` poll `setInterval(load,…)` isn't re-entrancy-guarded — only matters if a fetch
  takes >4s (not on localhost). An in-flight guard removes the theoretical double-append.

Verify: `npm run typecheck` green (node + web); `npx electron-vite build` green (all 3 bundles).

---

# Task: New SEO blog post — technical challenges building Parler

Goal (user, 2026-07-01): another SEO-boosting blog post on the *technical challenges*
while building Parler, to gain traction + improve SEO.

## Angle (distinct from the 4 shipped posts)
Debugging war stories: five real bugs that only surfaced past "compiles + passes on my
machine." SEO win: ranks for the exact error/concept searches other Rust/agent devs make
(rustls CryptoProvider panic, tokio-tungstenite wss, auth vs authz, electron restart loop,
async SQLite spawn_blocking). None of the existing posts own this cluster.

## The five war stories (all verified against repo code)
- [ ] 1. WebSocket only broke over TLS — tokio-tungstenite wss + rustls 0.23 dual-provider
      panic (ring vs aws-lc-rs) → ensure_crypto_provider() (parler-connector/src/client.rs)
- [ ] 2. Private hub that wasn't private — key proves identity not authorization; join
      secret, constant-time secret_matches() (parler-hub/src/server.rs, secret.rs)
- [ ] 3. Invite that skipped its own approval gate — minter auto-join self-join bypass; the
      "room already exists + not a member" guard (server.rs ~1254)
- [ ] 4. Crash loop that cooked a MacBook — RestartGate rolling window (desktop/.../restart-gate.ts)
- [ ] 5. One SQLite connection could freeze everyone — blocking I/O on async runtime →
      spawn_blocking for blobs + janitor (server.rs); honest deferral: uploads buffer in RAM

## Build steps
- [ ] Add metadata entry to web/lib/blog.ts POSTS (slug bugs-that-hid-until-production)
- [ ] Create web/components/blog/bugs-that-hid-until-production.tsx (prose primitives)
- [ ] Wire slug->body in web/app/blog/[slug]/page.tsx BODIES + import
- [ ] Create an on-brand SVG cover /public/blog/war-stories.svg
- [ ] Drop prose source docs/blog/bugs-that-hid-until-production.md
- [ ] Voice: no em/en dashes; run humanizer; scan for U+2014/U+2013
- [ ] Verify: cd web && npm run build green

## Review (done 2026-07-02)
- Post shipped: web/components/blog/bugs-that-hid-until-production.tsx (+ POSTS entry, BODIES wire,
  SVG cover web/public/blog/war-stories.svg, docs/blog source, docs/assets copy).
- Distinct angle: debugging war stories → ranks for exact error searches (rustls CryptoProvider
  panic, tokio-tungstenite wss, auth vs authz, electron restart loop, async spawn_blocking).
- All 5 code snippets verified against the real repo before writing.
- House voice: 0 em/en dashes across tsx/ts/svg (md scanned + cleaned too).
- Verified: npm run build GREEN (27/27 static pages); next start smoke — post 200 (correct
  <title>), cover 200 image/svg+xml, index lists it, sitemap+RSS include it, OG card 200 PNG.
- Memory updated: parler-blog-content-strategy (added post #0, refreshed untapped angles).

---

# 2026-07-02 — UX streamline, tranche 1 (from the connect/hub/rent-an-agent audit)

Scope for this tranche: the quick-win tier of the UX audit (CLI + small desktop pieces). The
medium/big items (approvals inbox, team mode in app, `parler work` rental daemon, card offers,
website hire flow) go to `tasks/backlog.md` as scoped entries.

## Plan
- [x] CLI zero-setup: every networked CLI command self-bootstraps an identity (same path as
      `parler mcp`) instead of erroring "run parler init"
- [x] `parler connect` re-run preserves each already-wired host's hub + join secret unless a hub
      flag (`--shared`/`--local`/`--team`/`--hub`) is passed; report shows the hub per host
- [x] Per-host "load it" guidance (Claude Code = next session; Cursor/Windsurf/Desktop = restart…)
      in human + `--json` output
- [x] `parler connect --verify`: after wiring, wait and report each agent as it dials the hub
      (polls `discover` per target hub; timeout flag)
- [x] `parler send/handoff/push --to <name>`: resolve a directory name to an agent id when the
      value isn't an nkey id (unique match required; helpful error otherwise)
- [x] `parler session requests/approve/deny/watch`: `--room` defaults to the active session
      (parity with the MCP tools)
- [x] `parler consolidate`: update the hardcoded legacy Anthropic model id → `claude-haiku-4-5-20251001`
- [x] Desktop: "Start Parler at login" setting (hub survives reboot)
- [x] Desktop: post-connect "dialed in" verification on the Connect screen (poll local directory,
      per-host green check when the agent first authenticates)
- [x] **Root-cause found + fixed:** dial-in/verify needs "connected == discoverable", but `parler
      mcp` didn't self-list. Added `auto_register` (private/same-hub by default; `PARLER_PUBLIC`/
      `PARLER_TAGS`/`PARLER_SKILLS`/`PARLER_DESCRIBE`/`PARLER_NO_REGISTER`) so a wired agent lists on
      connect. Also makes the desktop Agents screen's "connect an agent and it appears here" true.
- [x] Backlog: added the follow-on epic (`parler work`, card offers, `parler task`, approvals inbox,
      app team mode, signed task receipts, `[HUMAN] web:` hire flow)
- [x] Gate: `CI_SKIP_WEB=1 make ci` green + desktop typecheck + build green

## Review (done 2026-07-02)

**What shipped (all in `parler-cli` + `desktop/`, additive + backward-compatible):**

- **Zero-setup CLI** (`lib.rs` `connect()`): a networked command with no identity now mints one via
  the shared `mcp::load_or_bootstrap_config()` (prints a one-line "first run — created your identity")
  instead of erroring `run parler init`. Verified live: `register`/`send`/`presence` all bootstrap.
- **Hub-preserving re-run** (`connect.rs`): a bare `parler connect` reads each host's currently
  wired `PARLER_HUB`+`PARLER_JOIN_SECRET` (`configured_env`) and keeps it (`kept:true`); an explicit
  `--shared`/`--local`/`--team`/`--hub` moves it (`hub_pinned`). New `--shared` flag. Verified live:
  local→(bare)kept→(--shared)moved, confirmed in the written TOML.
- **Per-host restart hints** (`restart_hint`): Claude Code = "next session, no restart"; Codex/Gemini
  = "next run"; Cursor/Windsurf/Desktop = restart. In human output + `--json` (`restart` field).
- **`parler connect --verify`** (`lib.rs` `verify_dial_in`): after wiring, dials each target hub with
  the wired secret and prints each agent as its card appears (`✓ codex dialed in (6s)`), or a timeout
  hint. Verified live end-to-end against a real restarting `parler mcp`.
- **Name→id `--to`** (`lib.rs` `resolve_target`): `send`/`handoff`/`push --to <name>` resolves a
  directory name to its id (unique-match; helpful error on unknown/ambiguous). nkey ids pass straight
  through. Verified live: `--to reviewer` lands in the same DM room as `--to <id>`.
- **Session `--room` defaults** (`session_room`): `requests`/`approve`/`deny`/`watch` fall back to the
  active session, matching the MCP tools.
- **`parler consolidate`**: legacy `claude-3-5-sonnet-20241022` → `claude-haiku-4-5-20251001` (the
  fast tier, matching the gpt-4o-mini / gemini-2.5-flash siblings).
- **MCP auto-self-list** (`mcp.rs` `auto_register`): the root-cause fix so the two dial-in features
  work and the Agents screen fills. Private by default; env-tunable; opt-out. Verified live: a real
  `parler mcp` agent self-listed with a verified private card + its `PARLER_TAGS`.
- **Desktop start-at-login** (`settings.ts` `syncLoginItem`, `types.ts`, `ipc.ts`, `index.ts`,
  `settings.tsx`): `startAtLogin` setting → `app.setLoginItemSettings({openAtLogin, openAsHidden})`,
  reconciled on boot + on toggle, so the hub survives reboot.
- **Desktop dial-in** (`components/dial-in.tsx`, wired into `connect.tsx` + `onboarding.tsx`): after a
  local connect-all, polls the local directory (auto-minted token) and latches each wired agent to
  "dialed in ✓" — closing the old dead-end that ended at "restart them" with silence.

**Bug caught + fixed along the way:** `write_toml` used index-assignment on a fresh doc, which
produced an empty inline `mcp_servers = {}` and silently dropped the server on a first-time Codex
install. Materialize the table via `entry(...).or_insert(Item::Table(implicit))` first. Regression
test added (`toml_write_works_on_a_fresh_file`).

**Tests:** parler-cli 29 pass (+ `configured_env`, `restart_hint`, fresh-TOML, `auto_register`
self-list, `env_flag`/`env_list`). Full `CI_SKIP_WEB=1 make ci` green (build · clippy -D warnings ·
test · doc · deny). Desktop `npm run typecheck` + `npm run build` green.

**Deferred to backlog (medium/big):** `parler work` daemon (rental keystone), card `offers`, `parler
task --wait`, desktop approvals inbox, desktop team mode, signed task receipts, `[HUMAN] web:` hire
flow / A2A inbound. See `tasks/backlog.md` → "From 'connect agents' → 'operate a hub' → 'rent out'".

---

## Token-efficient agent comms — Wave P0 + P1 (2026-07-03, branch `token-efficient-agent-comms`)

Render-side token efficiency for the Parler MCP server: every tool result/spec lands verbatim in each
agent's LLM context, so the goal is to shrink the *rendered* char count with zero hub/protocol change
(additive, works against the deployed hub). Spec: `~/.claude/plans/system-instruction-you-are-working-goofy-crane.md`.
Sub-plan: `~/.claude/plans/system-instruction-you-are-working-goofy-crane-agent-ac4df53d7d8a849fc.md`.

### P0.1 — Measurement harness (DONE)
Budget tests in `mcp.rs mod tests` that print the rendered-char count so each later item's saving is
provable and gated: `tool_specs_stay_lean`, `join_with_backlog_renders_under_budget`,
`send_with_waiting_replies_renders_under_budget`; helpers `body_of(len)` (fixed-size) + `seed_room`
(post N fixed messages from a member). Consts `TOOL_SPECS_BUDGET`/`JOIN_RENDER_BUDGET`/`SEND_RENDER_BUDGET`
start loose (~20% headroom) and get tightened by the item that produces each saving.

**Baselines measured (before any diet):**
- tool_specs: 23 tools, **11,598 B** serialized (5,261 B of descriptions).
- join_session with ~100-msg backlog (full replay): **7,863 chars**.
- parler_send with ~20 waiting replies: **1,657 chars**.

Gate: `scripts/verify.sh --rust-only` green.

### P0.2 — Tool-spec diet + cheap-path steering (DONE)
Rewrote the 23 tool descriptions tight while teaching the cheap path in the prose: prefer `wait_secs`
long-poll over polling; keyed facts (`parler_remember key="session-digest"`) for durable state instead
of re-reading history; `since`+`limit` re-reads render in full (never truncated). Kept the security
guidance ("confirm with the user before approving", the join-key-can't-read warning) and kept specs
fully static (prompt-cache friendly). Added the `backlog: recent|full` param to `parler_join_session`
(wired in P0.3) and `to`-takes-a-name / `since`/`limit` param hints (wired in P1.2/P0.4).

**Descriptions 5,261 B → 4,304 B (−957 B).** Full `tools/list` payload 11,598 B → 11,030 B (net −568 B;
the description cut is partly offset by the new cheap-path param hints, which are themselves the steering
payoff). Tightened `TOOL_SPECS_BUDGET` to 11,600 and added a `TOOL_DESC_BUDGET` (4,500) so the diet
can't silently regress. Gate green.

### P0.3 — Digest-not-replay session joins (DONE)
`join_session` now renders a *digest* of the backlog instead of a full replay. The load-bearing
`pull(&room, None, None)` is unchanged (cursor still advances to the live edge past the whole backlog);
only the render changed. New `digest_backlog(msgs, mode)` helper (shared with P1.1): in `Recent` mode
(default) it renders the context **seed in full** (earliest message starting with the `📋 session
context` marker) → `— N earlier message(s) omitted; parler_recv since=<seq> to re-read, parler_recall
for decisions —` → the last `JOIN_TAIL=15` messages in full; roster is rendered as a **count** line, not
a full listing. New `backlog: "recent"|"full"` param on `parler_join_session` (default recent; `full`
is the escape hatch that replays everything). `PARLER_SESSION_KEY` env-join uses `Recent`.

**join_session with ~100-msg backlog: 7,863 → 1,458 chars (−81%).** `JOIN_RENDER_BUDGET` tightened to
3,000. Tests: `join_digests_long_backlog` (seed + omission line + tail present, a middle message not
replayed, roster is a count) and `join_full_mode_renders_entire_backlog` (full replay, no omission
line). Existing small-backlog join tests unaffected (≤ JOIN_TAIL → render everything). Gate green.

### P0.4 — Render budgets on recv + auto-pull (DONE)
Bounded the two unbounded per-call render paths, losslessly (a limited pull advances the cursor only
through the batch, so the remainder waits for the next call):
- `parler_recv` default `limit` → `RECV_DEFAULT_LIMIT=30` in **cursor mode only** (no explicit `limit`,
  no `since`, not verbose); `— more waiting: call parler_recv again —` when the batch fills.
- auto-pull-on-send caps at `AUTOPULL_LIMIT=10` with the same hint; the handoff banner scans the capped
  batch (a handoff past the cap resurfaces on the next recv — it stays unread).
- New `render_message_budgeted`: per-message body cap `MSG_MAX_CHARS=1200`, UTF-8-safe truncation with
  `…[+N chars — parler_recv since=<seq-1> limit=1 for full]`. **Never** applied to explicit-`since`
  re-reads (rendered full via `render_message`), the seed, or banners.
- Opt-outs: an explicit `limit` per call overrides the cap; `PARLER_MCP_VERBOSE=1` disables caps
  globally. Limit decision extracted to a pure `recv_limit(explicit, re_read, verbose)` so it's unit-
  testable without racing on process env.

**parler_send with ~20 waiting replies: 1,657 → 740 chars (−55%).** `SEND_RENDER_BUDGET` tightened to
2,000. Tests: `recv_caps_batch_but_drains_losslessly` (30 + "more waiting", second recv drains the
rest, union == all 40, third recv empty), `autopull_hints_more_when_replies_overflow_the_cap`,
`long_body_is_truncated_then_refetchable_in_full` (truncated body → `since=<seq-1> limit=1` returns it
in full, re-reads never truncated), `recv_limit_decides_the_cap` + `budgeted_render_truncates_only_
over_the_cap` (pure). Gate green.

### P1.1 — Digest the MCP prompts (DONE)
- `parler_session_handoff` prompt: now renders the shared `digest_backlog(Backlog::Recent)` (seed +
  recent tail + omission line) instead of the full replay, with a roster **count** and a "parler_recv
  since=<seq> / parler_recall for more" pointer.
- `parler_consolidate_session` prompt: analyzes at most the **last 100** messages and instructs writing
  the recap via `parler_remember key="session-digest" room="<room>"` (idempotent rolling digest — sets
  up P1.3), keeping "extract 1–5 decisions".
- Backlog **resource** doc string: relabeled as the explicit full-replay escape hatch and notes the hub's
  200/page cap (`since` pages). The resource read itself stays a full pull — it IS the escape hatch.

Test: `session_handoff_prompt_digests_not_replays` (drives `prompts/get` through the `run` loop; asserts
seed + omission line + roster count + `parler_recv since=` pointer, and that a mid-backlog message is not
replayed). Gate green.

### P1.2 — discover/roster compaction + name-based DM (DONE)
- **Name-based targets:** made `resolve_target` (lib.rs) `pub(crate)` and routed all four MCP DM paths
  through it — `parler_send to=<name>`, `parler_card <name>`, `parler_push to=<name>`, `parler_handoff`
  — so a directory **name** works without the 56-char id (unique-match-or-error; never guesses). All
  four were trivial one-liners on the `Target::Dm` they already build, so `parler_push`/`parler_handoff`
  were included too (no `[HUMAN]` follow-up needed).
- **discover:** client default `limit` → `DISCOVER_DEFAULT_LIMIT=25`; compact line drops the id
  (`name (role) [vis✓] status — tags`); `detail:true` restores the id; a full batch appends a "narrow
  or raise limit" nudge. The name in the compact line is enough to `parler_send to=` / `parler_card`.
- **roster:** `name (role) [status]`; ids only with `detail:true`.

Tests: `discover_is_compact_by_default_and_detailed_on_request`, `roster_hides_ids_by_default`,
`mcp_send_resolves_name_to_id` (name → unique id; unknown name errors, never guesses). Bumped
`TOOL_DESC_BUDGET` 4,500 → 4,700 for the ~230 B of cheap-path steering P1.2 adds (still ~730 B under the
pre-diet baseline; total specs 11,466 B, under budget). Gate green.

### P1.3 — Rolling session digest as a keyed fact (DONE)
Zero-hub-change convention: the host keeps a rolling recap via `parler_remember key="session-digest"
room=<room> text="SESSION DIGEST: …"` (idempotent upsert). A late `join_session` now recalls it
(`recall("SESSION DIGEST", room, limit=1)`) and renders it in a `--- session digest ---` block **above**
the backlog tail — a human-written recap beats re-reading raw history. Belt-and-suspenders acceptance:
the top hit is used only when its `key == "session-digest"` **and** its text starts with the
`SESSION DIGEST` sentinel (guards against a BM25 false positive). Silent skip when absent or on error.
`open_session` output + the open/remember tool specs teach the host to maintain it. Deterministic
fetch-by-key (no BM25) is the queued P2.1 upgrade.

Tests: `join_surfaces_session_digest_fact` (host remembers it → joiner sees the header + recap),
`join_without_digest_fact_is_silent` (absent → no header, no error). Gate green.

### P1.4 — Trim chatty tool results (DONE)
Shortened the wordy result strings, keeping every actionable artifact + test-pinned marker:
- `open_session`: ~850 → **615 chars**. Kept `KEY: `, the `claude mcp add parler … PARLER_SESSION_KEY=`
  one-liner, the approval `gate` sentence, the `session-digest` guidance, the watch-viewer pointer, and
  `link:`; cut the narration. Tightened the approval-gate prose too. `OPEN_RESULT_BUDGET=800` pins it.
- `parler_watch_session`: kept the code + "treat it like a password" warning; cut filler.
- `parler_invite`: dropped the duplicated trailing code echo.
- `parler_push`: folded the `blob:` line into the `parler apply <blob>` line (one blob id, not two).

Test: `open_then_join_shares_context_and_sets_active_session` now also asserts `link:`, `session-digest`,
`parler_watch_session` survive and that the result is under `OPEN_RESULT_BUDGET`. Gate green.
