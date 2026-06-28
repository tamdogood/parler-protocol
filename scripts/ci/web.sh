#!/usr/bin/env bash
#
# scripts/ci/web.sh — the website gate. `next build` type-checks every route and compiles the whole
# app, so it fails on any TS/JSX error. The site has no ESLint config, so we don't run `next lint`.
#
# Uses `npm ci` (lockfile-exact, reproducible) in CI; falls back to `npm install` locally when there
# is no node_modules yet, so a fresh contributor checkout just works.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=lib.sh
source ./lib.sh
cd "$(ci::repo_root)/web"

if ! ci::have npm; then
  ci::err "npm not found — install Node 20+ to build the website"
  exit 127
fi

if [ -f package-lock.json ]; then
  ci::run "npm ci" npm ci
else
  ci::warn "no package-lock.json; falling back to 'npm install'"
  ci::run "npm install" npm install
fi
ci::run "next build" npm run build
