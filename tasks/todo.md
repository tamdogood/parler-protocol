# Security audit remediation

## Plan
- [x] Bind signed autonomous messages to their resolved room and reject durable signed-UID replays, with negative tests.
- [x] Bound hostile hub inputs and persistent writes, add global upload backpressure, and close anonymous private-directory access.
- [x] Harden local secret/capability persistence, proxy-address handling, and unsafe private-hub startup.
- [x] Upgrade and harden the desktop runtime, then pin privileged CI/CD actions to immutable revisions.
- [x] Update security/operator documentation, run targeted tests plus the full repository and desktop gates, and self-review the complete diff.

## Risks
- Protocol changes must remain additive for deployed clients; enforce room binding in verification/runtime code without changing existing wire frames.
- Rate limits and quotas must preserve normal conversation catch-up, file transfer, and local/private hub workflows while failing closed under abuse.
- Desktop and action upgrades may contain breaking changes; verify packaging/build behavior rather than relying only on dependency audit output.

## Review
- Autonomous execution now validates the signed channel/service/DM-recipient target, atomically
  reserves `(author, uid)` per receiving identity before a host action, and rejects cross-room,
  cross-process, restart, and relay-id replays. Explicit pre-action failures release the reservation;
  crashes fail closed with an at-most-once posture.
- The hub now bounds structured fields/frames, all authenticated operations, aggregate accepted
  uploads, and durable rooms/tokens/keyed facts. Quota check-and-write sections are race-free, proxy
  headers are opt-in, anonymous HTTP hub-scope directory reads require a capability, and a network
  private hub cannot start without a join secret.
- CLI and desktop capability files use atomic owner-only replacement. The Electron renderer is
  sandboxed with a narrow CSP and HTTPS-only external navigation; the current lockfile audits clean.
  Release actions are immutable-SHA pinned and CLI/DMG/container artifacts receive provenance.
- Negative tests cover context/replay rejection, concurrent quota and upload exhaustion, private
  directory denial, proxy spoof handling, startup binding, secret permissions, and unsafe URLs.
  `make ci`, standalone `actionlint` 1.7.12, workflow YAML parsing, and `git diff --check` pass.
- Known architectural limits remain explicit rather than being mislabeled as fixed: the hub operator
  sees plaintext, and blob transfer is still single-frame/non-resumable even though accepted
  concurrent uploads are bounded. No unresolved high-severity finding was found in the final diff.

---

# Simplify first-use documentation

## Plan
- [x] Make one three-step onboarding path canonical: install/connect, start or resume a conversation, share the printed join command.
- [x] Rewrite the protocol README and docs index around progressive disclosure; add one beginner guide and keep rooms, sessions, MCP, workers, and hub internals in advanced guides.
- [x] Add a documentation maintenance contract so user-facing changes stay aligned across the protocol repo and `parler-web`.
- [x] Mirror the same information architecture and wording in the website landing page, quickstart, concepts, conversation guide, navigation, and developer README.
- [x] Run both documentation/build gates and review both repository diffs for command, support, security, and terminology drift.

## Risks
- The shorter path must not imply continuous visible support for hosts beyond Claude Code, Codex, and OpenCode.
- A conversation key is a bearer capability, and the shared hub sees plaintext; both warnings must remain visible without overwhelming first use.
- Low-level commands remain supported, so consolidation must move them behind clear links rather than erase their reference documentation.

## Review
- Replaced the 800-line root README with a 192-line first-use page and added one canonical five-minute guide.
- Kept visible-host support, bearer-key admission, and plaintext hub boundaries explicit in both repositories.
- Corrected stale website claims: CLI and MCP low-level sessions now document immediate admission by default and explicit approval opt-in, matching the tested implementation.
- Verified the protocol repository with `make ci` and the website with `npm run build` (122 static pages).

# Session viewer deep links

## Plan
- [x] Add one canonical `https://www.parlerprotocol.com/hub#sessions&k=<WATCH>` formatter and use it for CLI/MCP session-view output.
- [x] Add regression coverage for the exact deep-link shape and update the maintained viewer docs.
- [x] Run targeted tests, the full Rust gate, and self-review the focused diff.

## Risks
- The WATCH code is a bearer capability; keep the raw code and link behavior unchanged in scope, and do not put the seed or join key in the URL.
- The web viewer URL is a frontend fragment contract, so the exact `#sessions&k=` spelling must remain stable.

## Review
- Added one `session_view_link` formatter used by low-level `session open`/`session watch`, flagship `conversation`, and MCP watch/open results. The existing raw WATCH code remains available for compatibility.
- Added exact-format and real MCP hub-path regression tests; trimmed the MCP open-session output to keep its 960-byte context budget.
- Updated the README and maintained session/communication guides with the ready-to-open deep-link shape. No protocol or hub behavior changed.
- `scripts/verify.sh --rust-only` and `make ci` pass, including build, Clippy, all workspace tests, docs, smoke, advisories, sources, licenses, and bans. Self-review found no remaining findings.

---

# Current usage documentation refresh

## Plan
- [x] Establish one canonical “best way to use Parler now” flow from the shipped CLI behavior:
      install/connect, create or join a visible conversation, choose a host, and use approval/local
      mode deliberately.
- [x] Align `README.md`, maintained user guides, troubleshooting, marketing/registry copy, and the
      separate `parler-web` quickstart/docs/FAQ/homepage with that flow and an explicit support matrix.
- [x] Bring contributor docs and the visible-host extension contract in sync with the actual source
      layout, bounds, permission behavior, and required provider tests.
- [x] Build and inspect the website, run the protocol repository's documentation/full CI gates, and
      self-review both repository diffs for stale commands, security claims, and terminology drift.

## Risks
- The canonical visible flow and the compatible low-level MCP session flow have different admission
  defaults. State each default at its own boundary so readers do not accidentally treat a private
  conversation key as an approval request.
- “MCP messaging support” and “continuous visible conversation support” are different capability
  levels. List them separately so adding a connector is never mistaken for adding a native wake
  adapter.
- The website is maintained in `/Users/tamnguyen/conductor/workspaces/parler-web/kelowna`; keep its
  established design and change only the instructional content needed for parity.

## Review
- The README, maintained guides, articles, examples, registry/marketing copy, and website now lead
  with `parler connect` plus `parler conversation`, while keeping the low-level session flow clearly
  labeled as a compatible alternative (and its approval gate explicit when used).
- Support matrices separate continuous visible adapters (Claude Code, Codex, OpenCode), MCP tool
  hosts, bounded managed workers, and arbitrary supervised runners. Admission, permission, viewer
  lifetime, and conversation-scoped file-download boundaries match the implementation.
- Contributor guidance records the actual `Host` dispatch and `AdapterContext` boundary, provider
  module layout, bounded-state invariants, permission contract, parity tests, and update checklist for
  a future adapter.
- `CARGO_INCREMENTAL=0 make ci` passed after cleaning this workspace's generated target directory;
  the first attempt exhausted local disk while writing compiler metadata. Website `npm run build`
  passed all type/lint checks and generated 110 pages. Browser QA passed at mobile, tablet, and desktop
  sizes with no console errors or failed page loads on the final preview.
- Self-review found no remaining stale commands, false provider-parity claims, phantom MCP tools,
  repository-link drift, or whitespace errors.

---

# Scalable visible-host adapter architecture

## Plan
- [x] Replace Codex full-thread polling/resume hydration with bounded `thread/turns/list` pages and
      synchronize only while a turn is active or a status transition requires it.
- [x] Replace OpenCode full-history timer polling with its SSE event stream, bounded message tails,
      and bounded completion-id retention.
- [x] Centralize the provider-independent identity environment, backlog validation/materialization,
      connected lifecycle, and durable result contract so a new adapter cannot silently omit parity.
- [x] Clean up Claude hook state at session end and keep its existing bounded transcript/hook model.
- [x] Add scaling and contract regression tests, document the provider extension checklist, run
      targeted checks plus `make ci`, and self-review the union against `origin/main`.

## Risks
- Codex canonical history is still the fallback when detailed notifications are connection-routed.
  Use the host's paginated recent-turn API with full item detail and retain a window larger than a
  page so an anchor cannot be republished after eviction.
- OpenCode SSE is a latency/source-of-change signal, not durable state. Re-read a bounded canonical
  message tail on terminal session status and leave Parler's cursor uncommitted on stream/API errors.
- Backlog cursor advancement differs by host: Codex commits after a bootstrap turn, OpenCode after
  persisted no-reply context, and Claude after its rewake turn. Centralize preparation but keep each
  adapter's acknowledgement point explicit.
- No wire or hub change is needed. Keep one additive exact-cursor commit primitive in the connector
  and preserve old deployed hub compatibility.

## Review
- The adapter boundary now passes one shared context into native provider state machines. Identity,
  signed catch-up, files, lifecycle, result receipts, and cursor semantics are centralized; native
  attach/injection/completion and permission channels remain provider-owned.
- Room catch-up pages through 1,000-message batches, retains a 24,000-character trusted tail, stops
  explicitly at 10,000 messages, and commits the exact received cursor only after host acceptance.
- Codex reads bounded 64-turn canonical pages only around active/status transitions and retains 256
  terminal ids. OpenCode uses its SSE stream, a 256-message terminal tail, 1,024 ids, and bounded API
  buffers. Claude bounds native rewake context at 9,000 characters and removes ended hook state.
- OpenCode terminal reconciliation collapses multiple assistant records for one native parent into
  one final result. Claude catch-up selection preserves the newest context inside its native limit.
- `docs/visible-host-adapters.md` defines parity, scaling invariants, failure semantics, and the
  extension checklist for another provider. No wire frame or deployed hub behavior changed.
- `CARGO_INCREMENTAL=0 make ci` passes build, all-target Clippy with warnings denied, 171 CLI tests,
  19 connector tests, all workspace/integration suites, docs, smoke, advisories, sources, licenses,
  and bans. Self-review found no unresolved findings.

---

# Visible conversation parity for Claude Code and OpenCode

## Plan
- [x] Add an explicit conversation host selector while preserving Codex as the backward-compatible
      default, and keep identity, hub routing, backlog, files, presence, and loop prevention shared.
- [x] Add a normal visible Claude Code adapter using invocation-scoped MCP plus documented
      `asyncRewake` hooks, with durable per-session turn state and no automatic permission grants.
- [x] Add a normal visible OpenCode adapter using its documented local server, attached TUI,
      asynchronous prompt API, and canonical session/message state.
- [x] Add focused regression coverage for host configuration, hook state/locking, transcript and
      message parsing, durable result acknowledgement, and CLI selection.
- [x] Update README/AGENTS/docs for equal host behavior, run targeted checks plus `make ci`, and
      self-review against `docs/code-review-guidelines.md`.

## Risks
- Claude Code can fire overlapping async hooks. Serialize one waiter per visible session, use a
  separate atomic state update lock, and cancel an idle waiter when a local prompt starts.
- An injected peer turn must retain normal host permission policy. Use Claude Code's system-reminder
  rewake and OpenCode's server/TUI permission channel; never synthesize approval responses.
- OpenCode's HTTP API is local but still user-input-facing through `--resume`; validate session ids
  before constructing paths and accept only successful, bounded JSON responses.
- The deployed hub and old clients require additive compatibility. Keep the wire protocol and room
  storage unchanged.

## Review
- `parler conversation --host codex|claude|opencode` now selects a normal visible host while Codex
  remains the default. All adapters reuse the same signed backlog/file handling, terminal task
  receipt, explicit continuation marker, identity scope, presence, and durable cursor semantics.
- Claude Code runs with invocation-scoped MCP plus `asyncRewake` lifecycle hooks. Hook input,
  transcript reads, prompt state, file locks, and rewake text are bounded; overlapping lifecycle
  events are generation-cancelled; no permission hook or automatic approval path was added.
- OpenCode runs a loopback server and attached TUI, preserves configured Basic Auth, validates
  resume ids, bounds and times out every API response, rechecks canonical session status before
  injection, and leaves permission decisions with the visible TUI.
- Verified the installed Claude Code and OpenCode interfaces against their native local protocols.
  `make ci` passes build, Clippy `-D warnings`, the CLI tests plus the workspace suites, docs, smoke,
  advisories, sources, licenses, and bans. Self-review found no unresolved findings.

---

# Live interactive conversations and truthful presence

## Plan
- [x] Add one canonical `parler conversation [KEY]` flow: no key creates and shares a conversation;
      a portable key joins it. Default keys admit immediately, while an explicit approval flag keeps
      the gated mode for sensitive conversations.
- [x] Add a Codex interactive host adapter using the documented app-server + remote TUI seam, so
      signed peer messages start turns in the visible Codex thread instead of spawning `codex exec`.
      Mirror visible agent replies back to the conversation, preserve explicit addressed handoffs for
      longer autonomous chains, and never auto-approve shell/tool actions.
- [x] Catch late joiners up from the durable backlog, carry resumed Codex user/agent context into a
      newly shared conversation, and materialize shared file blobs into a content-addressed local inbox
      before injecting their paths.
- [x] Keep live identities genuinely live: refresh presence on protocol heartbeats, heartbeat MCP
      connections before the five-minute stale window, and have the interactive adapter publish
      waiting/working lifecycle state.
- [x] Add focused app-server protocol/selection tests, a real in-process hub conversation test,
      presence regression tests, CLI help/docs migration, and full CI/self-review.

## Risks
- Codex app-server WebSocket mode is documented as experimental. Probe capabilities/version at
  startup and fail with an actionable message; do not fall back silently to a headless runner.
- A shared conversation key grants transcript access and supplies model input. Keep keys private,
  require valid message signatures for automatic turns, retain Codex's normal sandbox/approval
  policy, and prevent response ping-pong unless an agent explicitly addresses a continuation.
- The deployed hub and older clients require additive compatibility. Keep `room` as the internal wire
  primitive and add `conversation` as CLI/UX sugar without renaming existing frames or fields.

## Review
- Added the canonical `parler conversation [KEY@HUB]` UX and hid the low-level `session` command from
  normal help. A fresh invocation opens a standard visible Codex TUI; another terminal can join at
  any point, catch up, and remain attached to the same durable conversation.
- Codex app-server adopts the thread created by that visible TUI. Signed peer messages become turns
  in the same window, complete local human/agent exchanges are shared back, peer results carry a
  terminal task receipt to prevent ping-pong, and an explicit addressed marker is required to extend
  an autonomous chain. No `codex exec` fallback and no self-approved escalation were added.
- Portable keys include the exact hub; each terminal gets a stable distinct identity; viewer tokens
  stay bound to the original room; non-owners are explicitly forbidden from creating `_watch`
  shadows. MCP and adapter heartbeats preserve waiting/working lifecycle instead of decaying a live
  agent to offline.
- Live two-TUI validation: the exact viewer reported 2 members / 2 online; a peer answer woke the
  other visible Codex without a keypress; simultaneous human turns queued safely; both automatic
  results landed once; both agents returned to waiting; the message count stayed stable with no loop.
- Verification: desktop `npm test`, typecheck, and production build pass; targeted conversation tests
  and Clippy pass; full `make ci` passes build, Clippy `-D warnings`, workspace tests, docs, smoke, and
  audit. Self-review against `docs/code-review-guidelines.md`: no unresolved findings.

---

# Autonomous agent runtime, attention, role queues, and local supervision

## Plan
- [x] Establish the green Rust baseline and map the current Stop-hook/push/cursor path; preserve its
      durable-pull invariant while making autonomous delivery host-independent.
- [x] Add an additive, persisted attention policy (open/dnd/focus plus per-room quiet/muted) and
      expose it consistently through CLI, MCP, presence, and connector decision helpers.
- [x] Make service work explicitly role-addressed and atomically claimed by an available worker, with
      presence driving eligibility; retain the existing service-room behavior for old clients.
- [x] Add an optional local supervisor/worker that holds a live mesh connection, wakes on delivery,
      invokes an explicitly configured runner, reports lifecycle/task state, and never sits on the
      message hot path.
- [x] Add behavior and end-to-end tests, update all user-facing docs/tool references, run the full
      Rust gate, and self-review the diff.

## Risks
- The deployed hub and old clients require additive wire/schema changes only; push remains a latency
  hint and may never advance a cursor or become a delivery authority.
- A generic MCP server cannot force every host to start a model turn. The supervisor must therefore
  provide genuine autonomy for explicitly launched runners while host-native hooks remain adapters.
- Muting/holding traffic must be an intentional local attention choice, never a lossy hub-side cursor
  filter; service claim races must yield exactly one executor.

---

# Per-room resource quotas + isolation docs

## Problem
On the shared public hub, room **data** isolation is solid (per-op `is_member`, `blob_rooms`,
fact `room`/`author` scoping, owner-only watch tokens). What's missing is **resource** isolation:
all rooms share one process, one SQLite writer lock, one blob disk budget, one connection ceiling.
Rate limits are per-agent / per-IP, never per-room. So one busy/abusive room can degrade latency or
exhaust disk for every other room (noisy-neighbor / write-DoS). No container sandbox is needed —
agents run on users' machines, not on the hub — the fix is per-room quotas.

## Plan
- [x] Extend `RateLimits` with `max_room_sends_per_min` + `max_room_blobs_per_hour` (defaults on).
- [x] Add `DEFAULT_MAX_ROOM_SENDS_PER_MIN` / `DEFAULT_MAX_ROOM_BLOBS_PER_HOUR` consts.
- [x] Add `room_rate` in-memory map to `HubState`; rename `AgentRate` → `RateWindows` (reused).
- [x] Extract `charge_window` helper; refactor `rate_allows`; add `room_rate_allows`.
- [x] Enforce per-room send limit in `Send` (after `resolve_target`).
- [x] Enforce per-room blob limit in `PutBlob` (after `resolve_target`).
- [x] Prune `room_rate` in `prune_rate_windows`.
- [x] CLI flags + env in `parler-hub` binary (`--max-room-sends-per-min`, `--max-room-blobs-per-hour`).
- [x] Unit tests (enforce, per-room independence, prune, 0-disables).
- [x] Docs: README FAQ (new isolation entry) + Security "Abuse limits" bullet;
      docs/storage-and-memory.md; AGENTS.md if it lists limits.
- [x] `make ci` green.

## Review
Shipped per-room resource quotas — the missing *resource* isolation on the shared hub (data isolation
was already strong). No sandbox: agent code runs client-side, so there's nothing on the hub to
sandbox; the real risk was noisy-neighbor / write-DoS.

**Code (`crates/parler-hub/src/server.rs`)**
- `RateLimits` gained `max_room_sends_per_min` (default 1200) + `max_room_blobs_per_hour` (default 600).
- New `room_rate` map on `HubState`; `AgentRate` → `RateWindows` (now reused for agent *and* room keys).
- Extracted `charge_window` helper (shared by both limiters); added `room_rate_allows`.
- Enforced in `Send` and `PutBlob` right after `resolve_target` (per-room, on top of per-agent).
- `prune_rate_windows` now prunes idle rooms too.
- 4 new unit tests: enforce+roll, per-room independence, 0-disables, prune-idle, defaults-on. All green.

**Config (`crates/parler-hub/src/main.rs`)** — `--max-room-sends-per-min` / `--max-room-blobs-per-hour`
(+ `PARLER_HUB_*` env), `0` disables. Mirrors the existing `max-*` flags.

**Docs** — README FAQ (new "are rooms isolated / is there a sandbox?" entry) + Security "Abuse limits"
bullet; `docs/storage-and-memory.md` (blob-limit line + new fairness/#3 scalability point). AGENTS.md /
agent-mesh.md don't enumerate abuse limits, so no drift there.

**Deliberately not done (noted for follow-up):** per-room *member* / *connection* caps. Those need a
store query + enforcement at several `add_member` call sites; the write-path rate quotas already bound
the two concrete DoS vectors (writer contention, disk fill) with minimal surface. Easy to add next.

---

# Marketing package and dark editorial artwork

## Plan
- [x] Audit the interrupted README and marketing-kit draft against current product behavior and voice.
- [x] Replace generic neon-tech artwork with a coherent modern, dark editorial campaign set.
- [x] Update the README and artwork guide to use the final assets, including accurate sizes and alt text.
- [x] Run documentation checks, inspect every final image, and self-review against the repo guidelines.

## Risks
- Marketing copy must not imply end-to-end encryption or bypass session join approval.
- Generated art must stay legible in dark GitHub themes and avoid fake product UI or unreadable text.

## Review
- Reframed the README around "Share the session. Skip the transcript." and linked the reusable kit.
- Added positioning, channel copy, campaign plans, and truthful objection-handling under
  `docs/marketing/`.
- Replaced the interrupted neon-terminal set with eight dark editorial assets: handoff hero and
  square, team session, join approval, local/private mode, shared memory, signed identity, and
  code/file handoff.
- Recorded dimensions, placements, alt text, crop rules, palette tokens, and the reusable prompt
  direction in the artwork guide.
- Verified every final image visually; `git diff --check`, `make selftest`, and `make ci` pass.

---

# macOS automation and guided handoff UX

## Plan
- [x] Keep installed agents connected in the background using the existing `parler connect` source of truth; never rewrite hosts already pointed at the selected hub.
- [x] Make the selected local/shared hub persistent so manual choices and background automation agree.
- [x] Put the flagship session handoff on the home screen and add the native macOS Share menu for intentional key sharing.
- [x] Add a dependency-free policy test, update desktop docs/settings copy, and run the desktop typecheck/build plus self-review.

## Risks
- Agent config writes must remain opt-in (`autoConnectAgents`) and must not race first-run onboarding.
- Session keys are capabilities; sharing stays behind an explicit user click and is never sent automatically.

## Review
- Background scans are single-flight, wait until onboarding is complete, and only run against a
  stopped hub when the remembered target is the shared hub.
- The reconciliation policy has coverage for missing, misdirected, ready, and uninstalled hosts.
- Session sharing validates bounded IPC inputs and only opens the macOS Share menu after the user
  clicks Share; non-macOS builds fall back to copying the invitation.
- Verified with `npm test`, `npm run typecheck`, `npm run build`, renderer smoke boot, and `make ci`.
  Self-review against `docs/code-review-guidelines.md`: no remaining findings.

---

# Fix room agent identity collapse

## Plan
- [x] Apply the existing workspace identity scope to CLI/hook agent commands, not only `parler mcp`,
      so terminal-driven joins do not reuse the flat `~/.parler/config.json` identity.
- [x] Parse full `parler://…/join/…` links as portable code + hub descriptors so CLI joins dial the
      room's hub and MCP joins report an honest hub mismatch.
- [x] Add regression coverage for command scoping, full-link parsing, and two identities appearing as
      two roster members; update identity/session docs.
- [x] Run targeted tests, `make ci`, and self-review the final diff.

## Risks
- Setup/admin commands (`connect`, `init`, `hub`, `doctor`) must keep using the unscoped home.
- Workspace identities must remain restart-stable; no seed may move, be logged, or cross the wire.

## Review
- Live diagnosis: `room.8tuhxc` contained one member named `probe`; invite `CQXL5SJN` had `uses=0`,
  proving the two terminal agents never joined and both observed the same flat identity.
- Agent-hosted CLI commands now use the same scoping seam as MCP. A stable host session discriminator
  splits same-directory terminals; ordinary human CLI/setup commands remain backward compatible.
- Fresh scopes inherit the legacy config's hub but never its seed or stale display name. Full join
  links carry their hub into both CLI routing and MCP mismatch errors.
- Binary-level isolated-hub test: a full join link produced 3 roster members with 3 unique ids.
- `CI_SKIP_WEB=1 make ci` passes. Self-review: no findings remain.

---

# Add room deletion

## Plan
- [x] Add additive protocol/connector support for owner-only room deletion.
- [x] Implement hub/store cleanup for room membership, messages, invites, join requests, watch tokens,
      room-scoped facts, and blob bindings.
- [x] Expose the capability through CLI and MCP.
- [x] Add store/e2e/MCP regression tests and update docs.
- [x] Run targeted checks, `CI_SKIP_WEB=1 make ci`, and self-review.

## Risks
- Deletion is destructive; non-owners must not be able to erase shared history.
- Cleanup must not leak deleted room data through watch tokens, blobs, memory, rooms, roster, or pull.

## Review
- Added additive `delete_room` / `room_deleted` frames, `MeshAgent::delete_room`, CLI
  `parler delete-room --room R`, and MCP `parler_delete_room` (defaults to active session).
- Store deletion is owner-only and atomically purges room-scoped messages, members, invites, pending
  joins, watch tokens, facts, and blob bindings; blob bytes remain for existing GC.
- Regression coverage: store cleanup/authorization, real WebSocket member access loss, MCP active
  session clearing, protocol serialization, MCP schema/doc guards.
- `scripts/verify.sh --rust-only` and `CI_SKIP_WEB=1 make ci` pass.

---

# Autonomous room worker

## Plan
- [x] Add `parler work` as a long-lived worker for an active session, explicit room, or service
      queue; use the existing self-healing long-poll path so a peer message creates a real turn.
- [x] Execute signed, allowed work through built-in Codex/Claude headless runners, with a bounded
      timeout and rate; targeted handoffs are actionable by default, with an explicit two-agent
      `--all-messages` mode for ordinary room requests.
- [x] Post signed working/done/failed lifecycle messages and the runner result, while treating
      lifecycle/result messages as non-actionable so two workers cannot recursively trigger.
- [x] Add a runner seam plus unit/e2e coverage for targeting, trust/allow-list gates, wake-to-run,
      result delivery, cursor durability, and loop prevention.
- [x] Update README/communication/mesh/task docs and add a Conductor-local run command for starting
      the worker in this workspace.
- [x] Run targeted checks, `make ci`, and self-review against `docs/code-review-guidelines.md`.

## Risks
- Conductor/Codex does not expose a documented hook that injects a message into an already-stopped
  interactive chat; autonomy therefore runs a managed headless turn in the same workspace.
- Remote room text becomes model input and may cause workspace edits. Require valid message
  signatures, keep service queues allow-listed by default, sandbox the runner, and bound time/rate.
- A worker must never execute its own status/result messages or a pair of workers can ping-pong
  forever; terminal task parts are a hard non-actionable boundary.

## Review
- Root cause confirmed: durable delivery and room context were working, but a stopped Codex/
  Conductor chat had no scheduler to create its next model turn. `recv --watch` could only print.
- Added `parler work` for rooms, active sessions, and service queues. It validates signatures and
  sender allow-lists, runs one sandboxed Codex/Claude process at a time, bounds time/rate, posts
  working + terminal receipts, and commits the cursor only after the result lands.
- Fresh room work gets one automatic return turn. A runner can deliberately extend or route a chain
  with one validated, addressed `PARLER_HANDOFF` final-line envelope; ordinary lifecycle/results
  remain inert, preventing accidental ping-pong.
- Conductor now treats its isolated workspace as the identity boundary, so the interactive MCP agent
  and Run-menu worker share the active room/cursor. Personal Run entries for Codex and Claude were
  installed in the repository's local Conductor settings.
- Coverage includes 11 worker tests with real in-process hub/WebSocket room, service-DM/fallback,
  automatic-return, and explicit three-agent continuation flows. The full CLI suite passed (124),
  final Clippy passed with warnings denied, documentation reference checks passed, and the exact
  unmodified `make ci` aggregate is green on the finished diff.
- Self-review found and removed an unsigned-mention authorization path: hub-normalized `mentions`
  cannot launch a turn; only a signed addressed handoff or explicit `--all-messages` can do so. A
  negative test now locks that trust boundary.
- Self-review: no correctness, security, protocol-compatibility, or documentation-drift findings
  remain. External side effects are honestly documented as at-least-once, and users are warned not to
  run two activation consumers on the same identity/room cursor.

---

# Fresh installs use the shared hub

## Plan
- [x] Confirm the slow MCP startup was stale local-hub wiring rather than a slow shared hub.
- [x] Make the desktop app's first-run settings, onboarding, and app identity follow the shared
      public hub without starting a local service.
- [x] Add a first-install default-policy test and document recovery from stale local wiring.
- [x] Run the desktop build/type checks and the repository CI gate; self-review the finished diff.

## Risks
- Existing saved local configurations must remain local; only a settings file absent on a fresh
  installation may receive the new shared-hub default.
- The shared hub sees plaintext, so the onboarding and troubleshooting guidance must state when a
  user should explicitly choose local mode.

## Review
- A missing desktop settings file now selects the shared hub and leaves `autoStartHub` off. Existing
  settings still merge over those defaults, so current local installations are not moved.
- Onboarding starts a local process only after the user explicitly selected local mode; automatic
  agent wiring and the app identity follow the selected target.
- Added the public/local recovery guide and linked it from the README, MCP setup docs, and the
  project map. The website implementation is maintained in its separate repository, so this repo
  now contains the canonical troubleshooting source to publish there.
- Verified with `npm test`, `npm run typecheck`, `npm run build`, `git diff --check`, targeted MCP
  documentation-reference coverage, and `make ci`. Self-review found no remaining findings.

---

# Trust Parler commands during connect

## Plan
- [x] Extend each supported provider config with its native Parler-only approval rule: Codex MCP +
      command rules, Claude Code permissions, Gemini trust, OpenCode permissions, and Cline tools.
- [x] Keep config merges idempotent, preserve unrelated user settings, and remove Parler-owned
      permission entries with `parler connect --remove`.
- [x] Add fresh-file, merge, idempotency, and cleanup regression tests for every written dialect.
- [x] Update setup/troubleshooting docs with the scoped trust behavior and the UI-only hosts.
- [x] Run targeted tests, `make ci`, and self-review the diff against the repository checklist.

## Risks
- Auto-approval includes Parler's mutating tools (send, fetch/apply, join decisions, and room
  deletion), so rules must match only the `parler` executable/MCP namespace and never weaken a
  provider's global approval policy.
- Provider config formats differ and user-owned settings must survive both connect and remove.

## Review
- `parler connect` now installs provider-native, Parler-scoped trust for Codex, Claude Code,
  Gemini CLI, OpenCode, and Cline. Cursor, Windsurf, VS Code, and Claude Desktop keep their one-time
  trust choice in the provider UI; no global approval policy is disabled.
- Connect/remove preserve unrelated config, Cline's allowlist derives from the real MCP tool list,
  and Codex's CLI rule file is narrow, owned, and never overwrites an unmanaged file.
- Regression tests cover fresh/merged config, idempotency, exact tool names, cleanup, and refusal to
  clobber user-owned policy. Full workspace CI and the documentation tool-reference guard pass.

---

# Frictionless live rooms

## Plan
- [x] Keep every active conversation on a durable push/long-poll receive loop and inject or run the
      next agent turn immediately through the host's supported activation seam.
- [x] Make possession of a valid room join key the default admission path across canonical, MCP, and
      low-level session flows, while retaining explicit owner approval for sensitive rooms.
- [x] Teach MCP-hosted agents to keep a receive long-poll outstanding while a session is active, so
      they do not wait for a human to ask them to fetch.
- [x] Replace stale approval-by-default prompts/options in CLI, MCP, desktop, scripts, and docs while
      retaining the explicit gate, protocol compatibility, and legacy `--no-approval` parsing.
- [x] Add focused runtime, admission, MCP, and end-to-end regressions; run full CI and self-review.

## Risks
- An MCP server cannot itself schedule a stopped model turn. Use provider-native visible adapters or
  the managed worker for actual wake/activation, and make the compatible MCP path keep listening
  inside its current turn instead of pretending notifications can wake every host.
- A join key becomes a bearer capability for room membership. Expiry, max-use limits, private-room
  membership authorization, signed identity, and the separate read-only viewer token must remain
  enforced.
- Continuous consumers share a durable cursor. Never start two activation loops for the same
  identity and room, or one can acknowledge work intended for the other.

## Review
- Added `ConnectorRuntime::listen_until`: push is a low-latency doorbell, durable Pull remains the
  source of truth, signature/attention policy runs before injection, and failed injection never
  advances the cursor. Claude's Stop hook now uses the shared bounded listener.
- Visible Codex, Claude Code, and OpenCode conversations continue through their native adapters.
  Compatible MCP hosts receive concise instructions to keep one 60-second receive outstanding and
  repeat after acting; this is honestly documented as active-turn best effort, not a fake wake API.
- Durable fallback remains one explicit `parler work` or `parler supervise` process. Safe examples
  execute signed addressed handoffs; ordinary-text execution is limited to a trusted two-agent room
  with `--all-messages --allow-from <trusted-id>`.
- MCP and low-level session creation now admit valid key holders immediately by default. Explicit
  `approval: true` / `--approval` still uses the unchanged owner-only gate, and old
  `--no-approval` scripts remain compatible. The desktop no longer offers a gate on new rooms but
  preserves controls for previously gated records.
- Full workspace CI passes (build, Clippy with warnings denied, 181 CLI tests, 45 mesh tests, docs,
  and cargo-deny). Desktop tests, TypeScript checks, and production build also pass.

---

# Continuous room listener contract

## Plan
- [x] Add a reusable `ConnectorRuntime` listener that continuously re-pulls durable room state,
      applies attention/signature policy, and invokes a supplied host-native injector immediately.
- [x] Preserve push as a latency hint, recover missed pushes through Pull, and keep failed injections
      unacknowledged for retry.
- [x] Use the bounded listener in Claude Code's Stop-hook injection path and add focused runtime tests.
- [x] Update the host-contract docs, run targeted tests, and self-review the diff.

## Risks
- The listener must not imply that arbitrary MCP hosts can wake a stopped model; they still need a
  native injection seam or an explicit `conversation`, `work`, or `supervise` process.
- A timeout or failed injector must never consume durable work, and attention-held traffic must stay
  behind the cursor.

## Review
- `ConnectorRuntime::listen_until` subscribes before its first Pull, waits on push when available,
  and periodically re-pulls so push loss cannot lose work. It returns after one accepted injection,
  preserving native host turn serialization.
- Claude Code's bounded Stop-hook listener now uses the shared contract instead of duplicating the
  polling loop. Hosts without an injection seam remain on the explicit worker/supervisor boundary.
- Real-hub tests cover immediate wake, failed-injector retry without cursor advance, and focus-held
  replay after attention opens. Full connector tests and targeted all-target Clippy pass.
- Self-review against `docs/code-review-guidelines.md`: no remaining findings in this slice.

---

# Frictionless active join

## Plan
- [x] Trace `parler join` / `session join` into the available visible-host and worker activation
      paths; retain an explicit, safe boundary for non-interactive and untrusted rooms.
- [x] Make the normal join output and behavior lead new users into an active listener without
      silently granting arbitrary remote messages authority to start workspace-writing turns.
- [x] Add focused CLI/unit coverage for the selected join activation behavior and update the
      user-facing docs/help text.
- [x] Run the relevant tests, full CI, and self-review the final diff.

## Risks
- A durable room membership or passive listener is not itself a model scheduler. The change must
  use a real native host injection seam or an explicit local runner, not imply that an idle MCP host
  can be awakened by a WebSocket delivery alone.
- Room messages are untrusted input. Ordinary-text execution must remain opt-in and sender-scoped;
  the safe default is a signed addressed handoff or a visible host's existing approval boundary.
- Only one activation consumer can own an identity/room cursor, so a join must not silently launch a
  second consumer beside an active conversation, worker, or supervisor.

## Review
- A channel/DM `parler join` or `parler session join` from a detected Codex/Claude agent now catches
  up, then enters the existing bounded `parler work` loop. Ordinary shells remain passive unless they
  opt in with `--active` / `--runner`; `--passive` preserves display-only joining.
- The automatic worker accepts only valid signed handoffs, at the existing 20-turn/hour and 15-minute
  bounds. It explicitly leaves arbitrary room text non-executable; service rooms still require their
  existing dispatcher sender policy.
- Focused decision/parser tests cover host detection, ambiguity, opt-out, explicit runners, worker
  safety defaults, room-kind boundaries, and flag conflicts. Help output and all maintained CLI/runtime
  docs describe the new path and the visible-conversation boundary.
- `cargo test -p parler-cli --lib`, CLI help checks, `git diff --check`, and `CI_SKIP_WEB=1 make ci`
  pass. Self-review against `docs/code-review-guidelines.md` found no remaining findings.
