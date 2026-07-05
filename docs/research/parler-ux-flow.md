# Research: a seamless, 10/10-UX Parler Protocol flow — setup to session

Date: 2026-07-03 · Method: 4 parallel researchers (2 codebase audits, 2 web) + orchestrator
verification against source. Raw findings in `.architect/research/` (gitignored). Companion to the
protocol-hardening round (issues #85–#96) — that round owns reliability + token efficiency; this
round owns **flow coherence and UX**, and deliberately does not overlap it.

## Brief (restated)

What concrete changes make Parler Protocol's end-to-end flow — install → `parler connect` → first message →
sessions → handoff → memory — seamless, bug-free, and 10/10 UX, with special weight on setup and
with no redundant steps or surface? Decision informed: which GitHub issues to file for implementing
agents. Constraints: keep the security model; additive/backward-compatible wire changes only;
`web/` is human-driven and out of scope.

## Answer first

The setup flow's *happy path* is genuinely good (idempotent config writers, hub probing, specific
restart hints), but it is undermined by three **verified blockers**, all of the same species:
**instructions Parler Protocol itself prints don't work, and the failure is silent — the agent lands on the
wrong hub and "works."** In-flow, the tool surface is one tool short of a working MCP code-handoff
(the banner tells the model to call a tool that doesn't exist), and the session join gate — Parler Protocol's
most distinctive feature, with no peer precedent — has an owner-offline dead end and asymmetric
name resolution inside a single flow. External evidence says the fixes are known patterns: every
printed command must be copy-paste-runnable (ngrok/Stripe), env/config precedence must be one rule
everywhere, `doctor` must be advertised at every failure point (flutter), and approval gates need
the two standard escape hatches (Tailscale: pre-approval + notification). ~14 issues, below.

## Verified defects (the load-bearing set)

Each was re-verified by the orchestrator reading the cited code this session. Confidence VERIFIED
unless noted.

**Setup blockers**

1. **Env/config precedence is incoherent, so "move your agents to another hub" silently doesn't.**
   `parler mcp` reads `PARLER_HUB`/`PARLER_NAME`/`PARLER_ROLE` only when no `config.json` exists
   (`crates/parler-cli/src/mcp.rs:117-124`), so re-running `parler connect --local/--team/--shared`
   rewrites env the agent ignores forever — contradicting README's "move them deliberately".
   Meanwhile the CLI's generic `agent()` helper *does* apply live `PARLER_HUB` over saved config
   (`crates/parler-cli/src/lib.rs:512-516`), and `PARLER_JOIN_SECRET` is read live everywhere
   (`parler-connector/src/client.rs:120`). Same machine, same env: CLI and MCP can talk to
   *different hubs*. → Implication: one precedence rule (env > saved config) applied identically in
   both paths. Would change conclusion: nothing short of removing saved-config hub pinning.

2. **The teammate one-liner `--team` prints is broken.** `connect.rs:873,903` print
   `PARLER_HUB=… PARLER_JOIN_SECRET=… parler connect`, but `cmd_connect` (`lib.rs:553-589`) and
   `ConnectArgs` read neither var; `Hub::Shared` is the hard-coded default. A fresh teammate running
   the printed line is wired to the **public hub with no secret**, silently.

3. **Every private-hub connect line uses shell-env prefixes on `claude mcp add`** (hub startup log
   `parler-hub/src/main.rs:178-180`, hub landing page, `deploy/private/README.md:41,53`), which do
   not persist into the stored MCP server config — the agent bootstraps onto the public hub with no
   secret and appears to work. `connect.rs` itself knows the correct `-e/--env` form (:401-404).

**Setup majors** — 4. `--team` re-run mints a fresh secret while the running hub enforces the old
one (`connect.rs:553-557`); 5. `--local/--team` never start (or daemonize) the hub, and an agent
started before the hub dies with only an `mcp.log` breadcrumb; 6. every Claude Code user on the
shared hub defaults to the name "claude-code" (`connect.rs:785`), making name-DMs ambiguous and
letting `--verify`'s case-insensitive name match (`lib.rs:665`) confirm a stranger's agent;
7. two workspaces on one machine share one identity (single `PARLER_HOME` per host app,
`connect.rs:783-794`) and one global `active_session` file (`lib.rs:1332`); 8. `parler doctor` — the
designated recovery tool — is mentioned nowhere a user would look (README, connect output, error
strings) (`mcp.rs:38-40` vs `connect.rs:858`); 9. README's `-e PARLER_HOME=~/.parler-bob` leaves a
literal unexpanded `~` (`config.rs:44-47` does no expansion).

**Flow majors** — 10. the handoff banner instructs the receiving LLM to `apply via parler_apply`
(`mcp.rs:1154`) — **no such tool exists** (apply is CLI-only, `lib.rs:73`); `parler_push` likewise
answers "The peer runs: `parler apply …`", a dead end for an MCP-only agent, and `parler_fetch`
defaults its output to the host's opaque cwd (`mcp.rs:484,495`); 11. `parler_approve_join` requires
the raw 56-char id while `parler_send` resolves names — one flow, two rules (`mcp.rs:685` vs
`:720`), and the param text "the joiner's id to resolve" implies the opposite; 12. owner-offline
dead end: join requests surface only when the owner happens to call a tool; pending joiners get no
timeout/owner-gone signal, and `close_session` neither revokes the key nor resolves pending
requests — joiners can poll a dead session forever (`mcp.rs:1048-1052,1112,1173`); 13.
`parler_join` and `parler_join_session` redeem the same code space with divergent outcomes (an
error-shaped "pending" vs a proper join with backlog) (`agent.rs:192`); 14. `close_session` leaves
you a hub member — nothing is closed for anyone, and the CLI has no session leave at all
(`mcp.rs:1171-1184`, `lib.rs:192-239`).

**Minors (batchable)** — "exactly one of room/to/service" unenforced (silent `room` precedence,
`mcp.rs:462-470,708,758`); `parler_invite` kind typos silently become DM (`mcp.rs:436` vs enum at
`:1267`); `recv since` flips three behaviors at once (`mcp.rs:789-813`); `parler_register` with no
args silently downgrades a `PARLER_PUBLIC` card and drops env tags (`mcp.rs:555-563`);
`PARLER_SESSION_KEY` join failure is stderr-only, invisible to doctor (`mcp.rs:62`); ~15
`unexpected reply: {other:?}` debug-dump error sites in `agent.rs`; naming collisions across
surfaces (prompt `parler_session_handoff` vs tool `parler_handoff` vs CLI `consolidate`;
`parler_card` param `id` accepts names); `probe_hubs` mutates process env and can mint an identity
as a side effect of a read-style check (`lib.rs:594-628`); install.sh proceeds silently when
checksum verification is impossible (`scripts/install.sh:65-78`); no Linux-ARM binary; `parler
init` still listed although the story is "no init needed".

**Checked and clean (don't churn):** config writers (idempotent, merge-preserving, refuse malformed
files actionably), bare-re-run "(kept)" semantics, missing-host explanations, zero-to-DM in 2 steps,
open/join session digests, denial messaging, join-gate ownership checks (the Invite self-join bypass
is closed), 0600 secret files, installer PATH handling.

## External evidence (what "10/10" looks like)

- **Copy-paste-runnable commands with values pre-filled** are the ngrok/Stripe onboarding core;
  Stripe's `stripe login` is the canonical auth handoff (readable pairing code, auto-opened browser,
  1s poll, headless fallback) [primary, 2026-07]. Parler Protocol's equivalents are exactly the broken lines
  in blockers 2–3.
- **clig.dev error canon**: what happened, why, and the *exact fix command*; suggest the next
  command in multi-step flows; "if you change state, tell the user" [primary, 2026-07].
- **`doctor` pattern** (flutter/brew): per-check ✅/❌ plus the exact command that fixes each failure,
  advertised as re-runnable — Parler Protocol has the command but not the advertisement [secondary, 2026-07].
- **Tool surface**: Anthropic — "Claude's ability to pick the right tool degrades once you exceed
  30–50 available tools"; tool-search recommended at 10+ tools; keep 3–5 hot tools loaded
  [primary, fetched 2026-07-03]. Parler Protocol's 23 tools + 2 prompts is below the degradation band but
  well above the "no action needed" band — and issue #89 (tool profiles) already owns the fix; this
  round's contribution is *not adding tools needlessly* and merging redundant ones.
- **Approval-gate precedent**: no surveyed agent framework (A2A v1.0, AutoGen, Claude agent teams,
  AgentMail, agenthub) gates session join on per-request owner approval — Parler Protocol's gate is a
  differentiator with no peer, so its UX has no ecosystem crutch. The closest analogue is Tailscale
  device approval, whose documented pain (approver away → blocked) is mitigated by **pre-approved
  keys** and **auto-approval hooks** [primary, 2026-07]. GitHub's device flow adds the other missing
  piece: codes **expire (15 min)** and denial is a terminal, explicit state [primary, 2026-07].
- **MCP install friction is normal and worth designing around**: Anthropic's own list (runtimes,
  JSON editing, no discovery); one-click paths exist (`.mcpb` bundles — not yet in Cursor; Cursor
  deeplinks; `claude mcp add --scope project` → committable `.mcp.json`; official registry preview
  since 2025-09) [primary, 2026-07]. A 1,400-server scan found 38.7% shipped no auth — setup
  complexity correlates with users skipping security [secondary, 2026-01].

## Disputes / tensions

- Anthropic ships two install philosophies (GUI `.mcpb` "no terminal" vs CLI-first `claude mcp
  add`) — Parler Protocol already bets on the CLI path via `parler connect`; `.mcpb` is a future option, not
  a correction.
- A2A's adoption is contested ("supported by ≠ used by", one analyst, 2026) — relevant only as a
  caution against protocol surface growth, not a Parler Protocol decision.
- No public per-step onboarding funnel dataset exists (NOT FOUND); "time to first hello world"
  drop-off claims are vendor-blog tier. Treat TTFHW as a design compass, not a measured benchmark.

## Expert positions map

Thin by finding: no canonical individual authority on multi-agent interop UX surfaced. Worth
tracking: Brian Warner (magic-wormhole; pairing-code design), Anthropic engineering (tool use +
desktop extensions posts), Rost Glukhov (A2A adoption skepticism; independent blog).

## Open questions

- Hub behavior when two live sockets share one agent id (dup-connection semantics) — untraced;
  resolve by reading `parler-hub` connection registry or a 2-socket integration test (feeds the
  per-workspace-identity issue).
- Whether `--verify`'s name match can actually confirm a stranger's agent on the live shared hub
  (med confidence) — resolve with a two-identity e2e test.
- Real teammate TTFHW for the private-hub flow after fixes (currently 6 manual steps, one
  undocumented, one defective) — measure by scripting the flow end-to-end in CI.
- `.mcpb` bundle as a future zero-terminal install path once Cursor support lands — revisit when
  the Cursor feature request closes.

## Citations

Code: file:line references above, this repo @ branch barcelona, all read 2026-07-03.
Web (all fetched 2026-07-03): Anthropic tool-search docs [primary]; Anthropic Desktop Extensions
post [primary]; clig.dev [primary]; Stripe CLI login docs [primary]; Tailscale device-approval KB
[primary]; GitHub device-flow docs [primary]; magic-wormhole docs [primary]; a2a-protocol.org spec
[primary]; fly.io launch docs [primary]; astral.sh uv post [primary]; MCP registry + mcpb blog
posts [primary]; ngrok quickstart [primary]; Claude Code MCP + agent-teams docs [primary];
bloomberry 1,400-server scan [secondary]; glukhov.org A2A analysis [secondary, single-author];
AgentMail/TechCrunch [secondary]; ottogin/agenthub repo [primary]; AutoGen conversation-patterns
docs [primary]; Syncthing device-ID docs [primary]; flutter-doctor guides [secondary]; TTFHW
vendor posts [secondary].
