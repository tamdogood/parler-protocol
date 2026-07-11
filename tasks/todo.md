# Make `parler_fetch` find a shared file by itself

**Problem:** A receiver agent asked to "fetch the file" replies it has no blob id —
`parler_fetch` requires a 64-char blob id, so the agent can't act without a human pasting it.

## Plan
- [ ] `parler_fetch`: make `id` optional; add `name` + `room` params.
- [ ] Route `parler_fetch` through `call_session_tool` so it can default to the active session.
- [ ] Add `looks_like_blob_id` (64 lowercase-hex) to tell a real id from a filename hint.
- [ ] Add `resolve_recent_blob(agent, room, name_hint)` — page the room history (pure `since`
      re-reads, no cursor move), collect `FileRef`/`BundleRef` parts, return the newest match
      (name-filtered when a hint is given), plus a suggested output filename.
- [ ] Default `out` to the resolved filename when `-o`/`out` is absent.
- [ ] Update the tool schema/description + `parler_send_file` hint.
- [ ] Docs: `docs/file-transfer.md`, `README.md`.
- [ ] Tests: MCP fetch-by-nothing and fetch-by-name; keep the explicit-id path green.
- [ ] `make ci` green.

## Review

Done. `parler_fetch`'s `id` is now optional. With no id (or a filename passed where an id was
expected), it pages the active session's history — pure `since` re-reads that never move a delivery
cursor, so `parler_recv` is untouched — collects every `com.parler.file`/`com.parler.bundle`
reference, and downloads the most recent match (name-filtered when `name` is given), defaulting the
output name to the file's own basename. A real blob id (64-char lowercase-hex) still fetches exactly.

- `crates/parler-cli/src/mcp.rs`: routed `parler_fetch` through the session-aware handler (for
  active-session default), added `looks_like_blob_id` + `resolve_recent_blob`, reworked the tool
  schema (`id` optional; `name`/`room` added), updated the `parler_send_file` hint, bumped
  `TOOL_SPECS_BUDGET` 13,200 → 13,500 (documented; load-bearing schema, not description bloat).
- Tests: extended `test_mcp_send_file_recv_fetch_e2e` with id-less auto-find, fetch-by-`name`, and a
  no-match error, over a real hub. `make ci` green (clippy -D warnings, deny, docs-drift all pass).
- Docs: `docs/file-transfer.md` updated (usage, what-changed, verified). CLI `parler fetch` still
  takes an explicit blob id — unchanged, and docs say so.
