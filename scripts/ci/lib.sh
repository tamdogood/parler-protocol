# shellcheck shell=bash
#
# scripts/ci/lib.sh — shared helpers for the Parler CI scripts.
#
# This file is *sourced*, never executed, so it does not set shell options of its own (the entry
# script owns `set -euo pipefail`). Everything here is small and side-effect-free enough to be
# unit-tested by scripts/ci/selftest.sh — that is the point: the pipeline's logic is testable.

# --- output -----------------------------------------------------------------------------------------
# Colours only when attached to a terminal; GitHub strips them anyway. `printf`, never `echo -e`.
if [ -t 1 ]; then
  _CI_BOLD=$'\033[1m'; _CI_DIM=$'\033[2m'; _CI_RED=$'\033[31m'; _CI_GRN=$'\033[32m'
  _CI_YEL=$'\033[33m'; _CI_CYN=$'\033[36m'; _CI_RST=$'\033[0m'
else
  _CI_BOLD=''; _CI_DIM=''; _CI_RED=''; _CI_GRN=''; _CI_YEL=''; _CI_CYN=''; _CI_RST=''
fi

ci::log()  { printf '%s\n' "${_CI_CYN}›${_CI_RST} $*"; }
ci::ok()   { printf '%s\n' "${_CI_GRN}✓${_CI_RST} $*"; }
ci::warn() { printf '%s\n' "${_CI_YEL}!${_CI_RST} $*" >&2; }
ci::err()  { printf '%s\n' "${_CI_RED}✗${_CI_RST} $*" >&2; }

# Foldable log groups: real groups in GitHub Actions, a plain header locally.
ci::group() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then printf '::group::%s\n' "$*"; else
    printf '\n%s\n' "${_CI_BOLD}── $* ──${_CI_RST}"; fi
}
ci::endgroup() { [ -n "${GITHUB_ACTIONS:-}" ] && printf '::endgroup::\n' || true; }

# --- environment ------------------------------------------------------------------------------------
# Repo root from this file's location — works no matter where the script is invoked from.
ci::repo_root() { ( cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd ); }

# True if a command is on PATH. Quiet; callers decide what to print.
ci::have() { command -v "$1" >/dev/null 2>&1; }

# --- step runner ------------------------------------------------------------------------------------
# `ci::run "<label>" <cmd> [args...]` — announce, time, run, and report a step. Returns the command's
# exit status so the caller (and `set -e`) can react. This is the single choke point every gate runs
# through, which is why selftest.sh exercises it for both the success and failure paths.
ci::run() {
  local label="$1"; shift
  local start end rc
  ci::group "$label"
  ci::log "${_CI_DIM}\$ $*${_CI_RST}"
  start=$(date +%s)
  "$@" && rc=0 || rc=$?
  end=$(date +%s)
  ci::endgroup
  if [ "$rc" -eq 0 ]; then
    ci::ok "$label ${_CI_DIM}($((end - start))s)${_CI_RST}"
  else
    ci::err "$label failed (exit $rc, $((end - start))s)"
  fi
  return "$rc"
}
