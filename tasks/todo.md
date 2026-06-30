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
