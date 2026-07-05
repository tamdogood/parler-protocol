# Task: Desktop app → 10/10 UX + CLI parity on the 3 key features

Goal: the macOS (Electron/React/Tailwind) app in `desktop/` reaches "10/10 UX with full CLI
parity," staying clean/minimalist, focused on **(1) mid-chat connection, (2) monitoring agents &
sessions, (3) set up a private hub.**

## Findings (verified against the code)

The app is already well-built (obsidian theme, one-click connect, hub supervisor, watch viewer). The
gaps that matter map 1:1 onto the three named features:

1. **Mid-chat connection is broken in-app.** Sessions open with **approval required by default**, but
   there is **no approve/deny UI** — `session requests/approve/deny` (CLI) has zero renderer surface
   (grep confirms none). You open a session, an agent asks to join, and you must drop to a terminal to
   admit it. The flagship flow can't be completed in the app.
2. **Opened sessions aren't remembered.** `OpenedSession` is ephemeral screen state — navigate away
   and the key/watch/room are gone. No list, no monitoring, no re-copy, no close.
3. **Hub activity is invisible.** `/api/hub` already exposes `liveConnections / messagesTotal /
   estimatedTokensTotal / pushesTotal`, but `HubSummary` omits `stats` and nothing renders it. No
   live "monitoring" surface.
4. **"Private hub" is buried** as a Settings sub-screen; the CLI's `--team` rung (LAN-reachable
   private hub + join secret) isn't a first-class choice.

## Plan (prioritized; each phase independently shippable)

### Phase 1 — Mid-chat connection: complete the session lifecycle  ← highest value
- [ ] CLI: add `--json` to `session requests` (structured `{room, requests:[{agent,name,role}]}`) so
      the app reads join requests robustly instead of scraping text. (approve/deny already exit 0/parse.)
- [ ] main: `parler-cli.ts` → `sessionRequests(room)`, `approveJoin(room, agent)`, `denyJoin(room, agent)`.
- [ ] IPC + preload + shared types: expose `session.requests/approve/deny`.
- [ ] Persist opened sessions in `settings.ts` (new `sessions: OpenedSessionRecord[]`): room, key,
      watch, topic, approval mode, createdAt. Add `session.list()/forget(room)`.
- [ ] Renderer: rework `sessions.tsx` into **Open → Session list**. Each row: copy key, copy watch,
      Watch here, **pending-join badge → Approve/Deny inline**, Close. Poll requests while running.

### Phase 2 — Monitoring: make it live
- [ ] `HubSummary.stats` type + `fetchHub` usage; small `useHubSummary(base)` poll hook.
- [ ] A restrained **live activity strip** (connections · messages · ≈tokens · pushes) on the Hub
      screen; a compact roll-up header on Agents (online/total). Keep it minimal — no chart junk.
- [ ] Directory already good; add live counts + gentle "active now" pulse only.

### Phase 3 — Private hub as a first-class feature
- [ ] Promote **Hub** to a sidebar item (Agents · Connect · Sessions · Hub · Settings).
- [ ] Present the ladder the CLI uses — **Private / Team / Public** — with the join secret prominent
      for Team, one-line teammate connect string. Reuse existing start/stop/logs/storage.

### Phase 4 — Cross-cutting 10/10 polish
- [ ] Lightweight global **toast** system (copy / connect / approve feedback) — no dependency.
- [ ] Keyboard: ⌘1–5 nav, Esc closes drawers, ⌘C on token fields.
- [ ] Consistent empty/loading/error states; motion via existing `slide-up-fade`.

## Constraints / guardrails
- Don't weaken the security model (approval gate, watch = read-only, seed never leaves device).
- `parler-protocol` changes ripple to hub/connector/cli/web — the only CLI change here is an additive
  `--json` on `session requests` (no wire/protocol change). Run `CI_SKIP_WEB=1 make ci` for it.
- Never run `cargo fmt`. Hand-match style.
- **Verification caveat:** this is a GUI Electron app; `node_modules`/bundled binaries aren't
  installed here. I can `npm install` + `npm run typecheck`/`build` the renderer, and `make ci` the
  Rust change, but true visual "10/10" needs a real run on the user's Mac.

## Review — shipped

**Phase 1 — session lifecycle (the flagship gap): DONE.** `session requests` gained an additive
`--json` (`{room, requests:[{agent,name,role,requestedAt}]}`; verified end-to-end against a scratch
hub). New `session-store.ts` persists opened sessions to `userData/sessions.json` (kept out of
preferences). `parler-cli.ts` drives `sessionRequests`/`resolveJoin`; IPC/preload/types expose
`session.list/forget/requests/approve/deny`; the open handler now saves a record. `sessions.tsx` is
reworked into Open → **Your sessions** list: each card shows both codes (copy), Watch, Close, and —
for a live, approval-gated session — **polls pending joiners and approves/denies them inline**. You
can now complete the whole handoff in-app.

**Phase 2 — live monitoring: core DONE.** `HubSummary.stats` + `useHubSummary` poll `/api/hub`; the
Hub screen shows a restrained **Live activity** strip (live connections · messages · ≈tokens ·
pushes). *Deliberately skipped* a separate Agents roll-up — the directory already polls every 5s with
status facet counts + per-card "active now", so more would be redundant, not cleaner.

**Phase 3 — private hub, first-class: DONE + made honest.** Hub is now a top-level sidebar item (back
button removed). The supervisor bound loopback-only, so a "Team" toggle would have been fake — added a
`hubReachable` setting that binds `0.0.0.0` (still gated by the existing join secret; same posture as
the CLI `--team`) and surfaced the **Private / Team / Public** ladder with a teammate connect
one-liner built from the machine's LAN IP.

**Phase 4 — polish: DONE (scoped).** ⌘1–5 screen nav; Esc already closed drawers. Added a
**pending-join badge on the Sessions nav item** (App-level `usePendingJoinCount`) so an agent asking
to join never hides on another screen — higher-value than the planned global toast system, which I
*deliberately skipped* to stay minimalist (copy/approve already give inline feedback).

**Verification.** `CI_SKIP_WEB=1 make ci` green (build · clippy -D warnings · test --locked · audit).
Desktop `npm run typecheck` + `npm run build` green (main + preload + renderer). `session requests
--json` verified live. **Not done here:** a real visual run of the Electron app (needs the user's Mac
+ `npm run build:binaries`); no screenshots taken.

**Follow-ups (not blocking):** the Sessions screen and the sidebar badge both poll requests when
you're on that screen (bounded double-poll, few sessions); could unify later. `agents` roll-up + a
global toast remain available if wanted.

---

## 2026-07-04 — Cross-model engineering contract + review agent (branch los-angeles)

**What:** made the guidelines tool-agnostic and enforceable for any agent (Claude, Codex, OpenCode).
Canonical docs: `docs/engineering-guidelines.md` (authoring contract: workflow, hard gates,
invariants promoted from lessons.md, Rust quality bar, token discipline, definition of done) and
`docs/code-review-guidelines.md` (review contract: verify-before-report, severity ladder, report
format, checklists, what-not-to-flag). Claude wrappers: `.claude/skills/code-standards/` (change
workflow), `.claude/skills/parler-review/` (review runbook), `.claude/agents/code-reviewer.md`
(spawnable read-only review agent). Pointers wired into `AGENTS.md` (docs index + working
agreements — the file Codex/OpenCode auto-read), `CLAUDE.md`, `CONTRIBUTING.md`.

**Design:** single source of truth in `docs/`, skills/agents are thin runbooks over it; layering
note in the guidelines makes `tasks/lessons.md` upstream (lessons land there, durable rules get
promoted). **Verified:** every path/symbol cited by the new docs exists (`write_private_file`,
`TOTAL_CACHE_KIB`, `verify.sh --rust-only`, `CI_SKIP_WEB`); both skills registered. Docs-only
change — no code gates to run.

---

## 2026-07-05 — Session Wrapped: shareable viewer URL + viral scorecard (branch session-share-scorecard)

**What:** a "Spotify Wrapped for a session" — a modern, screenshot-ready scorecard the user can post
to Instagram / Facebook / X, plus a first-class share affordance for the live viewer URL. Web-only;
no Rust/protocol change (the `/api/session` payload already carries stats + messages).

**Shipped (all in `web/`):**
- `lib/wrapped.ts` — pure `buildWrapped(view, messages)` → `{ totalTokens, totalMessages, agentCount,
  durationMs, toolCalls, tokensPerMessage, topAgents[], mvp, vibe, … }`; headline numbers from the
  whole-room `stats` aggregate, tool-call flavor counted from loaded messages, a derived "session
  vibe" badge. Plus `fmtCompact/fmtDuration/fmtPercent` helpers.
- `lib/wrapped-canvas.ts` — dependency-free `drawWrapped(canvas, wrapped)` renders a 1080×1920 (IG-
  story) card straight to a `<canvas>` (gradient blooms, orbit mark, hero token number, 2×2 stat
  grid, "who did the talking" leaderboard with bars, footer). Canvas = the download source, so WYSIWYG
  and crisp; no DOM-rasterization/font-embed pitfalls, and **no new npm dependency**.
- `components/session-wrapped.tsx` — `WrappedShare`: the canvas card + share rail (Download PNG,
  native Share incl. sharing the **image file** to the OS sheet on mobile, Copy link, Post to X,
  Facebook). Reused by the modal and the standalone page.
- `components/session-viewer.tsx` — threaded the watch token into `ConnectedView`; added a
  **"Wrapped"** button (opens a modal with `WrappedShare`) and a **"Share"** button (copies the live
  `/hub#sessions&k=<token>` viewer link).
- `app/wrapped/{page,layout,opengraph-image}.tsx` — a standalone, shareable `/wrapped#k=<token>` page
  (reads the token from the hash — never sent to the server, mirroring the viewer's security model —
  fetches once, renders the card + share rail; loading / notoken / unauthorized / error states),
  noindex, with a branded static OG link-preview card.

**Security:** unchanged model. The card is a static image with **no watch code baked in**; the share
links carry the existing read-only, room-scoped, expiring watch token exactly as the viewer already
does. `/wrapped` is noindex; the token stays in the URL hash.

**Verification — BLOCKED by the environment, not the code.** This sandbox's temp/output area is
capped (writes to `/private/tmp/claude-…` and npm's TMPDIR staging hit `ENOSPC` though `/` has ~60 GiB
free), and `web/node_modules/typescript` is missing its bundled `lib.*.d.ts` (a pre-existing broken
install), so `tsc`/`next build`/`next lint` can't run here. Did a careful manual review instead (types,
imports, no-unescaped-entities, hook deps) — including confirming every cross-module import resolves
and every `SessionView`/`SessionStats`/`SessionAgentStat`/`SessionMessage` field read matches
`lib/types.ts`. **To verify:** `cd web && npm ci && npm run build` (and `npm run lint`). No dependency
was added, so it should build clean.

**Follow-up (same branch):** added a **Wrapped** entry point on the paste-a-code card in
`SessionViewer` — paste a watch code → one-shot `fetchSession` → card, *without* entering the live
viewer — and **lifted the Wrapped modal to the `SessionViewer` root** so the connected-header button
and the paste-card button share one modal (`wrapped` state `= { view, token, messages? }`). There is
no persisted session list to hang a row off of: watch tokens are memory-only by design (never
localStorage), which is the correct security posture, so the pre-viewer entry lives on the code entry.
