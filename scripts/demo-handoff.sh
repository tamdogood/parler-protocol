#!/usr/bin/env bash
#
# demo-handoff.sh — the wedge, end to end, in one script. This is the rehearsal rig for the
# 90-second demo video: move a live coding-agent session from one tool to another in about
# 10 seconds, with no copy-paste and no re-briefing.
#
# It stands up a LOCAL hub on this machine, then plays two agents against it:
#   • agent A — the tool you started in — opens a session seeded with real working context
#     and prints a short KEY.
#   • agent B — the tool you switched to — joins with that KEY and comes up already caught up,
#     printing the exact context A had. That context transfer is the copy-paste you didn't do.
#
#   ./scripts/demo-handoff.sh
#   PARLER_HUB_ADDR=127.0.0.1:8080 ./scripts/demo-handoff.sh
#
# `codex` and `claude` stand in for two different coding tools on the same machine; each gets its
# own PARLER_HOME, exactly as two real tools would. Nothing leaves the box. Ctrl-C (or normal exit)
# tears the hub down and cleans up.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PARLER="${PARLER_BIN:-$ROOT/target/debug/parler}"
ADDR="${PARLER_HUB_ADDR:-127.0.0.1:7070}"
DIR="${PARLER_DEMO_DIR:-$ROOT/.demo-handoff}"
DB="$DIR/hub.sqlite"

# The shared cargo target dir may be locked by another build; cargo serializes on the lock, so this
# just waits its turn rather than failing.
if [[ ! -x "$PARLER" ]]; then
  echo "→ building the parler binary (waits if the cargo lock is held)…"
  (cd "$ROOT" && cargo build -p parler-bin)
fi

# Fresh state each run.
rm -rf "$DIR"
mkdir -p "$DIR"

banner() { printf '\n\033[1m── %s\033[0m\n' "$1"; }
say()    { printf '\033[2m   %s\033[0m\n' "$1"; }

# ── 0. a local hub, on this machine, nothing leaving the box ────────────────────────────────────
echo "→ starting a LOCAL hub on $ADDR (loopback; nothing leaves this machine)"
PARLER_HUB_ADDR="$ADDR" "$PARLER" hub --db "$DB" --name "Handoff Demo Hub" >"$DIR/hub.log" 2>&1 &
HUB_PID=$!
cleanup() {
  [[ -n "${CLEANED:-}" ]] && return
  CLEANED=1
  echo
  echo "→ shutting down the demo hub"
  kill "$HUB_PID" 2>/dev/null || true
  wait "$HUB_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# Wait for the hub's health endpoint before driving it.
for _ in $(seq 1 50); do
  curl -fsS "http://$ADDR/health" >/dev/null 2>&1 && break
  sleep 0.1
done

# Two tools on one machine → two PARLER_HOMEs. `as <who> …` runs a parler command as that tool.
init_tool() { PARLER_HOME="$DIR/$1" "$PARLER" init --hub "parler://$ADDR" --name "$1" --role "$2" --force >/dev/null; }
as() { local who="$1"; shift; PARLER_HOME="$DIR/$who" "$PARLER" "$@"; }

init_tool codex  "the tool you started in"
init_tool claude "the tool you switched to"

# ── 1. agent A opens a session seeded with the context it's holding right now ────────────────────
banner "1. agent A (codex) opens a session — seeded with what it's been working on"
CONTEXT="Refactoring auth in src/auth.rs. Chose PKCE + rotating refresh tokens. Done: login + token mint. TODO: token rotation and wiring the login UI. Watch out — the old /logout still nukes the whole session."
say "the context codex is holding:"
say "\"$CONTEXT\""
# The default is the pure wedge: one key in, instant catch-up, no gate to click through. The explicit
# approval-gated compatibility variant lives in ./scripts/hackathon-demo.sh.
OPEN_OUT="$(as codex session open --topic auth-redesign --context "$CONTEXT")"
echo "$OPEN_OUT"
KEY="$(printf '%s\n' "$OPEN_OUT" | sed -n 's/^[[:space:]]*KEY:[[:space:]]*\([^[:space:]]*\).*/\1/p' | head -1)"
ROOM="$(printf '%s\n' "$OPEN_OUT" | sed -n "s/^✓ session open — room '\\([^']*\\)'.*/\\1/p" | head -1)"
[[ -n "$KEY" && -n "$ROOM" ]] || { echo "!! couldn't parse KEY/ROOM from 'session open'"; exit 1; }

banner "…you switch tools. This is the ONLY thing you carry across — one short key:"
echo "    $KEY"
say "(in real life: paste it, or launch agent B with  PARLER_SESSION_KEY=$KEY  preset)"

# ── 2. agent B joins with that key — and lands already caught up ────────────────────────────────
banner "2. agent B (claude) joins with the key — no copy-paste, no re-briefing"
JOIN_OUT="$(as claude session join "$KEY")"
echo "$JOIN_OUT"

# ── 3. prove the wedge: B is holding the exact context A had ─────────────────────────────────────
banner "3. this is the copy-paste you didn't do"
if printf '%s' "$JOIN_OUT" | grep -qF "PKCE"; then
  say "agent B came up already knowing the auth design, what's done, what's TODO, and the /logout landmine."
  say "you never re-briefed it. it read the room's history in the same call it joined."
else
  echo "!! agent B did not receive the seeded context — the handoff did not land"
  exit 1
fi

# ── 4. and it's a live conversation, not a one-way dump ─────────────────────────────────────────
banner "4. it's live, both directions — B can pick up the work and A sees it"
as claude send --room "$ROOM" "got it — taking token rotation, leaving the /logout fix to you" >/dev/null
banner "…back in agent A (codex), the reply is just there:"
as codex recv --room "$ROOM"

cat <<EOF

$(banner "done — one live session, moved tool-to-tool in about 10 seconds")
No transcript pasted. No context re-typed. That is the whole pitch.

The local hub is still up if you want to poke at it:
   curl -s http://$ADDR/api/hub | jq .
Press Ctrl-C to tear it down.
EOF

# Keep the hub alive so you can inspect it after the run; Ctrl-C triggers cleanup.
while kill -0 "$HUB_PID" 2>/dev/null; do sleep 3600 & wait $!; done
