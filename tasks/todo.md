# Auto-install the wake (Stop) hook so agents keep polling (branch madrid)

Goal: agents in a session auto-poll for peers' messages and continue on their own,
instead of the human running `parler recv`. Make `parler connect` wire the Claude Code
Stop hook automatically (default-on, `--no-hooks` opt-out, user scope).

## Plan
- [ ] `parler hook stop` (new wake kind in `cmd_hook`): gated on an active session (instant
      no-op otherwise), blocks up to `PARLER_WAKE_WAIT_SECS` (default 30s) for new peer
      messages, drains + advances cursor, prints Claude Code `{"decision":"block","reason":…}`
      so the turn continues. Dependency-free (no jq/bash).
- [ ] `connect.rs`: on `Wiring::ClaudeCli` wire, install the Stop hook into
      `~/.claude/settings.json` (idempotent, preserves other hooks; 0600 only when it carries a
      join secret). `--remove` tears it out. Gate on `Options.install_hooks` (`--no-hooks`).
- [ ] Wire `--no-hooks` through `ConnectArgs` -> `Options` -> `run` / `run_remove`.
- [ ] Docs: README + docs/agent-mesh.md + docs/communication.md — hook is auto-installed now.
- [ ] Tests: settings.json install idempotent + preserves other hooks; remove drops only ours.
- [ ] `make ci` green.

## Review — ✅ DONE
- `parler hook stop` (wake path) added to `cmd_hook`; new `wake_hook()` in lib.rs. Gated on active
  session (instant no-op otherwise). Blocks up to `PARLER_WAKE_WAIT_SECS` (30s) for peers, drains +
  advances cursor, prints `{"decision":"block","reason":…}`.
- **Bug caught in verification:** `pull` returns the agent's *own* posts too, so the first draft woke
  A on its own seeded context → self-loop. Fixed by filtering `m.from.id != ag.id` (still committing
  the whole batch so own posts don't re-trigger). Re-verified E2E: own seed ignored, peer wakes.
- `connect.rs`: installs the Stop hook into `~/.claude/settings.json` on Claude Code wire
  (idempotent, preserves other hooks/settings, 0600 only when it carries a join secret); `--remove`
  tears it out. `--no-hooks` opt-out threaded through `ConnectArgs → Options`.
- Docs updated: README, docs/agent-mesh.md, docs/communication.md.
- Tests: +3 connect.rs unit tests (install idempotent/preserving, fresh-file + secret quoting, remove
  no-op). E2E smoke against a local hub. `CI_SKIP_WEB=1 make ci` green.
- Note: existing Claude Code users must re-run `parler connect` once to pick up the hook.
