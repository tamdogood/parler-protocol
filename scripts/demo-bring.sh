#!/usr/bin/env bash
#
# demo-bring.sh — the "second opinion in one line" wedge, end to end. This is the rehearsal rig for
# the short demo: mid-chat, you ask for an independent review from another AI agent and it lands in
# your session — no copy-paste, no switching windows.
#
# It stands up a LOCAL hub on this machine, opens a session (as the agent you're chatting with),
# then runs `parler bring codex` to get a second opinion on some real context. The review is posted
# straight into the session, so `parler recv` shows it arrive as a normal message — exactly what the
# `parler_bring` MCP tool does when your primary agent calls it mid-conversation.
#
#   ./scripts/demo-bring.sh
#   PARLER_HUB_ADDR=127.0.0.1:8080 ./scripts/demo-bring.sh
#
# Requires the `codex` CLI installed and logged in (`codex login`); it's the v1 review agent. If it
# isn't present, `bring` prints the exact remedy instead of a review. Nothing leaves the box beyond
# the codex API call itself. Ctrl-C (or normal exit) tears the hub down and cleans up.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PARLER="${PARLER_BIN:-$ROOT/target/debug/parler}"
ADDR="${PARLER_HUB_ADDR:-127.0.0.1:7070}"
DIR="${PARLER_DEMO_DIR:-$ROOT/.demo-bring}"
DB="$DIR/hub.sqlite"

if [[ ! -x "$PARLER" ]]; then
  echo "→ building the parler binary (waits if the cargo lock is held)…"
  (cd "$ROOT" && cargo build -p parler-bin)
fi

if ! command -v codex >/dev/null 2>&1; then
  echo "!! this demo needs the 'codex' CLI (the v1 review agent). Install it and run 'codex login', then retry."
  echo "   (the feature degrades gracefully — 'parler bring' would print that same remedy — but the demo has nothing to show without it.)"
  exit 1
fi

rm -rf "$DIR"
mkdir -p "$DIR"

banner() { printf '\n\033[1m── %s\033[0m\n' "$1"; }
say()    { printf '\033[2m   %s\033[0m\n' "$1"; }

# ── 0. a local hub, on this machine ─────────────────────────────────────────────────────────────
echo "→ starting a LOCAL hub on $ADDR (loopback)"
PARLER_HUB_ADDR="$ADDR" "$PARLER" hub --db "$DB" --name "Bring Demo Hub" >"$DIR/hub.log" 2>&1 &
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

for _ in $(seq 1 50); do
  curl -fsS "http://$ADDR/health" >/dev/null 2>&1 && break
  sleep 0.1
done

# One tool, its own PARLER_HOME — the agent you're chatting with.
as() { PARLER_HOME="$DIR/you" "$PARLER" "$@"; }
PARLER_HOME="$DIR/you" "$PARLER" init --hub "parler://$ADDR" --name you --role "the agent you're chatting with" --force >/dev/null

# ── 1. you're mid-chat and open a session to work in ────────────────────────────────────────────
banner "1. you're mid-chat — open a session for this piece of work"
CONTEXT="Reviewing a change to src/auth.rs: the login handler compares the submitted password hash to the stored one with '==', and unwrap()s the u32 parse of a 'max_attempts' form field. Want a second opinion before I ship."
say "what you want a second opinion on:"
say "\"$CONTEXT\""
OPEN_OUT="$(as session open --topic auth-review --no-approval --context "$CONTEXT")"
ROOM="$(printf '%s\n' "$OPEN_OUT" | sed -n "s/^✓ session open — room '\\([^']*\\)'.*/\\1/p" | head -1)"
[[ -n "$ROOM" ]] || { echo "!! couldn't parse ROOM from 'session open'"; exit 1; }
say "session room: $ROOM"

# ── 2. one line: bring codex for an independent review ──────────────────────────────────────────
banner "2. one line — ask codex for an independent second opinion"
say "parler bring codex --context \"…\" --room $ROOM"
START=$(date +%s)
printf '%s' "$CONTEXT" | as bring codex --context-file - --room "$ROOM" --quiet --timeout-secs 180
END=$(date +%s)
say "codex reviewed in $((END - START))s — its answer was posted straight into the session"

# ── 3. the review is just there, in your conversation ───────────────────────────────────────────
banner "3. no copy-paste — the second opinion landed in your session"
as recv --room "$ROOM" --all

cat <<EOF

$(banner "done — an independent review, in one line, no window-switching")
Your primary agent would call the parler_bring MCP tool and read this with parler_recv — same flow.

Press Ctrl-C to tear the hub down.
EOF

while kill -0 "$HUB_PID" 2>/dev/null; do sleep 3600 & wait $!; done
