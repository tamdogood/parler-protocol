# Docs revamp — bring README + all docs in line with latest code

## Findings (verified against source)
- README MCP tool list (README.md ~368) is **stale**: missing `parler_bring`,
  `parler_watch_session`, `parler_send_file`, `parler_apply` (lists 23, code advertises 27).
- New surface undocumented in README/AGENTS: `parler consolidate` CLI command + the two MCP
  prompts `parler_session_handoff` / `parler_consolidate_session` (rolling session digest).
- Session **web viewer** (`parler session watch` / `parler_watch_session`) shipped but not
  surfaced in README.
- File transfer (`parler send-file` / `parler_send_file`) not headlined in README "what you can do".
- Architecture diagram (`docs/architecture.mmd` → `architecture.png`) predates sessions,
  blobs, A2A cards, the desktop app, and the auth/join-secret detail.

## Plan
- [x] Audit CLI + MCP surface from source (authoritative lists)
- [x] Redraw `docs/architecture.mmd` for the latest design; regenerate `architecture.png`
- [x] README: fix tool list to all 27 + note prompts; add consolidate/session-digest,
      session viewer, file transfer; verify every command against code
- [x] AGENTS.md: refresh "architecture at a glance" (sessions · blobs · A2A · desktop)
- [x] docs/communication.md: add memory-consolidation prompts + confirm full tool coverage
- [x] Sweep remaining docs for any old-behavior instructions (drift tests cover phantom refs)
- [x] Gate: drift tests pass + `make ci` fully green

## Review
- Diagram (`docs/architecture.mmd` + regenerated `architecture.png`) now shows sessions, blobs,
  A2A cards, the session viewer, the Electron desktop app, and Ed25519 challenge-response / join
  secret — the design as it ships today. Rendered with `@mermaid-js/mermaid-cli` (npx), scale 3.
- README: MCP tool block now lists all **27** advertised tools + the two prompts; added a File
  transfer example, a Watch-a-session-from-the-browser note, and `parler consolidate` in the memory
  block. Every command verified against `crates/parler-cli/src/{lib,bring,mcp}.rs`.
- AGENTS.md + communication.md aligned (glance diagram, memory-consolidation prompts, at-a-glance
  tool cell). No user-facing doc left describing old behavior.
- Kept changes accuracy-focused (no rewrite of good marketing prose) per Minimal-Impact. `make ci`
  green; `sequence.mmd` left as-is (still accurate).
