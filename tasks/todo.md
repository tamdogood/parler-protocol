# Fun agent names + richer session-open output

## Goal
1. Give agents fun, random-looking default names (e.g. `mellow-otter-a3f2`) instead of
   `codex-tam` / `claude-code-tam` / `$USER-suffix`.
2. When a session/room is opened, surface the agent's own name (and keep the watch code)
   in the `parler_open_session` output so the host can relay it.

## Plan
- [x] Add `crates/parler-cli/src/names.rs` — deterministic `fun_name(seed)` → `adjective-animal-<hex>`.
- [x] Wire `fun_name` into `connect.rs::default_agent_name` (seed = `<host>-<user>`).
- [x] Wire `fun_name` into `mcp.rs::load_or_bootstrap_config` + `lib.rs::cmd_init` (seed = minted nkey id).
- [x] Add the agent name line to `open_session` output.
- [x] Update affected unit tests + removed dead `name_suffix`.
- [x] Docs: README env table + rustdoc.
- [x] `make ci` green + desktop typecheck green.

## Follow-on (surfaced while implementing)
Fun names broke a latent assumption in the desktop **dial-in indicator**: it matched the bare host
id (`codex`) against directory card names, but cards carry the wired `PARLER_NAME` (already
`codex-<user>` since #103, now a fun handle). Fixed by surfacing `card_name` in `connect --json`
(the exact wired name, recomputed deterministically) and matching the indicator on it.

## Review
- New: `crates/parler-cli/src/names.rs` (fun-name generator + tests).
- `connect.rs`: `default_agent_name` → fun handle; `emit_json` adds `card_name`; tests updated.
- `mcp.rs`: bootstrap uses fun name; `open_session` surfaces `you are '<name>'`; budget 900→960;
  removed dead `name_suffix` + test; #103 test rewritten around `fun_name`.
- `lib.rs`: `parler init` default → fun name.
- Desktop: `card_name` threaded through `ConnectResult` → `DialInList` matches on it.
- README + rustdoc naming descriptions refreshed.
- Verified: `parler init` → `jolly-falcon-34b1`; `connect` per-host distinct fun names;
  `open_session` renders the name line (885 B, ≤960); `card_name` == wired `PARLER_NAME`.
