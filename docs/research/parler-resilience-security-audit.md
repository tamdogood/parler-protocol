# Research: full audit + battle-tested resilience, security, and token-efficiency for Parler Protocol

Date: 2026-07-04 · Method: 6 parallel researchers (2 codebase audits at HEAD `af8261c`, 4 web) +
orchestrator verification against primary sources. Raw findings in `.architect/research/` (gitignored).
Third research round. Companions: `agent-comms-protocol-hardening.md` (issues #85–#96, reliability +
token) and `parler-ux-flow.md` (issues #99–#113, flow/UX). This round audits the code **as it stands
after those issues were filed** (and #105/#106 shipped) and pulls in battle-tested patterns the first
two rounds did not cover.

## Brief (restated)

Check the open GitHub issues, audit the whole codebase, and research battle-tested technologies and
algorithms to make the protocol more resilient, give a 10/10 UX flow, be bug-free and secure, with
top-notch token-efficient agent-to-agent communication that doesn't degrade agent performance.
Decision informed: which *new* issues to file (delta over #85–#113, no duplicates) and which existing
issues the evidence re-prioritises.

## Answer first (BLUF)

1. **The known-defect backlog is execution-bound, not discovery-bound.** Of the 24 issues from rounds
   1–2, only #105 (doctor) and #106 (`parler_apply`) have shipped; **22 remain open**. The single
   highest-leverage move is to *build* the reliability lane (#85 cursor-ack, #86 idempotent send, #87
   liveness, #89 tool profiles, #90 server-side wait) rather than research more. This round adds to
   that backlog; it does not replace it.

2. **The code is clean where it counts.** A fresh security sweep at HEAD found **no critical or
   exploit-grade holes**: crypto/signature enforcement, SQL parameterisation, blob path-traversal,
   `git bundle apply` (argv-only, no shell), watch-token scope isolation, and web XSS/token handling
   all verified clean. Correctness sweep found **nothing CRITICAL**. So "bug-free and secure" is
   mostly a matter of closing a small set of *defence-in-depth and scaling* gaps, not fixing live
   exploits.

3. **The real new gaps are throttling- and contention-shaped, and both have precedented fixes:**
   - ~~**No rate limiting anywhere on the REST surface**~~ — **rate-limit half now shipped** (A1): a
     per-IP fixed-window limiter guards the whole HTTP front door (REST/A2A + the `/ws` upgrade), so the
     directory/session/`/join` routes and connection floods are throttled with `429 + Retry-After`. Wide-
     open CORS (`Any/Any/Any`) is still open. `Redeem` (invite-code join, ~40-bit codes) is now covered
     by the same per-IP HTTP budget, though a dedicated `RateKind::Redeem` (MEDIUM, A2) is still worth
     adding. The limiter is an in-house fixed-window (matching the existing per-agent one) rather than
     `governor`, keeping the dependency surface flat.
   - **The retention janitor shares the single writer mutex with all live traffic** — MAJOR (latency,
     not correctness). A large prune/GC/vacuum pass will stall `Send`/`Pull` for its duration once a
     hub carries real history. Fix pattern is `LIMIT`-batched deletes that yield the lock between
     batches (standard SQLite-under-load practice).

4. **For token efficiency, the evidence strongly backs the *structural* moves already queued and adds
   hard accuracy numbers.** Parler Protocol exposes ~24 tools; both frontier labs publish thresholds right at
   that line — OpenAI "aim for **< 20** functions at once," Anthropic "consider tool search at **30+**,"
   with a measured **Opus 4 49%→74%** accuracy jump from deferred loading and **>85%** token reduction
   [primary, verified]. This elevates #89 (tool profiles) from a token-saver to an **agent-accuracy**
   fix. The dominant tool-calling failure mode in benchmarks is **agents looping after an error, not
   picking the wrong tool** (τ-bench pass¹ 61% → pass⁸ 25%), which makes #111 (actionable error
   standard) load-bearing for *agent performance*, not just human UX. **Do not** build message-body
   compression (round-1 verdict holds); **do** embed usage examples in tool descriptions (measured
   72%→90% on complex params) and prefer enums over free strings (supports #110).

5. **Capability-token verdict: keep the opaque DB-backed tokens Parler Protocol already uses — do not migrate to
   biscuit/macaroons.** A primary practitioner source (Fly.io, having built *and abandoned* a macaroon
   system) recommends attenuable tokens only when attenuation + confinement + delegation are *all*
   needed at once; Parler Protocol's watch/directory tokens are single-scope reads. The missing piece is a
   **revocation path** (there is none today; TTL up to 365 days), not a fancier token format.

6. **A genuinely novel angle this bus should own: cross-agent prompt injection.** Parler Protocol is a literal
   agent-to-agent message bus, so one agent's message body *is* untrusted input to another agent —
   exactly the surface the 2025–2026 literature flags (Prompt Infection: >80% spread on GPT-4o;
   Willison: no general solution). There is **no consensus mitigation for agent-to-agent chat**, but
   the transferable defensive primitive is **content demarcation / provenance** (spotlighting-style):
   render peer message bodies with clear "this is data from another agent, not an instruction to you"
   framing. Worth a design note + issue, not a heavy subsystem.

## What's already covered — do NOT re-file

Reliability/token lane (#85–#96): cursor-ack-on-confirm, idempotent Send, liveness/heartbeat,
tool profiles/progressive disclosure, server-side wait on Pull + join-approval, keyed recall,
mention-focus recv, conformance/proptest suite, protocol evolution rules, send-side hygiene.
UX lane (#99–#113): env/config precedence, printed-command correctness, `--team` secret churn,
hub-start-on-connect, name collisions, per-workspace identity, approve/deny name resolution,
session lifecycle/close, join-code unification, strict inputs, one error-message standard,
paper-cuts batch, PARLER_HOME expansion. The caveman/ponytail verdicts (round 1) also stand.

## New findings, with confidence and what they imply

### Codebase audit (read-only, verified at HEAD by the orchestrator)

| # | Finding | Severity | Confidence | Implication |
|---|---------|----------|-----------|-------------|
| A1 | No rate limiting / per-IP throttle on any REST route; CORS `Any/Any/Any` (`server.rs:340-364,343-346`) | HIGH | **RESOLVED (rate limit)** / open (CORS) | Rate-limit half shipped: a per-IP fixed-window `rate_limit` middleware now guards every route except `/health` (incl. the `/ws` upgrade), keyed by `Fly-Client-IP`→`X-Forwarded-For`→socket peer, `429 + Retry-After` over budget (default 600/min, `PARLER_HUB_MAX_HTTP_PER_MIN`). Built in-house to match the existing fixed-window limiter rather than adding `governor`. **Still open:** scope CORS to known origins on `/api/session` |
| A2 | `Redeem` invite-code join has no dedicated rate limit; codes ~40 bits, registration open (`server.rs:1569-1578`, `RateKind` `:119-123`) | MEDIUM | VERIFIED | Add `RateKind::Redeem`; RFC 8628 joint entropy×attempts model |
| A3 | Janitor prune/GC/vacuum holds the single writer mutex; blocks live Send/Pull for the pass (`store.rs:329-331`, `server.rs:387`) | MAJOR (latency) | VERIFIED path / MED magnitude | `LIMIT`-batched deletes yielding the lock between batches |
| A4 | Watch/directory tokens up to 365 days, **no revocation API** (`server.rs:66-68,1514-1530`; none in `store.rs`) | LOW | VERIFIED | Add owner `revoke_watch_token`; the "expiring capability" pitch needs it |
| A5 | DM auto-open needs **zero consent** from the target (any member DMs any public *or* private-same-hub card) (`server.rs:1699-1719`) | LOW (product) | VERIFIED | Decide: does a private card mean "discoverable" or "messageable"? Channels got a gate; DMs didn't |
| A6 | `Target::Service` auto-join is ungated while channels now gate (`server.rs:1721-1729`) | LOW (product) | VERIFIED | Decide service-room trust model explicitly |
| A7 | `com.parler.observation` part passed to the watch viewer verbatim, unlike other non-text parts (`server.rs:863-870`) | LOW | MED (didn't trace field sensitivity) | Confirm intentional or reduce to a label |
| A8 | `ttl_secs: 0` mints an already-expired token (`server.rs:1514-1530`) | MINOR | VERIFIED | Floor the TTL; harmless foot-gun |
| A9 | `refs/parler/<12-hex>` ref name has no collision check (`mcp.rs:534-549`) | MINOR (latent) | VERIFIED | Use full id or check existing ref; 48 bits makes accidental collision ~impossible |

Big **checked-and-clean** set (do not churn): pull/room_messages cursor logic, redeem approval state
machine, watch-token scope isolation, `api_session` sanitisation (no ids/blobs leak), reader/writer
pool routing, blob content-address verification + path-traversal gating, `git_in` argv safety,
message-signature coverage, FTS/SQL parameterisation, card-signature enforcement, join-secret
constant-time compare, handshake/idle timeouts + connection cap, secret file modes, web XSS/token
handling. (Full detail: `01-correctness-audit.md`, `02-security-audit.md`.)

### Battle-tested patterns to adopt (web, verified)

| # | Finding | Confidence | Implication |
|---|---------|-----------|-------------|
| B1 | NATS disconnects slow consumers at **65536 msgs / 64 MiB** pending per sub, "protect the system as a whole" over buffering [primary, docs.nats.io] | VERIFIED | Bound per-connection outbound queue; disconnect slow agents → they reconnect and resume from durable cursor (Parler Protocol already has cursors) |
| B2 | `governor` = GCRA, lock-free CAS, ~10× faster than mutex under contention; 58.3M downloads [primary docs] | VERIFIED (mechanism) / MED (adoption; dependents NOT FOUND) | The Rust rate-limiter for A1/A2 |
| B3 | AWS **full-jitter** backoff is canonical; Istio #58100 shows the server must **jitter its drain timeout too**, not just clients [primary] | VERIFIED | Reconnect backoff + staggered drain to avoid a reconnect storm on deploy |
| B4 | Discord publishes a **close-code table with per-code reconnect semantics** (4004/4010-4014 = do-not-reconnect; 4008 rate-limited = reconnect); code 1001 = graceful "going away", 1006 = abnormal [primary] | VERIFIED | Give Parler Protocol close codes that tell the client whether to reconnect; emit 1001 on deploy drain |
| B5 | Any long-lived **reader** starves WAL checkpoints → unbounded WAL growth; heuristic "alarm if WAL > 2× DB"; `busy_timeout=5000` [primary + practitioner] | VERIFIED (mechanism) / MED (numbers) | Cheap WAL-size monitor; keep reader transactions short |
| B6 | Litestream = single-node DR/PITR only (no failover); LiteFS Cloud **sunset Oct 2024**, LiteFS deprioritised/pre-1.0 [primary/med] | VERIFIED | For a single-node hub, Litestream is the right (and simpler) backup path; don't reach for LiteFS |
| B7 | Fly sends **SIGINT** by default; `kill_signal`/`kill_timeout` (up to 24h dedicated) control the grace window; app must translate the signal into a WS close frame [primary] | VERIFIED | Wire SIGINT→graceful WS close (B4) during Fly deploys |
| B8 | Caddy issue #6958: proxied WS dies at **~8-10s** regardless of `stream_timeout`, **closed unresolved** [primary GitHub] | VERIFIED | Heartbeat (#87) must assume hostile proxies; if Caddy is in the path, test the real idle behaviour before trusting configured timeouts |

### Security patterns (web, verified)

| # | Finding | Confidence | Implication |
|---|---------|-----------|-------------|
| C1 | Keep opaque DB tokens: Fly.io (built+abandoned macaroons) says attenuable tokens only pay off when attenuation+confinement+delegation all needed at once [primary practitioner] | VERIFIED (as opinion) | Don't migrate to biscuit/macaroons; add revocation (A4) instead |
| C2 | MCP spec: servers **MUST NOT** accept tokens not issued for them (token passthrough anti-pattern); tool-poisoning + no-reapproval "rug pulls" are the best-documented MCP attacks [primary spec + Invariant Labs 2025-04] | VERIFIED | Parler Protocol as an MCP server: never proxy a downstream token; surface tool-definition changes for re-approval |
| C3 | Constant-time compare is **not optional even on low-jitter networks**: 100ns-20µs timing differences are remotely recoverable (Crosby/Wallach 2009); "timeless" concurrency attacks (USENIX 2020) ignore jitter entirely [primary] | VERIFIED | Parler Protocol's join-secret compare is already constant-time (verified); confirm the same for any future secret-equality check |
| C4 | RFC 8628: entropy × attempt-limit × lifetime must be sized **jointly**; worked example 8-char/~34.5-bit code + 5 attempts ≈ 2⁻³²; WPS = canonical failure (never leak partial-correctness of a code) [primary] | VERIFIED | Sizing guide for A2 (invite-code throttling) |
| C5 | Cross-agent prompt injection: Prompt Infection >80% spread on GPT-4o; **no consensus mitigation for agent-to-agent chat**; transferable primitive is spotlighting/demarcation + Meta "Rule of Two" [arXiv/MED + Willison/primary] | MIXED | Novel bus-specific issue: demarcate peer message bodies as untrusted data, not instructions |

### Token efficiency & agent-facing UX (web, verified)

| # | Finding | Confidence | Implication |
|---|---------|-----------|-------------|
| D1 | OpenAI **< 20** functions/turn (soft); Anthropic **30+** → tool search; **Opus 4 49%→74%**, Opus 4.5 79.5%→88.1%; **>85%** token cut; degradation "once you exceed 30–50 tools" [primary] | VERIFIED (thresholds, >85%, 30–50) / UNVERIFIED (exact 49→74 deltas — on the linked advanced-tool-use post, not re-fetched) | Parler Protocol's ~24 tools sit at the line → #89 is an **accuracy** fix, not just tokens |
| D2 | Dominant tool-calling failure = **looping/non-recovery after an error**, not wrong selection; τ-bench pass¹ 61% → pass⁸ 25% [primary] | VERIFIED | #111 (actionable errors) is load-bearing for agent performance; error text is higher-leverage than description polish |
| D3 | Tool-use **examples in descriptions** moved accuracy **72%→90%** on complex params [primary] | VERIFIED | Add input examples to Parler Protocol's tool descriptions |
| D4 | Enums make invalid states unrepresentable; minimise required params; academic: fewer targeted tools raised selection accuracy **93.1% vs 87.1%** (76.8 vs 60.9 on medium) [primary + arXiv/MED] | VERIFIED (guidance) | Supports #110 (strict inputs) |
| D5 | Instructed brevity (not mechanical stripping) +26pp on math/science, **reverses** size hierarchies [arXiv 2604.00025, abstract VERIFIED]; a counter-case on cross-sentence integration (BoolQ) is reported deeper in the paper | VERIFIED (main) / UNVERIFIED (BoolQ counter-case, single source) | Refine #94: "be brief" helps status/result/handoff messages; risky for messages the receiver must integrate across many sentences |
| D6 | MCP **Tasks pulled from core → extension** with a new stateless lifecycle in the 2026-07-28 RC; `tools/list` now cacheable via **`ttlMs`/`cacheScope`** (SEP-2549) [primary spec blog, VERIFIED verbatim] | VERIFIED | Don't build #90 on MCP-native Tasks (still churning); #89 can lean on `ttlMs` caching |
| D7 | A2A vs MCP: secondary summary claims A2A **3.1× fewer tokens / 39% cheaper** on complex orchestration (arXiv 2603.22823) | **UNVERIFIED** — the abstract I fetched states objectives only; the numbers are from a secondary source, PDF extraction failed twice | Do not act on this number without reading the full PDF |

## Expert positions map

- **Anthropic engineering** (primary, 2025–26): fewer, richer, consolidated tools; actionable errors;
  examples in descriptions; deferred loading past ~30 tools. COI: sells the models; the architectural
  claims are locally testable and now have published numbers.
- **OpenAI** (primary): < 20 functions/turn; enums to make invalid states unrepresentable; "pass the
  intern test." Consistent with Anthropic directionally.
- **Fly.io engineering** (primary practitioner): opaque revocable tokens beat macaroons/biscuit for
  single-scope use — dissents from the research-stage biscuit-for-agents enthusiasm (arXiv IBCT/AIP).
- **Simon Willison** (primary): after 2.5+ years prompt injection has *no general solution*; only
  harm-reduction (dual-LLM, spotlighting). The pessimistic anchor for C5.
- **NATS / Discord** (primary vendor docs): both converge on *disconnect the misbehaving connection*
  over buffering/tolerating — the design template for backpressure (B1) and rate-limit response (B4).

## Recommended new issues (delta over #85–#113)

Security / correctness (from the audit):
1. **REST-surface rate limiting + CORS tightening** (A1, HIGH) — per-IP `governor`/`tower-governor`
   layer on `/api/*` and `/join/:code`; scope CORS on `/api/session`.
2. **`Redeem` invite-code throttling** (A2, C4, MEDIUM) — add `RateKind::Redeem`, size per RFC 8628.
3. **Janitor writer-lock contention: batched deletes** (A3, MAJOR) — `LIMIT`-batched prune/GC that
   yields the writer mutex between batches.
4. **Watch/directory token revocation API** (A4, C1) — owner-invokable revoke; floor `ttl_secs` (A8).
5. **DM + service-room consent model** (A5, A6) — one product decision: does discovery imply
   messageability, and do service rooms need the channel gate?
6. **Watch-viewer observation-field disclosure** (A7) — confirm intentional or reduce to a label.

Resilience (from B):
7. **Per-connection outbound backpressure** (B1) — bound the send queue; disconnect slow agents to
   resume from cursor, NATS-style, rather than buffering unboundedly.
8. **Graceful drain on deploy** (B3, B4, B7) — SIGINT→WS close (1001) with a "reconnect" close code and
   jittered drain; pairs with #87's degraded-mode work.
9. **SQLite durability ops** (B5, B6) — WAL-size monitor ("alarm if > 2× DB"), `busy_timeout`, and
   Litestream backup wired for the single-node hub (reconciles the roadmap in `storage-and-memory.md`).

Token efficiency / agent UX (fold into existing where noted):
10. **Tool descriptions: add input examples + enums** (D3, D4) — batch into #110/#111; measured wins.
11. **Cross-agent prompt-injection demarcation** (C5) — design note + issue: render peer bodies as
    untrusted data; document the Rule-of-Two posture for tools that act on message content.
12. **Refine #94 brevity wording** (D5) — instructed brevity for status/result/handoff, not for
    integrate-heavy messages; A/B via the #82 token metrics.

## Open questions (next round's input)

1. **A2A-vs-MCP token numbers (D7)** — read arXiv 2603.22823 in full; the 3.1×/39% figures are
   currently unverified secondary claims. Only then decide whether a structured A2A-style envelope
   would cut Parler Protocol's per-message overhead vs its current MCP shape.
2. **Brevity counter-case (D5)** — confirm the BoolQ/cross-sentence-integration result against the full
   paper before it shapes #94's copy.
3. **Backpressure thresholds for *this* workload** — NATS's 65536/64 MiB are for a high-throughput bus;
   size Parler Protocol's per-connection bound against its actual tens-of-agents profile (needs a load probe).
4. **Real Caddy/Fly WS idle behaviour** (B8) — live probe against parler-hub.fly.dev to set the #87
   heartbeat interval; the configured timeout is not trustworthy per #6958.
5. **Tool-usage distribution** — instrument which of the 24 tools sessions actually touch, to design
   the #89 core-vs-deferred split on evidence rather than guess.

## Sources (dated, tier-labelled; fetched this session)

- arXiv 2604.00025 — brevity abstract [primary, 2026-03] — abstract confirmed via arxiv.org/abs.
- blog.modelcontextprotocol.io/posts/2026-07-28-release-candidate — Tasks→extension + SEP-2549 `ttlMs`/`cacheScope` [primary, 2026-07-28] — quoted verbatim.
- platform.claude.com/docs/.../tool-search-tool — >85% token cut, 30–50 tool degradation threshold [primary, 2026] — confirmed.
- github.com/caddyserver/caddy/issues/6958 — WS ~8-10s drop, closed unresolved [primary, 2025].
- arXiv 2603.22823 — agent-protocol comparison [primary, 2026-03] — **abstract states objectives only; 3.1×/39% NOT in abstract**.
- docs.nats.io slow_consumers; docs.rs/governor; aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter; docs.discord.com opcodes-and-status-codes; sqlite.org/wal.html; fly.io/blog all-in-on-sqlite-litestream; fly.io/docs configuration [all primary].
- biscuitsec.org FAQ; fly.io/blog/api-tokens-a-tedious-survey; modelcontextprotocol.io security_best_practices; invariantlabs.ai tool-poisoning (2025-04-06); simonwillison.net (2025-04-09, 2025-06-13); RFC 8628; OWASP WebSocket cheat sheet; Crosby/Wallach 2009; USENIX Security 2020 timeless-timing [all primary].
- anthropic.com/engineering/writing-tools-for-agents + advanced-tool-use; developers.openai.com function-calling; arXiv 2406.12045 (τ-bench); arXiv 2605.24660 (tool count); gorilla.cs.berkeley.edu BFCL v3 [primary/academic].
- Secondary (pointers only, not load-bearing): chatforest.com RC summary, mcp.directory, arXiv 2410.07283 (Prompt Infection, PDF unreadable — figure from abstract), CaMeL/spotlighting summaries.
