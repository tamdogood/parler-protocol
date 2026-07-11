#!/usr/bin/env bash
#
# hackathon-demo.sh — stage the two-person session flow end to end, the way a team at a hackathon
# actually uses Parler Protocol: one person opens a session, shares one key, a teammate's agent joins with
# the full context, they exchange messages and code, and a read-only watch code lets anyone follow
# along in the browser. Everything runs against a local hub on this machine.
#
#   ./scripts/hackathon-demo.sh
#   PARLER_HUB_ADDR=127.0.0.1:8080 ./scripts/hackathon-demo.sh
#
# Two identities stand in for two people: `alice` (the host) and `bob` (the teammate). Each gets its
# own PARLER_HOME, exactly as two laptops would. Leave it running to open the web viewer; Ctrl-C
# tears the hub down and cleans up.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PARLER="${PARLER_BIN:-$ROOT/target/debug/parler}"
ADDR="${PARLER_HUB_ADDR:-127.0.0.1:7070}"
DIR="${PARLER_DEMO_DIR:-$ROOT/.hackathon-demo}"
DB="$DIR/hub.sqlite"

if [[ ! -x "$PARLER" ]]; then
  echo "→ building the parler binary…"
  (cd "$ROOT" && cargo build -p parler-bin)
fi

rm -rf "$DIR"
mkdir -p "$DIR"

banner() { printf '\n\033[1m── %s\033[0m\n' "$1"; }

echo "→ starting a local hub on $ADDR"
PARLER_HUB_ADDR="$ADDR" "$PARLER" hub --db "$DB" --name "Hackathon Hub" --public >"$DIR/hub.log" 2>&1 &
HUB_PID=$!
cleanup() { echo; echo "→ shutting down the demo hub"; kill "$HUB_PID" 2>/dev/null || true; wait "$HUB_PID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

for _ in $(seq 1 50); do
  curl -fsS "http://$ADDR/health" >/dev/null 2>&1 && break
  sleep 0.1
done

# Two people, two machines → two PARLER_HOMEs.
init_person() { PARLER_HOME="$DIR/$1" "$PARLER" init --hub "parler://$ADDR" --name "$1" --role "$2" --force >/dev/null; }
as() { local who="$1"; shift; PARLER_HOME="$DIR/$who" "$PARLER" "$@"; }

init_person alice host
init_person bob   teammate

# ── 1. alice (the host) opens a session, seeded with a recap ────────────────────────────────────
banner "1. alice opens a session and gets a key"
OPEN_OUT="$(as alice session open \
  --topic hackathon \
  --context "Building a Next.js dashboard. Auth is done in src/auth.ts; wiring the /api/session viewer next. Blocker: the watch token 401s.")"
echo "$OPEN_OUT"
KEY="$(printf '%s\n' "$OPEN_OUT"  | sed -n 's/^[[:space:]]*KEY:[[:space:]]*\([^[:space:]]*\).*/\1/p' | head -1)"
ROOM="$(printf '%s\n' "$OPEN_OUT" | sed -n "s/.*room '\\([^']*\\)'.*/\\1/p" | head -1)"
[[ -n "$KEY" && -n "$ROOM" ]] || { echo "!! couldn't parse KEY/ROOM from session open"; exit 1; }

banner "…she drops ONE line in the team chat (no install, no setup for bob):"
echo "    claude mcp add parler -e PARLER_SESSION_KEY=$KEY -- parler mcp"

# ── 2. bob (the teammate) joins — held pending until alice approves ─────────────────────────────
banner "2. bob asks to join with the key — he is held pending (can't read a word yet)"
as bob session join "$KEY"

# ── 3. alice sees the request and approves bob ─────────────────────────────────────────────────
banner "3. alice sees who's knocking, and approves bob"
as alice session requests --room "$ROOM"
BOB_ID="$(sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$DIR/bob/config.json" | head -1)"
[[ -n "$BOB_ID" ]] || { echo "!! couldn't read bob's id"; exit 1; }
as alice session approve --room "$ROOM" "$BOB_ID"

# ── 4. bob joins for real — the full context lands in the same call ────────────────────────────
banner "4. bob joins again — now he lands with the whole backlog"
as bob session join "$KEY"

# ── 5. they talk in the same room ──────────────────────────────────────────────────────────────
banner "5. two people, one conversation"
as bob   send --room "$ROOM" "on it — the 401 is because the watch code isn't the join key" >/dev/null
as alice recv --room "$ROOM"
as alice send --room "$ROOM" "nice. pushing my branch so you can pull it" >/dev/null
as bob   recv --room "$ROOM"

# ── 6. hand off actual code as a git bundle (best-effort; needs a commit to bundle) ────────────
# `parler push` bundles the cwd's git repo, so run it from the repo root regardless of where this
# script was invoked from.
banner "6. alice hands off code as a git bundle (never auto-merged)"
if ( cd "$ROOT" && PARLER_HOME="$DIR/alice" "$PARLER" push --room "$ROOM" --base HEAD~1 --note "the reconnect patch" ) 2>/dev/null; then
  as bob recv --room "$ROOM"
else
  echo "   (skipped — no HEAD~1 to bundle in this checkout)"
fi

# ── 7. anyone can WATCH it live in the browser (read-only, separate from the join key) ─────────
banner "7. alice mints a read-only watch code for the browser viewer"
WATCH_OUT="$(as alice session watch --room "$ROOM")"
echo "$WATCH_OUT"
WATCH="$(printf '%s\n' "$WATCH_OUT" | sed -n 's/^[[:space:]]\{2,\}\([A-Za-z0-9]\{16,\}\)[[:space:]]*$/\1/p' | head -1)"

banner "…proving the gate: the WATCH code reads the session, the JOIN key does NOT"
WATCH_CODE="$(curl -s -o "$DIR/session.json" -w '%{http_code}' "http://$ADDR/api/session?token=$WATCH")"
KEY_CODE="$(curl -s -o /dev/null -w '%{http_code}' "http://$ADDR/api/session?token=$KEY")"
echo "   GET /api/session  with watch code → HTTP $WATCH_CODE   (roster + messages)"
echo "   GET /api/session  with join key   → HTTP $KEY_CODE   (rejected — a key can't read the backlog)"
echo "   viewer JSON saved to: $DIR/session.json"

cat <<EOF

$(banner "done — the whole team flow, from one key")
Watch this session live:  parler session watch $WATCH

The hub is still up so you can try it. Press Ctrl-C to tear it down.
EOF

# Keep the hub alive so the web viewer can connect.
while kill -0 "$HUB_PID" 2>/dev/null; do sleep 3600 & wait $!; done
