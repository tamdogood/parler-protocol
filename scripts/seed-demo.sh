#!/usr/bin/env bash
#
# seed-demo.sh — boot a public Parler Protocol hub and populate its directory with a cast of agents so the
# website (web/) has something to render. Each agent gets its own identity, publishes a SIGNED
# discovery card (some public, some private), and reports a presence status.
#
#   ./scripts/seed-demo.sh            # hub on 127.0.0.1:7070, named "Parler Protocol Public"
#   PARLER_HUB_ADDR=127.0.0.1:8080 ./scripts/seed-demo.sh
#
# Leave it running; it keeps the agents' presence fresh. Ctrl-C tears the hub down and cleans up.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PARLER="${PARLER_BIN:-$ROOT/target/debug/parler}"
ADDR="${PARLER_HUB_ADDR:-127.0.0.1:7070}"
HUB_NAME="${PARLER_HUB_NAME:-Parler Protocol Public}"
DIR="${PARLER_DEMO_DIR:-$ROOT/.demo}"
DB="$DIR/hub.sqlite"

if [[ ! -x "$PARLER" ]]; then
  echo "→ building the parler binary…"
  (cd "$ROOT" && cargo build -p parler-bin)
fi

# Fresh state each run.
rm -rf "$DIR"
mkdir -p "$DIR"

echo "→ starting a PUBLIC hub '$HUB_NAME' on $ADDR"
PARLER_HUB_ADDR="$ADDR" "$PARLER" hub --db "$DB" --name "$HUB_NAME" --public >"$DIR/hub.log" 2>&1 &
HUB_PID=$!

cleanup() {
  echo
  echo "→ shutting down the demo hub"
  kill "$HUB_PID" 2>/dev/null || true
  wait "$HUB_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# Wait for the hub's health endpoint.
for _ in $(seq 1 50); do
  if curl -fsS "http://$ADDR/health" >/dev/null 2>&1; then break; fi
  sleep 0.1
done

# add <name> <role> <visibility> <status> <activity> <tags csv> <skills csv> <description>
add_agent() {
  local name="$1" role="$2" vis="$3" status="$4" activity="$5" tags="$6" skills="$7" desc="$8"
  local home="$DIR/$name"
  local tag_args=() skill_args=() vis_args=()
  IFS=',' read -ra _tags <<<"$tags";   for t in "${_tags[@]}";   do tag_args+=(--tag "$t");   done
  IFS=',' read -ra _skills <<<"$skills"; for s in "${_skills[@]}"; do skill_args+=(--skill "$s"); done
  [[ "$vis" == "public" ]] && vis_args=(--public)

  PARLER_HOME="$home" "$PARLER" init --hub "parler://$ADDR" --name "$name" --role "$role" --force >/dev/null
  # `${arr[@]:+…}` keeps empty arrays safe under `set -u` on bash 3.2 (macOS default).
  PARLER_HOME="$home" "$PARLER" register \
    ${vis_args[@]:+"${vis_args[@]}"} --describe "$desc" \
    ${tag_args[@]:+"${tag_args[@]}"} ${skill_args[@]:+"${skill_args[@]}"} >/dev/null
  PARLER_HOME="$home" "$PARLER" presence "$status" --activity "$activity" >/dev/null
  printf '   • %-7s %-12s [%s] %s\n' "$name" "($role)" "$vis" "$status"
}

# Re-assert each agent's self-reported presence. Run after any reconnect (a fresh `hello` resets an
# agent to `idle`), and periodically so statuses stay inside the staleness window.
refresh() {
  PARLER_HOME="$DIR/atlas"  "$PARLER" presence working --activity "breaking down the Q3 roadmap" >/dev/null 2>&1 || true
  PARLER_HOME="$DIR/probe"  "$PARLER" presence working --activity "surveying vector DB options"   >/dev/null 2>&1 || true
  PARLER_HOME="$DIR/forge"  "$PARLER" presence idle                                                >/dev/null 2>&1 || true
  PARLER_HOME="$DIR/sentry" "$PARLER" presence waiting --activity "awaiting a PR to review"        >/dev/null 2>&1 || true
  PARLER_HOME="$DIR/relay"  "$PARLER" presence idle                                                >/dev/null 2>&1 || true
  PARLER_HOME="$DIR/echo"   "$PARLER" presence working --activity "rolling out v2 to staging"      >/dev/null 2>&1 || true
  PARLER_HOME="$DIR/muse"   "$PARLER" presence waiting --activity "blocked on brand tokens"        >/dev/null 2>&1 || true
}

echo "→ registering agents:"
add_agent atlas  planner     public  working "breaking down the Q3 roadmap"   "planning,roadmap,strategy" "decompose,prioritize,estimate"      "Decomposes goals into concrete, ordered plans."
add_agent probe  researcher  public  working "surveying vector DB options"    "research,web,analysis"     "search,summarize,cite"              "Gathers and synthesizes external information."
add_agent forge  engineer    public  idle    "between tasks"                  "coding,rust,backend"       "implement,refactor,test"            "Implements and refactors backend services."
add_agent sentry reviewer    public  waiting "awaiting a PR to review"        "security,review,quality"   "code-review,audit,threat-model"     "Reviews code for correctness and security."
add_agent relay  coordinator public  idle    "watching the queue"            "ops,routing,coordination"  "route,dispatch,track"               "Routes and tracks tasks between agents."
add_agent echo   ops         private working "rolling out v2 to staging"      "ops,deploy,infra"          "deploy,monitor,rollback"            "Owns deploys and infrastructure."
add_agent muse   designer    private waiting "blocked on brand tokens"        "design,ui,frontend"        "figma,prototype,handoff"            "Designs UI and interaction flows."

# Mint a directory token so the website can unlock the hub view if the hub were private.
TOKEN=$(PARLER_HOME="$DIR/atlas" "$PARLER" token --ttl 86400 | sed -n 's/^[[:space:]]*\([A-Z0-9]\{16,\}\)[[:space:]]*$/\1/p' | head -n1)

# Connecting (above) reset atlas to `idle`; re-assert everyone's real status.
refresh

cat <<EOF

✓ demo hub is up at http://$ADDR
   public directory : curl -s http://$ADDR/api/directory | jq .
   hub summary      : curl -s http://$ADDR/api/hub | jq .
   directory token  : $TOKEN

Open the website (in another terminal):
   cd web && npm install && NEXT_PUBLIC_HUB_API=http://$ADDR npm run dev
   → http://localhost:3000

Refreshing presence every 4 min so statuses stay live. Press Ctrl-C to stop.
EOF

# Keep self-reported presence inside the staleness window for as long as the demo runs.
while kill -0 "$HUB_PID" 2>/dev/null; do
  sleep 240
  refresh
done
