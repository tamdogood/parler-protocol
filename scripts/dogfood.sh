#!/usr/bin/env bash
#
# dogfood.sh — use Parler Protocol the way real users will, with zero manual agent wrangling.
#
# The other scripts in here are *demos*: they replay a fixed line of hand-written send/recv calls,
# so they prove the wire works but never test whether a real agent can figure the tools out. This
# one is different — it stands up a local hub and then launches a small team of **real headless
# Claude agents**, each wired to the Parler MCP server exactly as a user's `parler connect` would
# wire it. They discover each other, land in one shared session, and actually talk, decide, and hand
# off work on their own. You just watch.
#
# That is the point of dogfooding: the friction the agents hit (a confusing tool name, a missing
# nudge to poll for replies, an awkward approval step) is the friction your users will hit.
#
#   ./scripts/dogfood.sh                 # 3 agents, a fast model, one shared task
#   AGENTS=4 ./scripts/dogfood.sh        # bigger room
#   ROUNDS=5 ./scripts/dogfood.sh        # more back-and-forth
#   TASK="Pick the on-disk format for session exports and record the decision." ./scripts/dogfood.sh
#   MODEL=sonnet ./scripts/dogfood.sh    # any `claude --model` alias or full id
#
# Heads up: each round runs one `claude` turn per agent, so this spends real Claude usage. Keep
# AGENTS/ROUNDS small while you are just kicking the tires. Ctrl-C tears everything down.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PARLER="${PARLER_BIN:-$ROOT/target/debug/parler}"
ADDR="${PARLER_HUB_ADDR:-127.0.0.1:7070}"
DIR="${PARLER_DOGFOOD_DIR:-$ROOT/.dogfood}"
DB="$DIR/hub.sqlite"

AGENTS="${AGENTS:-3}"          # how many real agents to seat in the room
ROUNDS="${ROUNDS:-3}"          # how many turns each agent takes at the shared task
MODEL="${MODEL:-claude-haiku-4-5-20251001}"   # a fast, cheap, tool-capable default
TASK="${TASK:-Agree on the WebSocket reconnect backoff policy for the hub: max retries, base delay, jitter, and when to give up. Discuss briefly, converge, and have ONE agent record the final decision with parler_remember.}"

command -v claude >/dev/null 2>&1 || { echo "!! the 'claude' CLI is not on PATH — install Claude Code first"; exit 1; }

if [[ ! -x "$PARLER" ]]; then
  echo "→ building the parler binary (waits if the cargo lock is held)…"
  (cd "$ROOT" && cargo build -p parler-bin)
fi

rm -rf "$DIR"
mkdir -p "$DIR"

banner() { printf '\n\033[1m── %s\033[0m\n' "$1"; }
say()    { printf '\033[2m   %s\033[0m\n' "$1"; }

# ── 0. a local hub, on this machine ─────────────────────────────────────────────────────────────
echo "→ starting a local hub on $ADDR"
PARLER_HUB_ADDR="$ADDR" "$PARLER" hub --db "$DB" --name "Dogfood Hub" --public >"$DIR/hub.log" 2>&1 &
HUB_PID=$!

AGENT_PIDS=()
cleanup() {
  [[ -n "${CLEANED:-}" ]] && return
  CLEANED=1
  echo
  echo "→ shutting down (hub + any live agents)"
  for p in ${AGENT_PIDS[@]:+"${AGENT_PIDS[@]}"}; do kill "$p" 2>/dev/null || true; done
  kill "$HUB_PID" 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

for _ in $(seq 1 50); do
  curl -fsS "http://$ADDR/health" >/dev/null 2>&1 && break
  sleep 0.1
done

# ── 1. seed one shared session and grab its KEY ─────────────────────────────────────────────────
# A "lead" identity opens the room with `--no-approval`, so any agent that boots its MCP server with
# PARLER_SESSION_KEY set lands already caught up — no gate to click through. Distributing that one
# key is the only thing this script orchestrates; everything after it, the agents do themselves.
as_lead() { PARLER_HOME="$DIR/lead" "$PARLER" "$@"; }
as_lead init --hub "parler://$ADDR" --name lead --role coordinator --force >/dev/null

banner "1. the lead opens a shared session and gets a key"
OPEN_OUT="$(as_lead session open --topic dogfood --no-approval \
  --context "Team room for a live task. Introduce yourself when you arrive, read the backlog with parler_recv before you post, and keep messages short. The task: $TASK")"
echo "$OPEN_OUT"
KEY="$(printf '%s\n' "$OPEN_OUT"  | sed -n 's/^[[:space:]]*KEY:[[:space:]]*\([^[:space:]]*\).*/\1/p' | head -1)"
ROOM="$(printf '%s\n' "$OPEN_OUT" | sed -n "s/.*room '\\([^']*\\)'.*/\\1/p" | head -1)"
[[ -n "$KEY" && -n "$ROOM" ]] || { echo "!! couldn't parse KEY/ROOM from 'session open'"; exit 1; }

# ── 2. seat N real agents, each wired to the Parler MCP server ───────────────────────────────────
# A cast to draw names/roles from; the room uses the first $AGENTS of them.
NAMES=(atlas probe forge sentry relay muse echo)
ROLES=(planner researcher engineer reviewer coordinator designer ops)

# One MCP config per agent — the exact shape `parler connect` writes, pointed at our local hub. The
# agent boots with PARLER_SESSION_KEY so it auto-joins the shared room and pulls the backlog.
write_mcp_config() {  # <name> <role> <home> <outfile>
  local name="$1" role="$2" home="$3" out="$4"
  cat >"$out" <<JSON
{
  "mcpServers": {
    "parler": {
      "command": "$PARLER",
      "args": ["mcp"],
      "env": {
        "PARLER_HUB": "ws://$ADDR",
        "PARLER_HOME": "$home",
        "PARLER_NAME": "$name",
        "PARLER_ROLE": "$role",
        "PARLER_SESSION_KEY": "$KEY"
      }
    }
  }
}
JSON
}

# What we ask each agent to do. It only has the Parler tools, so it cannot touch files or the shell —
# a clean sandbox. It has to collaborate purely through the mesh.
prompt_for() {  # <name> <role>
  local name="$1" role="$2"
  cat <<TXT
You are "$name", a $role agent, and you have just joined a shared Parler session with your teammates.
You can ONLY use the parler_* tools — that is how you talk to the others.

The task for the room:
$TASK

Do this, in order:
1. See who else is here with parler_roster (and parler_discover if you're curious).
2. Read what has been said with parler_recv BEFORE you post, so you don't repeat anyone.
3. Add your contribution as a $role with parler_send — one short, specific message. Then parler_recv
   again to see replies, and respond if it moves the task forward.
4. If the room has converged on a decision and nobody has recorded it yet, record it with
   parler_remember so it survives the session.

Keep every message to a sentence or two. Don't monologue. Stop once you've made your point for this turn.
TXT
}

banner "2. seating $AGENTS real agents (model: $MODEL) — each auto-joins with the key"
declare -a A_NAME A_ROLE A_CFG A_LOG
for ((i=0; i<AGENTS; i++)); do
  n="${NAMES[$((i % ${#NAMES[@]}))]}"
  r="${ROLES[$((i % ${#ROLES[@]}))]}"
  home="$DIR/$n"; cfg="$DIR/$n.mcp.json"; log="$DIR/$n.log"
  mkdir -p "$home"
  write_mcp_config "$n" "$r" "$home" "$cfg"
  A_NAME[i]="$n"; A_ROLE[i]="$r"; A_CFG[i]="$cfg"; A_LOG[i]="$log"
  say "• $n ($r)  →  log: ${log#"$ROOT"/}"
done

# Run one agent's turn. Everything it says lands in the shared room, so its teammates read it on
# their next turn — the room itself is the shared memory between runs.
run_turn() {  # <index> <round>
  local i="$1" round="$2"
  {
    printf '\n\033[1m═══ %s · round %s ═══\033[0m\n' "${A_NAME[$i]}" "$round"
    claude -p "$(prompt_for "${A_NAME[$i]}" "${A_ROLE[$i]}")" \
      --model "$MODEL" \
      --mcp-config "${A_CFG[$i]}" --strict-mcp-config \
      --allowedTools "mcp__parler" \
      --permission-mode bypassPermissions 2>&1 || echo "(turn exited non-zero)"
  } >>"${A_LOG[$i]}" 2>&1
}

# ── 3. run the conversation: every agent takes a turn each round, in parallel ────────────────────
banner "3. the room is live — $ROUNDS rounds, all agents in parallel each round"
say "tailing every agent's log below; Ctrl-C to stop."
echo
# Live view of what the agents are saying to each other.
tail -n +1 -F "${A_LOG[@]}" 2>/dev/null &
AGENT_PIDS+=($!)

for ((round=1; round<=ROUNDS; round++)); do
  turn_pids=()
  for ((i=0; i<AGENTS; i++)); do
    run_turn "$i" "$round" &
    turn_pids+=($!)
    sleep 1   # small stagger so they don't all fire the exact same instant
  done
  for p in "${turn_pids[@]}"; do wait "$p" 2>/dev/null || true; done
done

# ── 4. show the result, and leave the hub up so you can inspect it ───────────────────────────────
sleep 1
banner "4. what landed in the shared room"
as_lead recv --room "$ROOM" || true
banner "…and what the room chose to remember"
as_lead recall "$TASK" 2>/dev/null || say "(no facts recorded — a sign the 'record the decision' nudge didn't land)"

# A read-only watch code so you can replay the whole thing in the browser.
WATCH_OUT="$(as_lead session watch --room "$ROOM" 2>/dev/null || true)"
WATCH="$(printf '%s\n' "$WATCH_OUT" | sed -n 's/^[[:space:]]\{2,\}\([A-Za-z0-9]\{16,\}\)[[:space:]]*$/\1/p' | head -1)"

cat <<EOF

$(banner "done — a real team of agents dogfooded the mesh on their own")
Per-agent transcripts:   $DIR/*.log
Hub log:                 $DIR/hub.log
Inspect the hub:         curl -s http://$ADDR/api/hub | jq .

Replay it:  parler session watch ${WATCH:-<run session watch to mint one>}

The hub is still up. Press Ctrl-C to tear it all down.
EOF

# Keep the hub alive so you can poke at it / open the web viewer; Ctrl-C triggers cleanup.
while kill -0 "$HUB_PID" 2>/dev/null; do sleep 3600 & wait $!; done
