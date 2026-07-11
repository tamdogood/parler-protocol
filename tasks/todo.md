# Plan — ACP borrows: implement "Worth borrowing" (2026-07-10, branch moroni) — ✅ DONE

Audited https://agentcommunicationprotocol.dev (now merged into A2A). Implemented all five borrows,
each **additive + backward-compatible** with the deployed hub. `CI_SKIP_WEB=1 make ci` **green** (all
gates: selftest · rust incl. clippy -D warnings + cargo doc -D warnings · audit).

**Results:**
- **Error codes** — `ServerFrame::Error` gained optional `code`; `parler_protocol::error_code` catalog
  (16 stable classifiers) + shared `CodedError` (Display == message, so no string-behavior change); hub
  codes ~20 sites via `coded()`/`error_frame()`; connector surfaces them (`hub_error_code`). Tests:
  protocol codec round-trip (coded + uncoded) + `coded_error_displays…` + e2e `not_member` over the wire.
- **Task lifecycle** — `com.parler.task` `TaskRef`/`TaskStatus` (extension part, zero hub change);
  `parler task <status>` CLI + `parler_task` MCP tool (budget raised 13,200→14,200 for the 27th tool,
  documented); one-line render (`✅/🔧/⏳/❌ task …`); terminal receipts carry `tokens`/`elapsedMs`.
  Tests: codec + MCP e2e (peer sees the rendered working/done lines).
- **Capability descriptor** — `/api/hub.capabilities` + `/.well-known/parler.json` (push/longPoll/blobs/
  size caps/joinPolicy/messageKinds). Tests: smoke.rs (2 live-HTTP) + smoke.sh contract probes.
- **Portable session key** — `session open` prints `<code>@<hub>`; `session join <code>@<hub>` dials that
  hub (`connect_with_hub`); `split_portable_key` unit-tested.
- **Docs** — new `docs/task-lifecycle.md` + `docs/patterns.md`; root `llms.txt`; updated communication.md,
  discovery.md, agent-mesh.md, AGENTS.md, README.md; backlog follow-ons (hub-derived telemetry,
  federation questions, `[HUMAN] web:` serve llms.txt).

Originally-planned checklist (all complete):
- [x] **P1 · Error codes on the wire** — `ServerFrame::Error` gains optional `code` (serde default,
  skip-if-none) + a shared `error_code` catalog in `parler-protocol`; hub sets codes at the known
  frame sites; connector surfaces them via a typed `HubError` (Display == message, so string behavior
  is unchanged) a caller can downcast for the code. Tests: codec round-trip + coded-frame downcast.
- [x] **P1 · Task lifecycle** (`com.parler.task`) — the centerpiece. New `TaskRef`/`TaskStatus` in
  `parler-protocol` mirroring `HandoffRef` (extension part → zero hub change); terminal receipts carry
  optional `tokens`/`elapsedMs` (feeds telemetry). `parler task <status>` CLI + `parler_task` MCP tool
  (lean desc, watch the budget); render a one-line `✅/🔧/⏳/❌ task …`. Docs: `docs/task-lifecycle.md`
  + communication.md row + AGENTS.md index. Tests: codec + CLI/MCP emit + render.
- [x] **P2 · Hub capability descriptor** — extend `/api/hub` with a `capabilities` block + add
  `/.well-known/parler.json` (mirrors the existing `/.well-known/agent-card.json`) so a client can
  probe push/wait/blobs/joinPolicy/features before handshaking. Smoke + doc.
- [x] **P2 · Portable session descriptor** — `session open` also prints a portable
  `join <key>@<hub>` line; `session join` parses `key@hub` and dials that hub for the join (sugar over
  `PARLER_HUB=… parler session join`). Design note in agent-mesh.md + backlog item for the deeper
  cross-hub questions (auth, history availability). Tests: parse + hub override.
- [x] **P2 · Doc/positioning** — `docs/patterns.md` (chaining/routing/parallel recipes over Parler
  verbs), repo-root `llms.txt` (machine-readable doc index), and the telemetry design note folded into
  `docs/task-lifecycle.md` + a backlog item ("hub-derived, not self-reported"). `[HUMAN] web:` serve
  llms.txt from parlerprotocol.com.

Verify: `scripts/verify.sh --rust-only` between phases; `CI_SKIP_WEB=1 make ci` (runs `cargo doc -D
warnings`) before done. Never `cargo fmt`. Not committing/PRing unless asked.

## Follow-up — MCP tool audit + description diet (2026-07-10)

Audited the 27-tool `tools/list` surface (a permanent per-session context cost). Data-driven (per-tool
byte breakdown): no tool is dead weight, but descriptions had crept back up (5,190 B). Dieted all 27
descriptions — kept the load-bearing steering, cut verbosity — **no capability removed**:
- specs 13,908 → **12,727 B** (−1,181), descriptions 5,190 → **4,297 B** (−893).
- Net **below** the pre-`parler_task` baseline (12,945 B): the new tool now *reduces* the surface cost.
- Budgets cut to lock it in: `TOOL_SPECS_BUDGET` 13,200 → **13,000**, `TOOL_DESC_BUDGET` 5,000 → **4,600**
  (both *below* the originals despite +1 tool). `CI_SKIP_WEB=1 make ci` green.
- Tool merge/retire (breaking) deferred to a deliberate call — candidates logged in `backlog.md`.

---

# E2E functional audit — durable-cursor fix (2026-07-09, branch nairobi)

## Audit verdict (live run, real binaries: hub + 4 CLI agents + MCP stdio)

**GREEN** — invite/join/roster, 3-agent room fan-out, real-time push (`recv --watch` live),
file transfer (blob id == sha256, byte-exact both peers, dedup on re-send, non-member fetch/recv
denied), session open→key→approval-gate→context catch-up, memory remember/recall, MCP stdio path
(send/recv/auto-pull), 30 concurrent sends (seq 1–38, no loss/gaps), hub restart durability
(roster/memory/blobs survive), DB efficiency (WAL + NORMAL sync per conn, capped shared cache,
blobs off the SQLite path, `idx_messages_room_seq`/`idx_members_agent`, incremental autovacuum).

**RED — one HIGH functional bug:**

### The durable cursor never commits on any one-shot pull path
- Deferred-ack (#85): `store.pull` with `ack` present does **not** advance the cursor past the
  returned batch — the client commits it via `ack` on its *next* pull. `MeshAgent.pending_ack`
  is an in-memory HashMap, so the ack **dies with the process**.
- `parler recv` = one pull per process ⇒ the ack never flushes ⇒ `members.cursor` stays 0 forever
  (verified in SQLite after 4 pulls), every recv re-reads the whole history, "— cursor at N —" is
  a lie, `parler rooms` unread counts are wrong, `session join`'s "advances the fresh cursor"
  comment (lib.rs:1101) is durably false, and an MCP cold start re-pulls everything the CLI ever
  "read" (all reproduced live).
- The existing e2e test masks it: `reconnect_resumes_from_durable_cursor` pulls **twice** in the
  first connection to flush the ack before dropping.

## Fix plan (assigned to Opus subagent; reviewed by main agent after)

1. **connector**: add `MeshAgent::commit_reads(&mut self, room)` — one `Pull { room, since: None,
   limit: Some(0), wait_secs: None, ack: pending }` round trip = pure ack commit (store already
   applies `ack` before the read; LIMIT 0 reads nothing; `ack.is_some()` ⇒ no advance-on-read).
   Plus best-effort `flush_acks(&mut self)` over all rooms with a pending ack. No new frames, no
   schema change — reuses the additive #85 field (no `deny_unknown_fields` in the protocol).
2. **CLI one-shot call sites**: flush after render in `cmd_recv` (non-watch; and once after the
   initial backlog in watch mode) and `session join` (lib.rs:1101). `--since/--all`/`Some(0)`
   reads stay pure. Long-lived loops (watch iterations, hook watch at lib.rs:1200) self-ack.
3. **MCP**: flush after `parler_recv` / `parler_join_session` render + best-effort `flush_acks`
   when the stdio run loop exits.
4. **Tests (red first)**: e2e "single-pull process → reconnect sees only newer" (fails today);
   MCP-layer recv-then-restart test; store-level ack-only-pull (limit 0) commit test if missing.
5. **Docs**: re-grep cursor/ack language (`docs/communication.md`, `agent-mesh.md`,
   `storage-and-memory.md`) — behavior returns to documented ("never re-read"), so mostly verify.
6. **Gates**: `scripts/verify.sh --rust-only` then `CI_SKIP_WEB=1 make ci`. Live re-run of the
   audit repro (recv twice from two processes; second must be empty; `members.cursor` > 0).

- [x] Audit (this section)
- [x] Fix implemented (Opus subagent) — `commit_reads`/`flush_acks` on the connector; wired into
      `cmd_recv`/`session join` (CLI) and `parler_recv`/`enter_session`/run-loop-exit (MCP). No wire
      change (reuses the additive #85 `ack` field).
- [x] Regression tests red→green — e2e `single_pull_process_commits_cursor_via_commit_reads` +
      `commit_reads_is_idempotent_and_safe_on_empty`; store `ack_only_limit_zero_pull_commits_cursor_without_reading`;
      MCP `mcp_recv_commits_cursor_across_a_restart` + `mcp_run_loop_flushes_deferred_acks_on_exit`.
      All three behavioral tests verified red against the pre-fix code, then green.
- [x] Review passed (parler-review contract) — full-context read of every changed function, all
      candidate findings verified-then-dropped (raw-vs-filtered commit guard confirmed raw; all
      other pull paths audited: follow_session/watch self-ack, consolidate/--all pure, hook
      send-only); gates re-run independently; live repro re-run against a PRE-fix hub binary
      (old-hub compat proven). Verdict: approve.
- [x] `make ci` green + live repro green — `CI_SKIP_WEB=1 make ci` all gates pass; live: bob's
      second `recv` prints "(no new messages)", `members.cursor` = max seq read, `rooms` 0 unread.

---

# UX redesign: the wire, not the window (v2 — post-audit)

Goal: make Parler feel as simple as Darren Bounds' one-line `codex exec` hack for the solo case,
while keeping the niche Mosaic-style apps can't touch — **agents that don't share a screen, a
machine, or an owner**. Cut conceptual load, make watch-live the visceral demo, and shrink the
macOS app to the one job only a resident app can do.

Positioning (decided 2026-07-08): Parler is the **wire** (agent↔agent, async, durable,
cross-tool/machine/owner); Mosaic is a **window** (humans watching shared terminals, sync,
macOS-only). Don't chase the window. Mosaic is GPL-3.0 — ideas only, never code. The solo
one-liner is the **funnel**, not the niche.

**Success criteria (measured, not vibes):**
- Fresh machine → second opinion in chat: **< 60 s, ≤ 2 concepts touched** (install, `bring`).
  Today's happy path touches ~6 (hub, key, join, approval, identity, session).
- Owner-offline join request → owner acts on it **without opening a terminal** (Phase 4).
- Kill criterion for `bring` v2 (MCP-joiner mode): if a headless agent can't reliably drive the
  join→pull→reply loop in the Phase 0 spike, ship pipe-mode only and revisit.

## Phase 0 — Spike + spec (1–2 days; de-risk before design)

- [ ] **Spike the riskiest assumption first**: can `codex exec` (headless, one-shot) reliably
      drive parler MCP tools (join session → pull → reply)? Timebox: half a day.
- [ ] Decide the v1 transport based on the spike:
  - **Pipe mode (default v1, zero protocol risk):** `bring` opens the session, pipes the recap
    into `codex exec --sandbox read-only` on stdin, and posts codex's output back into the
    session itself. Deterministic — no dependency on the joiner's tool-calling behavior. The hub
    stays the system of record; cross-machine/MCP mode comes later.
  - **MCP-joiner mode (v2):** joiner self-bootstraps with `PARLER_SESSION_KEY` and participates
    as a real agent. Only if the spike passes.
- [ ] Write `docs/research/parler-bring-spec.md` covering both modes plus:
  - **No protocol change needed for approval:** the host client creates/knows the joiner id, so
    it polls `JoinRequests` for its own room and auto-resolves that exact id in-process.
    Owner-initiated, single-id — the gate is not weakened. (#108's general pre-approval is now
    *not* a blocker for `bring`.)
  - **Async return shape:** `parler_bring` must NOT block an MCP tool call on a multi-minute
    review (host tool-call timeouts). It returns immediately ("codex is reviewing in room X");
    the reply lands as a normal message via recv/auto-pull.
  - **Subprocess hygiene:** whitelisted agent names only (no shell interpolation from tool
    args), hard timeout, kill on session close, reap zombies. An MCP tool that spawns processes
    is a new security surface — spec it, review it against the security model.
  - **Tool-list budget:** coordinate with #89 — `bring` must not just grow the 11 KB tools/list;
    a pipe-mode joiner needs zero parler tools, an MCP joiner needs a minimal profile.
- [ ] File issues (bring, menubar approver, JoinRequested push frame, messaging pass); link into
      epic #113.

## Phase 1 — `parler bring` v1, pipe mode (the on-ramp)

Moved ahead of the big UX issues: nothing in #104/#108/#111 blocks pipe mode, and this is the
only phase that ships new user-visible value. 

- [ ] CLI `parler bring codex` (open session → spawn codex with recap → post reply back).
- [ ] `parler_bring` MCP tool, async return; calling agent supplies the recap.
- [ ] Handle the unhappy paths: codex not installed / not logged in / times out — error names
      the remedy (#111 style even before #111 lands).
- [ ] `scripts/demo-bring.sh` — the 15-second demo; measure the <60 s success criterion in it.
- [ ] Docs: README "second opinion in one line"; docs/communication.md + tool tables.
- [ ] Verify: live run on local hub AND shared hub; every printed command copy-paste-runnable
      (the #99–#103 lesson); `CI_SKIP_WEB=1 make ci`.

## Phase 2 — Conceptual simplification (existing UX lane, resliced)

- [ ] #108 **sliced**: session close + expiring keys + owner-offline signal. (General
      pre-approval hatch is still worth having for teammates — keep, but it no longer gates
      anything here.)
- [ ] #104 per-workspace identity — also what stops `bring`'s spawned joiner colliding with the
      host on one machine; `bring` v1 must set a scoped `PARLER_HOME` until #104 lands properly.
- [ ] #111 one error-message standard.
- [ ] Verify each: e2e test per issue acceptance criteria + CI green; docs greped in same PR.

## Phase 3 — Messaging (rescoped: #161 already shipped the hero)

The landing page was redesigned 2026-07-09 (f10c226): 4 sections around the 42 s demo video.
Do NOT redo it.

- [ ] Audit current README + site copy against the "wire vs window" positioning; fix drift only.
- [ ] Fold `bring` into the demo video / quickstart once Phase 1 ships (the video predates it).
- [ ] Blog post via `write-blog`: "a window or a wire" angle (distinct from the 4 shipped posts;
      humanizer pass).
- [ ] Verify: `cd web && npm run build`; every claim matches shipped behavior.

## Phase 4 — macOS app: shrink to menubar approver

Verified feasible: the hub already supports multiple concurrent connections per agent id
(`subscribers: HashMap<String, Vec<Subscriber>>`), so the app can sit alongside the MCP session
as the owner. Verified gap: **join requests are poll-only today** — `JoinRequests` is
request/reply; there is no push to the owner.

- [ ] **v1: poll, don't push (2026-07-09 simplification).** `parler session requests --json`
      already exists for the desktop app; a human approval flow tolerates seconds of latency, so
      the menubar approver polls it (~3–5 s) — **zero protocol change, no deploy ordering, no
      compat risk**. Ship the notification UX on that.
- [ ] v2 (only if polling proves costly): `ServerFrame::JoinRequested` push. **CORRECTION
      (map-joinpush, verified):** "old clients ignore it" is FALSE — `ServerFrame` is
      internally-tagged with no serde catch-all, and both connector recv paths (`client.rs:160`,
      `:195`) propagate the error, so an unknown frame **hard-errors and drops an old subscribed
      client's connection**. The frame itself is compile-safe (all match sites have `_` arms), but
      **delivery must be opt-in-gated**: a new `ClientFrame` op (e.g. `WatchRequests`) or an
      optional `#[serde(default)]` field on the unit `Subscribe` variant (`hub.rs:415`), pushed
      only to connections that opted in. Also needs `Store::room_owner(room) -> Option<String>`
      (only the `room_owned_by` bool exists). Ripples protocol → hub → connector → CLI (+ docs);
      deploy hub first — necessary but NOT sufficient without the opt-in gate.
- [ ] App: menubar + native notification "X wants to join <room> · Approve / Reject". Reuse the
      app's existing architecture — shell out to the bundled CLI (e.g. a new
      `parler session watch-requests --json` long-poll) rather than reimplementing WS in Node.
- [ ] Keep one-click Connect; drop/de-emphasize Directory + Sessions screens (web viewer owns
      watching). App README updated to the narrowed scope.
- [ ] Note in docs: this is macOS-only sugar; headless/Linux owners (the CI niche) use
      pre-approval or CLI — no capability is app-exclusive.
- [ ] Verify: real join request on the shared hub fires the notification; approve from menubar
      admits the agent end-to-end.

## Out of scope (explicit)

- No terminal/workspace GUI (not competing with Mosaic/Ghostty on their terrain).
- No Mosaic code (GPL-3.0 vs our Apache-2.0).
- No weakening of the join gate: `bring`'s auto-admit is the owner's own client resolving the
  one id it just created; nothing broader.
- `web/` hero rework (shipped in #161) and session-viewer feature work.

## Review

### Phase 0 — spike + spec: DONE
- Spike (machine-verified, `codex-cli 0.142.5`): pipe mode viable exactly as specced; codex's final
  answer is the only thing on stdout (chatter → stderr), so zero output parsing. MCP-joiner mode is
  mechanically possible but its tool-calling reliability is unproven → deferred to a v2 spike.
- Transport decision: **pipe mode is v1.** Spec written: `docs/research/parler-bring-spec.md`.
- Correction captured: Phase 4's "additive, old clients ignore it" is FALSE (see Phase 4 note) —
  `ServerFrame` has no serde catch-all, so the push must be opt-in-gated. Not yet built (Phase 4).
- GitHub issues NOT filed (outward-facing; left for an explicit go). The plan phases here track it.

### Phase 1 — `parler bring`, pipe mode: DONE & VERIFIED
- New module `crates/parler-cli/src/bring.rs`: whitelist (`SUPPORTED_AGENTS=["codex"]`), codex
  runner (tokio::process, `--sandbox read-only --ignore-user-config -o <file> -`, stdin recap,
  hard timeout + kill/reap), typed errors → one-line remedies (#111 style). 6 unit tests.
- CLI `parler bring <agent>` (`cmd_bring`): `--context`/`--context-file` (`-`=stdin)/`--instruction`
  /`--room`/`--quiet`/`--timeout-secs`. Prints the review; `--room` posts it into a session.
- MCP `parler_bring { agent, context }`: uses/opens the active session, spawns the bundled
  `parler bring … --context-file - --room <room> --quiet` **detached** (context over stdin, reaped
  in the background), returns immediately — never blocks the tool call. Registered in the
  session-aware dispatch + `tool_specs` (budget ceilings raised with documented justification,
  matching the `parler_send_file` precedent; `tool_specs_stay_lean` green).
- Docs (no drift): README "Second opinion" example, `docs/communication.md` (row 11),
  `docs/agent-mesh.md` tool list, `web/components/docs/reference.tsx` (CLI + MCP tables).
- Demo: `scripts/demo-bring.sh` (shellcheck-clean).
- **Verified:** real `parler bring codex` returned a clean review in ~10 s, exit 0, stdout =
  review only. Full room flow on a local hub (scoped PARLER_HOME): session open → bring `--room`
  → `parler recv` shows codex's review landed as a message. `make ci` **green** (selftest, rust
  with clippy -D warnings, web, audit).

### Post-implementation review (2026-07-09, parler-review contract): 2 MEDIUM + 1 LOW found & fixed
- **MEDIUM (fixed):** a failed review was silent in the detached MCP path (bail before room post,
  stderr nulled) — the #100 phantom-tool trap reintroduced. Now a ⚠ remedy notice is posted into
  the room on failure; live-verified (1 s-timeout run → notice landed via recv).
- **MEDIUM (fixed):** the whitelist/context validation gates had no negative tests. Added 2
  MCP-layer tests (injection-shaped agents rejected before any side effect; missing/blank context
  rejected). 92 tests green.
- **LOW (fixed):** CLI `--room` post failure discarded the already-paid-for review — review now
  prints before the post. Also: `agent` made optional in the tool spec (matches the codex
  default), MCP return text updated to mention the failure notice.
- Verdict after fixes: approve. `CI_SKIP_WEB=1 make ci` green.
- **Plan improvement adopted:** Phase 4 v1 switched from the push frame to polling the existing
  `session requests --json` (zero wire change); push demoted to a gated v2 optimization.

### Not done (intentionally, per plan)
- Phase 2 (#104/#108/#111), Phase 3 (messaging/blog), Phase 4 (menubar approver + gated push).
- MCP-joiner mode (v2), kill-on-session-close, cross-machine bring, agents beyond codex.
- Nothing committed/pushed — changes are in the working tree for review.
