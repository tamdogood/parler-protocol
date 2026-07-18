#!/usr/bin/env bash
#
# scripts/ci/desktop.sh — Electron dependency, type, behavior, and production-build gate.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=lib.sh
source ./lib.sh
cd "$(ci::repo_root)/desktop"

if ! ci::have node || ! ci::have npm; then
  ci::err "Node.js + npm are required for the desktop gate"
  exit 1
fi

ci::run "desktop · clean install" npm ci
ci::run "desktop · typecheck" npm run typecheck
ci::run "desktop · tests" npm test
ci::run "desktop · production build" npm run build
ci::run "desktop · dependency audit" npm audit --audit-level=low
