# Fix: portable invite/session codes carry their hub (once-and-for-all cross-hub join)

## Problem
A bare invite code (`ZX6Y2QPX`) doesn't say which hub it belongs to. An agent on a
different hub redeems it and gets a cryptic `invalid or unknown invite code` — the exact
failure that made a Codex hand-off take a long, confusing detour while Claude (already on
the hosted hub) joined instantly.

A portable form `<code>@<hub>` already exists, but:
- only `parler session join` understood it — `parler join` did not;
- nothing led with it, so the copy-pasted line was always the bare code;
- the **MCP tools** (`parler_join`, `parler_join_session`) — how agents actually join —
  didn't understand it at all and gave the bare cryptic error.

## Plan (parler-cli + docs; no wire-protocol change)

### CLI (`crates/parler-cli/src/lib.rs`)
- [ ] `explain_unknown_code()` helper: rewrite the terminal "unknown invite code" into a
      signpost (which hub we tried + the portable form that carries the minting hub).
- [ ] `cmd_join`: accept `<code>@<hub>` (strip + dial the embedded hub, like `session join`);
      wrap the redeem error with the signpost.
- [ ] `cmd_invite`: lead the hand-off line with the portable `<code>@<hub>`; bare as fallback.
- [ ] `session open`: lead with the portable key; bare as fallback.
- [ ] `cmd_session` Join: wrap the redeem error with the signpost.
- [ ] Unit tests for `explain_unknown_code`.

### MCP (`crates/parler-cli/src/mcp.rs`) — the path agents actually use
- [ ] `same_hub()` loose URL equality (ignore scheme + trailing slash).
- [ ] `portable_code_for_hub()`: strip `@<hub>`; same hub → bare code; different hub → a
      precise error ("this invite is on <hub>; relaunch your MCP server with PARLER_HUB=<hub>")
      instead of the cryptic one — the MCP agent is single-hub by design (#99), so it can't
      transparently cross hubs, but it can say exactly how to fix it.
- [ ] `explain_unknown_code_mcp()` for a bare unknown code.
- [ ] `parler_join` + `join_session`: resolve portable code, apply the signpost.
- [ ] invite + open_session output: hand off the portable `<code>@<hub>`.
- [ ] Unit tests for `same_hub` + `portable_code_for_hub`.

### Docs
- [ ] Update `docs/agent-mesh.md` (+ README/AGENTS if they show the hand-off) so the
      documented hand-off is the portable form and cross-hub behavior is described.

## Gate
- [ ] `make ci` green (clippy -D warnings, tests, fmt untouched by hand).

## Review — ✅ DONE
- **CLI** (`lib.rs`): `explain_unknown_code` signposts the wrong-hub error; `cmd_join` now accepts
  `<code>@<hub>` (strips + dials the embedded hub) and wraps the error; `cmd_invite` + `session open`
  lead the hand-off with the portable `<code>@<hub>`; `cmd_session` Join wraps the redeem error.
- **MCP** (`mcp.rs`): `same_hub` + `portable_code_for_hub` + `explain_unknown_code_mcp`; `parler_join`
  and `join_session` resolve the portable code and, for a different hub, return the exact relaunch fix
  (single-hub agent, #99) instead of the cryptic error; invite + open_session output hand off the
  portable `<code>@<hub>`.
- **Docs**: `docs/agent-mesh.md` portable-codes section rewritten (covers `parler join`, the
  signposted error, and the MCP single-hub behavior) + command-table note.
- **Tests**: +3 unit tests (`explain_unknown_code`, `same_hub`, `portable_code_for_hub`).
- **Gate**: `make ci` fully green (clippy -D warnings, tests, cargo-deny).
- **E2E verified** against two live local hubs (7090 = invite's hub, 7091 = joiner's wrong default):
  bare code → signpost naming hub 7091 + the portable form; `join CVLM9LNK@ws://127.0.0.1:7090` from
  the same joiner → `✓ joined channel room 'demo'`.
- Note: no wire-protocol change; existing bare-code hand-offs still work on the same hub.
