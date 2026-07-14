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
