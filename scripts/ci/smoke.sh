#!/usr/bin/env bash
#
# scripts/ci/smoke.sh — black-box smoke test of a *running* hub's HTTP surface. The same script
# proves a freshly-built binary boots (CI, via --boot) AND that a live deploy is healthy (CD, against
# the public URL). If this fails after a deploy, the release is bad — roll back.
#
#   scripts/ci/smoke.sh                         # probe http://127.0.0.1:7070
#   scripts/ci/smoke.sh https://parler-hub.fly.dev
#   scripts/ci/smoke.sh --boot                  # build+boot a local hub, smoke it, tear it down
#
# Asserts the contract the website and CLI depend on: /health, /api/hub (JSON), /api/directory
# (JSON array), and the landing page. Mirrors crates/parler-hub/tests/smoke.rs (in-process version).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=lib.sh
source ./lib.sh
cd "$(ci::repo_root)"

BOOT=0
[ "${1:-}" = "--boot" ] && { BOOT=1; shift; }
PORT="${SMOKE_PORT:-7099}"
URL="${1:-http://127.0.0.1:${PORT}}"
URL="${URL%/}"   # drop any trailing slash
HUB_PID=""

# shellcheck disable=SC2329  # invoked indirectly via the trap below
cleanup() { [ -n "$HUB_PID" ] && kill "$HUB_PID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

if [ "$BOOT" -eq 1 ]; then
  bin="target/release/parler-hub"
  [ -x "$bin" ] || bin="target/debug/parler-hub"
  if [ ! -x "$bin" ]; then
    ci::run "build hub" cargo build -p parler-hub
    bin="target/debug/parler-hub"
  fi
  ci::log "booting $bin on 127.0.0.1:${PORT} (in-memory)"
  "$bin" --addr "127.0.0.1:${PORT}" --name "Smoke Hub" >/tmp/parler-smoke-hub.log 2>&1 &
  HUB_PID=$!
fi

# --- checks ---------------------------------------------------------------------------------------
fail=0
get() { curl -fsS -m 10 "$@"; }   # fail on HTTP errors, silent, 10s ceiling

# /health — retried, because a fresh boot or a rolling deploy needs a moment to accept connections.
ci::group "GET ${URL}/health"
health=""
for ((i = 1; i <= 30; i++)); do
  if health="$(get "${URL}/health" 2>/dev/null)"; then break; fi
  ci::log "waiting for hub… ($i/30)"; sleep 1
done
ci::endgroup
if [ "$health" = "ok" ]; then ci::ok "/health → ok"; else ci::err "/health did not return 'ok' (got: '${health:-<unreachable>}')"; fail=1; fi

assert_contains() { # <label> <haystack> <needle>
  case "$2" in *"$3"*) ci::ok "$1" ;; *) ci::err "$1 — missing '$3' in: ${2:0:160}"; fail=1 ;; esac
}

if body="$(get "${URL}/api/hub")"; then
  assert_contains "/api/hub has protocolVersion" "$body" 'protocolVersion'
  assert_contains "/api/hub has name"            "$body" 'name'
else ci::err "/api/hub unreachable"; fail=1; fi

if body="$(get "${URL}/api/directory")"; then
  case "$body" in \[*) ci::ok "/api/directory → JSON array" ;; *) ci::err "/api/directory not a JSON array: ${body:0:120}"; fail=1 ;; esac
else ci::err "/api/directory unreachable"; fail=1; fi

if get -o /dev/null "${URL}/"; then ci::ok "/ landing page → 200"; else ci::err "/ landing page unreachable"; fail=1; fi

if [ "$fail" -eq 0 ]; then ci::ok "smoke passed against ${URL}"; else ci::err "smoke FAILED against ${URL}"; fi
exit "$fail"
