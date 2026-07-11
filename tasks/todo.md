# Add popular agents to `parler connect`

Goal: make `parler connect` (and the desktop app picker, a thin driver over the same registry)
wire the most popular MCP agents — starting with the one that bit users (OpenCode) plus the
highest-traffic editors. One registry drives both surfaces, so this is a CLI-only change.

## Verified config formats
- **OpenCode** — `~/.config/opencode/opencode.json`, top-level `mcp`, entry `{type:"local",
  command:[bin,"mcp"], enabled:true, environment:{…}}`.
- **VS Code** (Copilot) — `~/Library/Application Support/Code/User/mcp.json` (macOS) /
  `~/.config/Code/User/mcp.json` (Linux), top-level `servers`, entry `{type:"stdio", command, args, env}`.
- **Cline** — `…/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json`,
  top-level `mcpServers`, standard entry (`command`, `args`, `env`).
- Skipped this pass: Zed (ambiguous flat-vs-nested `command` shape), Continue (moved to YAML).

## Plan
- [ ] Generalize `Wiring::Json(path)` → `Json { path, key, shape }` + `JsonShape { Standard, OpenCode, VsCode }`.
- [ ] Teach the JSON writer/reader/remover/snippet about `key` + `shape` (env field is `environment`
      for OpenCode, `env` otherwise; command is an array for OpenCode).
- [ ] Add OpenCode, VS Code, Cline to `registry()` + `canonical_id` aliases + `restart_hint` arms.
- [ ] Unit tests: write→read-back→remove for OpenCode + VS Code shapes; keep the existing 4 green.
- [ ] Docs: update the connect/agent tables in `README.md`, `AGENTS.md`, `docs/agent-mesh.md`.
- [ ] `make ci` green.

## Review
- Generalized `Wiring::Json(path)` → `Json { path, key, shape }` + `JsonShape { Standard, OpenCode,
  VsCode }`; `env_field()` handles OpenCode's `environment` vs everyone's `env`. The 4 existing JSON
  hosts moved to `Standard` with **no behavior change** (all prior tests still green).
- Added OpenCode, VS Code, Cline to `registry()` + `canonical_id` aliases + `restart_hint` arms.
  Registry is the single source of truth, so the desktop app's per-agent picker gets all three for
  free via `parler connect --list --json` — **zero TypeScript changes**.
- New unit tests round-trip the OpenCode + VS Code shapes (write → read-back via `configured_env` →
  remove), asserting no cross-shape key leakage. `cargo test connect::` = 28 passed.
- **Live-verified**: `parler connect --list` shows all three; a real wire into a sandbox HOME writes a
  correct `opencode.json` (`mcp` key, `command` array, `environment`, `type:"local"`, `enabled:true`,
  per-agent `PARLER_HOME`) — the exact config OpenCode needs to expose `parler_*` tools.
- Docs synced (no drift): README prose (×2) + per-host table (+3 rows), AGENTS.md, docs/agent-mesh.md.
- Skipped Zed (ambiguous flat-vs-nested `command` shape) + Continue (YAML migration) — flagged as
  easy follow-ups once verified on a real install. `make ci` fully green.
