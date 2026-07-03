# Backlog — the autonomous loop's work queue

This is the **forward queue**: a prioritized list of small, independently-shippable items the loop
pulls from, one per iteration (top unchecked item first). It is the single source of truth for "what
to work on next".

- `tasks/todo.md` is the **log** of finished work — append a summary there when you complete an item.
- `tasks/lessons.md` is the **memory** — append a rule there after any correction or surprise.

The **`web/` app is out of scope** for this loop — Tam drives it by hand. Loop items must be
Rust/CLI/protocol only; anything that also needs a UI/site change carries a `[HUMAN] web: …` note for
the part the loop leaves untouched. The loop gates with `scripts/verify.sh --rust-only`.

Each item must be small enough to land behind `scripts/verify.sh --rust-only` in one iteration, additive,
and backward-compatible with the deployed protocol/hub. If an item is too big, the loop should split it
and check in the sub-items here rather than attempt it whole. Keep `[P0]`/`[P1]`/`[P2]` priority tags.

> **Editing rules:** add new items at the right priority; never delete history (check items off with
> `[x]` and let `todo.md` carry the write-up). Anything referencing the pre-pivot NATS architecture
> (`parler-manager`, `parler-delivery`, `parler-console`, KV planes…) is **dead** — those crates were
> removed in cc686ea. Do not resurrect them.

---

## Now (pull from the top)

### Epic: Full-app audit remediation (2026-07-03) — security hardening + setup UX
*Senior-eng/architect audit of the whole app (Rust hub/connector/protocol/CLI-MCP + web + desktop).
Full write-up: `~/.claude/plans/system-instruction-you-are-working-calm-newt.md`; summary in
`tasks/todo.md` (2026-07-03). Security posture verified **strong** — no critical/high vulns; the core
"compromised hub can't lie" invariants hold under inspection. These are the follow-ons; each additive
+ backward-compatible.*

Wave 1 — quick wins (**DONE 2026-07-03**, see `tasks/todo.md`):
- [x] **[P1] Atomic 0600 secret writes** — `parler_auth::write_private_file` (temp file + rename)
  replaces the `write`-then-`chmod` window for the nkey seed (`config.rs`) and hub join secret (`secret.rs`).
- [x] **[P2] Redacting `Debug`** — `Identity`/`ConfigFile` no longer print the seed via `{:?}`.
- [x] **[P2] Installer PATH self-heal** — `install.sh` smoke-tests the binary and prints an exact
  shell-rc fix + full-path fallback instead of a missable note.
- [x] **[P2] Detection dead-end hint** — `parler connect` names the checked path + `--print` escape.

Wave 2 — first-run confidence (**DONE 2026-07-03**, see `tasks/todo.md`):
- [x] **[P1] Reachability probe + next-step on `parler connect`** — a bare `connect` now dials each hub
  once (3s timeout, `probe_hubs`) and reports reachability; the `--verify`/`--list` next-steps already
  print. Subsumes the localhost-hub hint below.
- [x] **[P2] First-run online visibility** — `parler mcp` announces the minted id+hub and appends a
  trimmed `~/.parler/mcp.log` (connect/auto-register outcome); `parler doctor` shows "Recent MCP activity".
- [x] **[P2] Localhost-hub-not-running hint** — covered by the probe (`report_unreachable` → "start it
  and keep it running: parler hub --local"). `[HUMAN] web:` a README local-hub walkthrough still welcome.
- [x] **[P2] Doc: signing is flagged-not-rejected** — added to `docs/discovery.md` (security model).

Wave 3 — scale & resilience (**DONE 2026-07-03**; reconnect stays queued):
- [x] **[P1] Retention defaults + `messages(ts)` index** — `Retention::default()` now bounds messages
  (30d), unkeyed facts (500), and idle blobs (14d); `main.rs` treats an explicit `0`/negative as "keep
  all"; `idx_messages_ts` added; guard test asserts the defaults are on.
- [x] **[P2] `Arc<ServerFrame>` fanout** — the push channel now carries `Arc<ServerFrame>`, so fan-out
  clones a pointer, not the frame; push e2e stays green.
- [x] **[P2] Handshake protocol-version echo** — `Challenge` carries an optional `version`; the client
  warns on a major mismatch (`warn_on_protocol_mismatch`). Additive.
- [ ] (already queued below) **self-healing reconnect + cursor resume** — Verifiable-mesh epic P2.

Wave 4 — maintainability & observability (**mostly DONE 2026-07-03**; god-file split deferred):
- [x] **[P2] `parking_lot::Mutex` for hub locks** — store + server locks are non-poisoning; `.lock()`
  returns the guard directly (dep was already in-tree, so cargo-deny stays green).
- [x] **[P2] Lightweight metrics** — `Metrics` counters (connections/messages/pushes + live gauge)
  exposed under `/api/hub` `stats`; smoke test asserts them.
- [x] **[P2] Hardened auth challenge nonce** (was queued in the Verifiable-mesh epic) — domain-separated,
  hub-bound, expiring `parler-auth:v1:<hub>:<exp>:<rand>`; validated on step 2; zero client change.
- [ ] **[P2] Split the god-files** — `server.rs`/`store.rs`/`cli/lib.rs` into submodules. **Deferred to
  its own PR** on purpose: a large pure-refactor diff shouldn't ride with behavioral changes.

`[HUMAN] web/desktop` (Wave 5): desktop empty-state install links; README "two lines" honesty; document
the `parler://` scheme. Pairs with the existing `[HUMAN] web:` hire-flow items in "Next".

### Epic: Verifiable mesh — the hub can relay but can't lie (security + resilience)
*Audit (2026-06-29, `tasks/todo.md`): the "compromised hub can't impersonate anyone" guarantee covers
signed cards but NOT messages — a malicious hub can forge/alter/reorder the conversation a joining
agent is "caught up" on. Borrows distributed-ledger / Certificate-Transparency / reliable-messaging
ideas. Each item additive + backward-compatible.*

- [x] **[P0] Authenticated messages (signatures)** — DONE 2026-06-29 (see `tasks/todo.md` review).
  Author signs each message; carried as a `com.parler.sig` extension part (mirrors `com.parler.bundle`)
  so it needs **no hub/protocol/schema change** and works against the deployed hub. Signed payload =
  parts(non-sig) + target + author id + replyTo + client ts/uid (excludes `mentions` — hub normalizes
  them). `canonical_message_bytes` + `MessageSig` codec in `parler-protocol`; `MeshAgent::send`
  auto-signs; `verify_message(...) -> SigStatus`; CLI/MCP show ⚠/✗ (valid is clean) + hide the sig
  part; hub `/api/session` drops it; +13 tests (2 codec, 6 connector unit, 5 e2e). `VERIFY: PASS`.

- [ ] **[P1] Tamper-evident room log (hash chain + fork detection)** — sig payload commits to `prev`
  (hash of the author's last-seen message in that room); `parler verify --room R` walks the chain and
  prints a head; comparing two members' heads detects hub equivocation/split-view. Builds on the P0
  signature. *Done when:* chain fields in the sig payload, a CLI verifier, an e2e that detects a
  tampered/reordered backlog, doc in `docs/`. Additive.

- [ ] **[P1] Exactly-once sends (idempotency key)** — reuse the signed `uid` as an idempotency key; the
  hub dedups a re-sent message within a window so a retry after a dropped `Sent` ack never duplicates.
  *Done when:* hub dedup (store unique-ish on (room,uid) or a short LRU), connector retries safely, an
  e2e that double-sends one uid and asserts one stored row + same returned id. Additive.

- [ ] **[P2] Self-healing connection (auto-reconnect + cursor resume)** — a reconnecting transport
  re-handshakes on socket loss, resumes from the durable cursor, re-arms `subscribe`, exponential
  backoff. *Done when:* opt-in reconnect wrapper, an e2e that kills the socket mid-session and asserts
  the next `recv` transparently resumes. Additive (pure client-side).

- [x] **[P2] Hardened auth challenge (domain-separated, hub-bound, expiring nonce)** — DONE 2026-07-03
  (Wave 4 above). `issue_challenge`/`challenge_valid` build + validate `parler-auth:v1:<hub>:<exp>:<rand>`
  (hub token = 12 hex of `sha256(public_url)`, 60s TTL); validated on `Hello` step 2; unit test covers
  expired/foreign/malformed; zero client change; `make ci` green.

- [x] **[P0] Seed `tasks/lessons.md` discipline** — DONE 2026-06-29. The verify gate (`scripts/verify.sh
  --rust-only`) was confirmed trustworthy: it correctly **failed** on a real error (the missing `uuid`
  lock edge) and **passed** once fixed. Five new lessons appended after this iteration's surprises.

- [ ] **[P1] Code-handoff frontier index** (`docs/code-handoff.md` Phase 3) — index the latest bundle
  per room (tip id / short summary / author / ts) in the hub store; expose `parler frontier --room R`
  on the CLI; surface "latest handoff" in `parler rooms` output. *Done when:* new store table/columns
  + migration, CLI subcommand, an e2e test that pushes two bundles and asserts `frontier` returns the
  second, and the README/`docs/code-handoff.md` Phase 3 box is checked. Additive only.

- [ ] **[P1] Streaming blob upload** (`docs/storage-and-memory.md` P3 / B1) — replace the single
  fully-buffered-in-RAM blob frame with chunked upload so large handoffs don't pin memory. Keep the
  25 MiB cap as a configurable ceiling; verify sha256 incrementally. *Done when:* protocol frames for
  chunked put, hub assembles to disk without buffering the whole blob in RAM, connector streams from a
  file, and an e2e test moves a >1 MiB bundle in chunks. Backward-compatible: old single-frame path
  still accepted.

## Next

### Epic: From "connect agents" → "operate a hub" → "rent out an agent" (2026-07-02 UX audit)
*Tranche 1 (zero-setup CLI, connect --verify, hub-preserving re-run, name-based `--to`, session
`--room` defaults, per-host restart hints, mcp auto-self-list, desktop start-at-login + dial-in
verification) shipped — see `tasks/todo.md` 2026-07-02. These are the follow-on medium/big items.*

- [ ] **[P1] `parler work` — the worker daemon** (the rental keystone). `parler work --service
  code-review --runner 'claude -p "{task}"'`: watch a service queue (reuse `recv --watch`), spawn a
  headless runner per task, post the result back to the requester (DM the task author). Safety flags
  for exposing to strangers: `--approve` (each task pends until accepted — reuse the session
  join-approval pattern + a desktop notification), `--allow-from <ids>`, `--max-per-hour`. *Prereq:*
  promote the **[P2] self-healing connection (auto-reconnect + cursor resume)** item in the "Now"
  epic above — a long-lived worker must survive socket loss.
  *Done when:* the subcommand, a runner-exec seam, an e2e that enqueues a task and asserts a result DM,
  and docs. `[HUMAN] web:` a "this agent is for hire" surface can come later.

- [ ] **[P1] Card `offers` — advertise a service on the directory card** so discover→submit needs no
  human reading prose. Add an `offers` field (queue name + one-line what-it-does + input hint) to
  `AgentCard`, surface it in `discover`/`card`, and project it onto the A2A skill list. `parler
  discover --offers` filters to hireable agents. Additive (new optional card field). Pairs with
  `parler work`. `[HUMAN] web:` show offers on the agent page.

- [ ] **[P2] `parler task <agent|service> "…" --wait`** — send + long-poll the reply in one call (the
  "hire" verb; pure sugar over `send` + `recv --watch`). Also the natural home for the name→id
  resolution just added to `send`.

- [ ] **[P2] Desktop approvals inbox** — the app can act as any *local* identity (seeds live under
  `~/.parler/agents/<id>`), so it can poll `join_requests`/pending `work` tasks for locally-owned rooms
  and fire a native notification ("gemini wants to join 'auth-redesign' — Approve / Deny"). Turns the
  app into the hub's control tower. Needs new IPC (`session.requests/approve/deny`) — none exists yet.
  `[HUMAN] web:` n/a (desktop only).

- [ ] **[P2] Desktop team mode** — expose the CLI's `--team` (LAN bind + minted join secret +
  teammate one-liner) as a GUI panel: one click flips the local hub to `0.0.0.0` + secret and shows
  the exact `PARLER_HUB=… PARLER_JOIN_SECRET=… parler connect` line (+ optional QR). `HubTarget` is
  currently only `local | public` — extend it. `[HUMAN] web:` n/a (desktop only).

- [ ] **[P1] Signed task receipts** (trust rail before any payments) — a request+result pair signed
  with the existing `com.parler.sig` machinery, a per-service audit log, and caps. Builds on shipped
  message signatures + the hash-chain backlog item. No money — reputation/attribution first.

- [ ] **[HUMAN] web: hire flow on the agent page** — today an agent's page on parler-hub.fly.dev is a
  dead end. Short term: a "send this agent work" copy-paste block. Medium term: the inbound A2A
  `message/send` endpoint (already the documented phase-2 in `docs/a2a-interop.md`) translating into a
  service-queue post, so the whole A2A ecosystem can hire Parler agents.

- [ ] **[P2] sqlite-vec semantic memory** (`docs/storage-and-memory.md` P4) — this needs a client
  embedding source that does not exist yet, so it is **blocked**: land it only as a self-contained
  follow-up so the deployed protocol isn't left half-changed. Until unblocked, leave checked-off-able
  design notes only. *Prereq:* decide where embeddings come from (client-supplied vs hub-side model).

- [ ] **[P2] schemars schema export** — `parler-protocol`: generate `spec/parler.schema.json` from the
  frame types via `schemars`, plus a test that the checked-in schema matches the generated one (so the
  wire format can't drift silently). *Done when:* schema file + drift test in CI's `cargo test`.

## Icebox (needs a human decision before the loop touches it)

- [ ] Benchmarks vs the old Node implementation (criterion + e2e RTT/throughput) → `docs/benchmarks.md`.
- [ ] Anything that changes the deployed wire protocol in a non-additive way (needs explicit sign-off).
