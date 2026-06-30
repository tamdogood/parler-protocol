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

# Task: Verifiable mesh — sign/chain the conversation so the hub can relay but can't lie — 2026-06-29

**User ask (`/loop`):** audit the main features (chat sessions, etc.); pull in research/other-field
concepts (e.g. **blockchain**) to make the protocol *better, more secure*, and the agent-to-agent
connection *more resilient, reliable, efficient, smooth*; then come up with scenarios and run **e2e**
of each case to prove it works. Self-paced loop — one verified increment per iteration.

## Audit — what exists, and the gap that matters most
Mature, security-conscious system: nkey/Ed25519 identity (agent id == pubkey), **self-signed
discovery cards** (`canonical_card_bytes`, re-verifiable), challenge-response auth (+ optional
constant-time join secret), rooms (1:1 / 1:many / many:1), **durable per-(room,agent) cursor** (at-
least-once + crash-safe resume), best-effort **push** layer, approval-gated **sessions**, owner-only
read-only **watch** tokens, content-addressed **git-bundle handoff**, FTS5 memory (+ vector
scaffolding). AGENTS.md's headline claim: *"the hub is a relay, not a root of trust — even a
compromised hub can't forge a listing or impersonate anyone."*

**The gap:** that claim holds for **cards** (signed) but **NOT for messages.** A `Send` is stored with
a hub-set `from`, and nothing is signed. So a **compromised/malicious hub can forge a message from any
agent, alter authored content, reorder, drop, or fabricate an entire backlog** — and a joining agent
that gets "caught up" (the flagship session-handoff flow) has **no way to detect it**. An agent then
*acts* on that context (decisions, file paths, "deploy to prod"). This is the highest-value place to
apply distributed-ledger / Certificate-Transparency / reliable-messaging ideas.

## Roadmap (each item additive, backward-compatible, behind `scripts/verify.sh --rust-only`)
Concepts borrowed and where they map:
1. **[P0] Authenticated messages (signatures).** Author signs each message; carried as a
   `com.parler.sig` **extension part** (like `com.parler.bundle`) ⇒ **zero hub/protocol/schema change,
   works against the live deployed hub today**. Verified on receive; surfaced as ✓/⚠/✗ in CLI & MCP.
   *Property: a malicious hub cannot forge or alter authored content — extends the signed-card
   guarantee to the conversation itself.* [ledger: every transaction signed by its originator]
2. **[P1] Tamper-evident, fork-detectable room log (hash chain).** Sig payload also commits to
   `prev` = hash of the author's last-seen message in that room; `parler verify --room R` walks the
   chain + reports a head; two members comparing heads detect hub **equivocation / split-view**.
   [blockchain + Git DAG + Certificate-Transparency: hash-linked append-only log, gossip the head]
3. **[P1] Exactly-once sends (idempotency).** The signed `uid` doubles as an idempotency key; hub
   dedups within a window ⇒ a retried send after a dropped ack never duplicates. [reliable messaging:
   at-least-once + idempotent consumer = effectively-once; Stripe idempotency-key]
4. **[P2] Self-healing connection (auto-reconnect + resume).** Reconnecting transport re-handshakes,
   resumes from the durable cursor, re-arms `subscribe`, with backoff. [durable cursor + reconnect =
   session continuity]
5. **[P2] Hardened auth challenge (domain-separated, hub-bound, expiring nonce).** Make the nonce an
   opaque structured token so the signature is domain-separated + replay-bounded — zero client change.
   [crypto: SIWE / EIP-712 domain separation]

## This iteration — #1 Authenticated messages
- **protocol** (`hub.rs`, pure): `MESSAGE_SIG_KIND="com.parler.sig"`; `canonical_message_bytes(from,
  target, parts_without_sig, reply_to, ts, uid)` (reuses `canonicalize`); `MessageSig{sig,ts,uid,
  target}` with `to_part()`/`from_parts()`. **Sign over parts + target + author id + reply_to + client
  ts/uid. Exclude `mentions`** (hub normalizes them → would break the sig). + round-trip tests.
- **connector**: `MeshAgent::send` auto-signs when an identity is present (covers send/push/session
  seeding). `verify_message(from_id, parts, reply_to) -> SigStatus{Unsigned,Valid,Invalid}` (uses
  `parler_auth::verify`). Add `uuid` dep for `uid`.
- **cli/mcp**: filter the sig part from display; prefix each message with ✓ (valid) / ⚠ (unsigned) /
  ✗ (BAD). hub `/api/session` viewer: drop the sig part server-side (Rust, in scope).
- **e2e** (`mesh_e2e.rs`, real WS hub): (a) signed channel msg verifies; (b) signed DM verifies;
  (c) **tampered content** ⇒ Invalid; (d) **forged `from`** ⇒ Invalid; (e) legacy unsigned ⇒ Unsigned;
  (f) a pushed `Delivery` is also verifiable.
- **gate:** `scripts/verify.sh --rust-only` (or `CI_SKIP_WEB=1 make ci`) green.
- *Deferred to #2:* binding the signature to the delivered room (DM rooms use a random suffix, not a
  reproducible hash, so room-binding rides the per-room hash chain). `mentions`/anti-reorder ride #2.

## Review — iteration 1 DONE (2026-06-29) ✅ `VERIFY: PASS` (`--rust-only`)
Shipped **authenticated messages** with **zero hub/protocol/schema change** (signature rides inside
`parts` as a `com.parler.sig` extension — works against the live deployed hub):
- `parler-protocol/hub.rs`: `MESSAGE_SIG_KIND`, `is_message_sig_part`, `MessageSig{sig,ts,uid,target}`
  (`to_part`/`from_parts`), `canonical_message_bytes(...)` (JCS-style, filters the sig part so signer
  and verifier can't disagree on framing). +2 codec tests.
- `parler-connector`: `MeshAgent::send` auto-signs when an identity is present (so send / push /
  session-seed are all authenticated); `verify_message(from_id, parts, reply_to) -> SigStatus`
  (`Valid`/`Unsigned`/`Invalid`); added the `uuid` dep. +6 unit tests (valid, altered, forged-from,
  replyTo-covered, target-covered, unsigned).
- `parler-cli`/MCP: `render_message` prefixes `⚠` (unsigned) / `✗ UNVERIFIED` (tampered) — valid is
  clean (silent success); `render_parts` hides the sig part. (One choke point → both CLI & MCP.)
- `parler-hub`: `/api/session` web viewer drops the sig part server-side.
- **e2e** (`mesh_e2e.rs`, real WS hub, +5): signed channel + DM verify; hub-altered content → `Invalid`;
  forged `from` → `Invalid`; pushed `Delivery` verifies; legacy unsigned → `Unsigned`. **No regressions**
  — code-handoff/push/session tests still green with signatures riding along (28/28).
- **Property proven:** a compromised/malicious hub can no longer forge or alter authored content — it's
  reduced to drop/withhold (a liveness problem, addressed by #3 idempotency + #2 chain), never an
  integrity one. The signed-card guarantee now extends to the conversation itself.
- *Next (#2):* per-room hash chain (`prev`) for tamper-evident ordering + fork/equivocation detection.

---

# Structured handoff messages (`com.parler.handoff`) — 2026-06-29

Build the "more autonomous handoff" feature promised in discussion #49. The wakeup primitive
(`recv --watch` / `parler_recv wait_secs`, #37) and outbound timeline streaming (hooks, #50) already
ship. The missing piece is **explicit "you're up next" semantics**: a structured handoff part that a
worker loop / host agent can detect and act on. Rides existing room/cursor/push machinery — no new
protocol frame, no hub change.

## Design

`com.parler.handoff` extension part (mirrors `BundleRef`):
- `next: String` (required) — the instruction for the next agent
- `summary: Option<String>` — recap of what was just done / current state
- `to: Option<String>` — addressee: target agent **name or role**; absent = "any agent in the room"
- `bundle: Option<String>` — optional blob id of an attached code bundle (cross-link to BundleRef)

`HandoffRef::{to_part, from_part, is_for(name, role)}` + `HANDOFF_KIND` const in `parler-protocol`.

## Tasks

- [x] protocol: add `HANDOFF_KIND` + `HandoffRef` (to_part/from_part/is_for) + round-trip test
- [x] cli: `parler handoff [--room|--to|--service] --next <s> [--summary <s>] [--for <who>] [--bundle <id>]`
- [x] cli: render handoff in `render_parts` (🤝 line)
- [x] mcp: `parler_handoff` tool (sends; defaults to active session)
- [x] mcp: in `parler_recv`/`parler_send` results, prepend a "🤝 HANDOFF TO YOU" banner when an
      incoming handoff is addressed to this agent (name/role match or unaddressed) — the nudge that
      makes the host continue autonomously
- [x] docs: `docs/agent-mesh.md` handoff section + the `recv --watch` worker pattern; README mention
- [x] tests: protocol round-trip, mcp handoff send→recv banner; `CI_SKIP_WEB=1 make ci` green

## Review

Shipped `com.parler.handoff` — structured turn handoff with explicit "you're up next" semantics.

- **No protocol frame / hub change.** It's an extension `Part` (like `com.parler.bundle`), so it
  rides the existing room / cursor / push / durability machinery untouched. Old clients/hubs still
  interoperate (they just see a renderable extension part).
- **`HandoffRef` mirrors `BundleRef`**: `next` (required), optional `summary` / `to` / `bundle`, plus
  `to_part` / `from_part` / `is_for(name, role)` (case-insensitive name-or-role match; unaddressed =
  everyone).
- **The autonomous nudge** is the receiver side: when a handoff addressed to *me* lands, the MCP
  `parler_recv` / `parler_send` result is prefixed with a `🤝 HANDOFF TO YOU` banner — an explicit
  instruction to act on now. Pair with `recv --watch` / `parler_recv wait_secs` (the #37 push) for a
  worker that continues the instant it's handed the turn.
- **Surfaces:** `parler handoff` CLI + `parler_handoff` MCP tool; rendering in `render_parts`; docs
  in `docs/agent-mesh.md` (+ README example matching the discussion's flow).
- **Tested end-to-end:** protocol round-trip/addressing unit test; two MCP tests that boot a real
  in-memory hub, connect real agents, send a handoff through it, and assert the banner appears for
  the addressee and *not* for a bystander in the same room. `CI_SKIP_WEB=1 make ci` green.

Honest boundary (documented): "agent B continues with zero prompting in its *own separate chat*"
still needs the host to inject a turn on an incoming event. Parler now delivers the handoff instantly
and carries the intent; the final "now go" hop is the host's (or a `recv --watch` worker).
