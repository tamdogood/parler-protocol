# Research: hardening the agent-to-agent connection — protocol correctness + token efficiency

Date: 2026-07-03 · Author: architect-research run (Claude) · Raw findings: `.architect/research/` (gitignored)

## The brief

How should Parler further improve the connection between agents? Two goals: (1) the protocol
works *flawlessly* (no lost/duplicated messages, robust reconnect), and (2) it wastes no
unnecessary tokens on the agent side. Study [ponytail](https://github.com/DietrichGebert/ponytail)
and [caveman-compression](https://github.com/wilpel/caveman-compression) for lessons, plus survey
the broader state of the art. Deliverable: an architect plan broken into GitHub issues (no
implementation here).

## Answer first (BLUF)

1. **The protocol is *good* but not *flawless*. Two verified correctness gaps remain**, both
   acknowledged or visible in the code itself:
   - **A pull-loss window**: `Store::pull` advances the member cursor server-side *before* the
     reply reaches the client (`store.rs:591-597`). If the connection drops after the hub commits
     the cursor but before the client receives the batch, the connector's transparent
     retry-once re-pulls *past* those messages — they are silently skipped from the agent's
     view. Every surveyed production system (Kafka, NATS JetStream) converges on "advance the
     cursor only after the consumer confirms" [primary docs, 2026]. Fix: additive client-ack
     cursor advance.
   - **Duplicate sends on retry**: the connector's reconnect-and-retry-once explicitly documents
     "a duplicate line is a benign worst case" (`agent.rs:102-105`). For a chat line that's
     benign; for a session seed or a handoff instruction it's noise every member pays tokens to
     re-read. Fix: additive idempotency key on `Send`, hub-side dedupe.
2. **Token efficiency: the render-side diet (#84) is done and solid; the remaining wins are
   structural, not compressive.** The three biggest remaining line items:
   - **Tool-list overhead**: 23 tools ≈ 11,030 B of `tools/list` paid by *every agent every
     session* (`mcp.rs:1471`). Most sessions touch ~6-8 tools. Progressive disclosure /
     profiles is the SOTA answer (Anthropic's code-execution-with-MCP post reports 98.7%
     context reduction from not preloading tool surface [first-party, 2025-11]).
   - **Empty-poll spend**: long-poll (`wait_secs`) exists but silently degrades to
     poll-and-burn when the push subscription never activated, and `join_session` approval
     waits are manual re-polls. Server-side wait on `Pull` closes both.
   - **Protocol-touching tail already designed in `tasks/backlog.md`** (P2.1 keyed recall,
     P2.2 mention-focus recv) — promoted to issues.
3. **Do NOT build message-body compression (the caveman mechanism).** The independent evidence
   is against it: a 24-prompt benchmark found the literal instruction "be brief." matched the
   caveman plugin on both quality and tokens [independent blog, 2026]; the CAVEWOMAN paper
   (arXiv 2606.24083) found measurable accuracy penalties across models/benchmarks with no
   universally beneficial configuration; the LLMLingua literature flags instruction-following
   as particularly vulnerable. **What transfers instead is caveman's *verification* pattern**:
   its fact-to-question LLM-judge benchmark is exactly the harness Parler needs to prove its
   *digests* (join backlog digest, session-digest fact) don't drop decision-relevant facts.
4. **Ponytail's transferable idea is the pre-action gate, not the plugin.** Its "laziness
   ladder" (don't emit what already exists / can be referenced) maps onto send-side hygiene:
   reference a fact or bundle instead of pasting content the receiver can fetch. Parler
   already has ponytail's other good ideas: an intensity dial (`PARLER_MCP_VERBOSE`), budget
   regression tests (`TOOL_SPECS_BUDGET`), and debt tracking (backlog). Ponytail's headline
   numbers (54% LOC / 22% tokens) are self-benchmarked with no independent reproduction found
   — adopt the pattern, don't cite the numbers.
5. **Make "flawless" testable, not aspirational**: a proptest-state-machine model of the
   cursor/room/session invariants plus disconnect-injection tests. `proptest-state-machine`
   and Toxiproxy are the standard tools; Discord's jittered-heartbeat + zombied-connection
   detection is the reference liveness pattern.

## What's already done (do not re-propose)

- **#84 (commit 2018081)**: client-side render diet — tool descriptions 5,261→4,304 B, digest
  joins (7,863→1,458 chars on a 100-msg backlog), bounded recv (30) / auto-pull (10),
  1,200-char message truncation with lossless refetch, compact roster/discover. Budget
  regression tests enforce ceilings.
- **#82 (commit 6210e92)**: hub-side per-message token *estimation* (observability only).
- The capped-render losslessness invariant is real and tested
  (`recv_caps_batch_but_drains_losslessly`): cursors advance only through returned batches, so
  caps never skip content. The remaining loss window is the transport one described above.
- Retention is now default-on (30 days, 10k msgs/room floor, 500 facts, 14-day blob TTL —
  `main.rs:70-91`). `docs/storage-and-memory.md` still says "no retention" — doc drift.

## Key findings with confidence and implications

| # | Finding | Confidence | Implication |
|---|---------|-----------|-------------|
| F1 | Pull advances cursor before delivery confirmation; drop+retry skips messages | VERIFIED (code read: `store.rs:591-597`, `agent.rs:98-114`) | Highest-priority correctness fix; additive `ack` on Pull |
| F2 | Send retry can double-post; acknowledged in code | VERIFIED (`agent.rs:102-105`) | Idempotency key, additive |
| F3 | 23 tools / 11,030 B tools-list paid per agent-session | VERIFIED (budget test `mcp.rs:1471-1531`) | Tool profiles / progressive disclosure |
| F4 | `wait_secs` silently no-ops when push never activated; join approval is manual re-poll | VERIFIED (`mcp.rs:795-807`, `mcp.rs:1105-1106`) | Server-side wait; honest degraded-mode signal |
| F5 | Grammar-strip compression's benefit over "be brief" is absent-to-negative | VERIFIED (≥2 independent: Max Taylor 24-prompt benchmark; CAVEWOMAN arXiv 2606.24083; LLMLingua survey) | Don't compress message bodies |
| F5b | Counter-paper: brevity constraints *improved* accuracy up to 26pp on some benchmarks | DISPUTED (single paper, not fetched in full) | Doesn't rescue mechanical stripping; consistent with "be brief" norms |
| F6 | Ponytail's 54%/22%/20%/27% figures are self-reported; no independent repro | UNVERIFIED (single origin) | Use the ladder pattern, not the numbers |
| F7 | Code-execution-with-MCP: 150K→2K tokens by progressive disclosure | UNVERIFIED figure (single first-party source) but pattern corroborated by MCP spec direction (tools/list caching, resource links) | Motivates F3 fix |
| F8 | Production consensus: at-least-once + idempotent handling; "cursor advances only after confirm"; no surveyed system advances on send | VERIFIED (Kafka/NATS/Discord docs) | Design template for F1/F2 |
| F9 | serde enums without a catch-all variant reject unknown variants (breaking for old clients); structs are safe (no `deny_unknown_fields` anywhere in parler-protocol) | VERIFIED (grep + serde issue #2634) | Add unknown-variant tolerance guidance + tests to protocol |
| F10 | Hub replies gracefully to malformed/unknown frames (`server.rs:1197-1205`), keeps connection | VERIFIED (code read) | Conformance suite codifies this |
| F11 | MCP push (`resources/subscribe` notifications) is spec'd but rarely implemented by clients | med (secondary, 2026) | Don't bet the inbox on MCP notifications; keep long-poll primary |
| F12 | Discord resume = session_id+seq replay; jittered heartbeats; zombied-conn detection. Phoenix is at-most-once with no replay (open issue since 2014) | VERIFIED (official docs) | Parler's durable-cursor design is already the stronger pattern; add heartbeat/liveness |

## Expert positions map

- **Anthropic engineering** (first-party, 2025-11): tool definitions and intermediate results
  are the dominant context cost; progressive disclosure + filtering in an execution
  environment beats preloading. Conflict of interest: sells the models either way; the
  architectural claim is testable locally.
- **Max Taylor** (independent, 2026): dedicated compression tooling failed to beat a two-word
  instruction. No COI found.
- **CAVEWOMAN authors** (academic, 2026): linguistic compression carries measurable accuracy
  penalties; unfavorable for high-reliability tasks. Opposed by the brevity-constraints paper
  (F5b) — the genuinely open question is *instructed brevity* vs *mechanical stripping*; the
  evidence only supports the former.
- **HN commentariat on ponytail** (2026): the value is "essentially just these rules"; the
  plugin machinery is boilerplate. Consistent with adopting the rules-as-tool-description
  approach rather than a subsystem.

## Open questions

1. **F5b resolution**: fetch and read "Brevity Constraints Reverse Performance Hierarchies in
   Language Models" in full — does instructed brevity on *inter-agent messages* improve or
   harm downstream task completion? Cheap experiment: A/B a "be brief" line in `parler_send`'s
   description against the current text using the #82 token metrics.
2. **Actual tool-usage distribution**: P2.3 (tool merge) and the profile split both want usage
   evidence. The hub can already count tool-shaped traffic; an `mcp.rs` debug counter would
   settle which tools belong in the core profile.
3. **Fly/Caddy idle behavior against long-lived WS**: community reports of proxies dropping
   WS conns early (Caddy issue #6958); needs a live probe against parler-hub.fly.dev to size
   the heartbeat interval.
4. **MCP Tasks primitive** (2025-11-25 revision, experimental): if it stabilizes, approval
   waits (join_session) map onto it natively; re-check the spec in a quarter.

## Sources (fetched this session)

Primary: modelcontextprotocol.io spec + versioning pages; anthropic.com/engineering
(code-execution-with-MCP; writing tools for agents); docs.discord.com gateway; docs.slack.dev
RTM changelog; hexdocs.pm Phoenix channels; docs.nats.io JetStream consumers; github.com
(ponytail, caveman-compression, serde #2634/#2121, tokio-tungstenite #35, caddy #6958,
phoenix #576, proptest, toxiproxy); arxiv.org (2606.24083 CAVEWOMAN, 2310.05736 LLMLingua,
2411.02820 DroidSpeak, 2510.26585, 2503.18891 AgentDropout, 2410.12388 survey);
linuxfoundation.org A2A announcement. Secondary: maxtaylor.me caveman benchmark; HN 48527946;
assorted med-confidence blogs (tagged in raw findings). Codebase: `crates/parler-cli/src/mcp.rs`,
`crates/parler-hub/src/{store,server,main}.rs`, `crates/parler-connector/src/agent.rs`,
`crates/parler-protocol/src/{hub,types,lib}.rs`, `tasks/backlog.md`, `tasks/lessons.md`.

## The plan (filed as GitHub issues)

Epic A — protocol correctness: (A1) ack-based cursor advance closing the pull-loss window;
(A2) idempotent Send; (A3) heartbeat + push-subscription self-healing + honest degraded mode;
(A4) proptest-state-machine + disconnect-injection conformance suite, plus documented
schema-evolution rules (additive-only, enum catch-all guidance).

Epic B — token efficiency: (B1) tool profiles / progressive disclosure for the 11KB tool list;
(B2) server-side wait on Pull + join-approval wait (kill empty polls); (B3) P2.1 keyed recall;
(B4) P2.2 mention-focus recv; (B5) digest-quality eval harness (caveman's QA pattern);
(B6) send-side hygiene norms (ponytail's ladder as tool-description text + a size nudge wired
to #82 estimates).

Epic C — housekeeping: (C1) storage doc retention drift fix.
