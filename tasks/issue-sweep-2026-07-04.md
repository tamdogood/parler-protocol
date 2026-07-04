# Issue sweep — 2026-07-04

Address every open issue on tamdogood/parler-ai, one coherent PR per lane, then a Fable-model
code review. Two issues are tracking epics (#96 reliability, #113 UX) — closed when their children land.

## Constraints (from AGENTS.md / CLAUDE.md / lessons.md)
- Additive / backward-compatible wire changes only (hub is live, old clients in the wild).
- Never repo-wide `cargo fmt`. `CI_SKIP_WEB=1 make ci` green before done; clippy `-D warnings` is hard.
- Don't weaken security (approval gate, seed-never-leaves, private-by-default, join-secret, 0600 files).
- Cursor invariant: a push never advances the cursor; a limited Pull advances only through its batch.
- One PR per lane, branched off `main`; do NOT merge — leave for review.

## Conflict map (why waves, not all-parallel)
Hot shared files: `mcp.rs` (~15 issues), `connect.rs` (~6), `store.rs` (~7), `agent.rs` (~6).
Parallelism only across disjoint lanes; mcp.rs-heavy work is sequential.

## Wave 1 (parallel — disjoint lanes) — LAUNCHED
- **A · ux/setup-flow** — #99, #100, #101, #102 — connect.rs, lib.rs(cmd_connect/ConnectArgs/agent), config.rs, hub/main.rs+server.rs printed lines, deploy/private/README, README.
- **B · reliability/liveness-and-wait** — #87 + #90 (overlap: recv wait_secs + push state) — protocol Pull.wait_secs, hub/server.rs park+notify, agent.rs heartbeat/subscribe, mcp.rs recv/join_session.
- **C · docs/storage-retention** — #95 — docs/storage-and-memory.md only.

## Wave 2 (after W1 integrates — mcp.rs / store.rs cluster, sequential)
- **D · ux/session-lifecycle** — #107, #108, #109 — session close/leave/expiry/pre-approval, approve-by-name, one-code-one-door. (touches join_session → after B)
- **E · ux/mcp-input-hygiene** — #110, #111 — strict inputs + error-message standard (mcp.rs errors).
- **F · protocol/cursor-idempotency** — #85, #86 — cursor-ack + idempotent send (conflicts with B's Pull path → after B).

## Wave 3 (after W2)
- **G · ux/identity-workspace** — #103, #104, #112 — unique names, per-workspace identity, paper-cuts (tilde/probe/install/init/naming). (connect.rs → after A)
- **H · token/recall-profiles** — #89, #91, #92, #94 — tool profiles, Recall.key, focus:mentions, send-side hygiene.
- **I · conformance+eval** — #88, #93 — property state-machine suite (after #85/#86), digest-quality eval harness.

## Final
- Fable-model review agent across all branches; fix findings; then close epics #96, #113.

## Status log
- 2026-07-04: Wave 1 launched (A/B/C, background worktree agents).
- 2026-07-04: Wave 1 DONE — PRs #117 (#95), #118 (#99-#102), #119 (#87 #90). Merged into
  `issue-sweep-integration` (off origin/main), `CI_SKIP_WEB=1 make ci` GREEN, pushed.
- 2026-07-04: Wave 2a launched off issue-sweep-integration — D=ux/session-lifecycle (#107/108/109),
  F=reliability/cursor-ack-idempotent (#85/86). E (#110/111 error sweep) held for 2b.
