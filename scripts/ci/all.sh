#!/usr/bin/env bash
#
# scripts/ci/all.sh — run the whole local pipeline, the same gates CI runs. This is what `make ci`
# calls. It does NOT stop at the first failure: every gate runs so a contributor sees the complete
# list of problems in one pass, then it exits non-zero if any failed.
#
#   scripts/ci/all.sh                # selftest + rust + audit
set -uo pipefail   # NB: no -e — we want to run every gate and aggregate the result
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"

declare -a failed=()
gate() { # <label> <script> [args...]
  local label="$1"; shift
  if "$@"; then ci::ok "gate: $label"; else failed+=("$label"); ci::err "gate: $label"; fi
}

ci::log "${_CI_BOLD}Parler Protocol local CI${_CI_RST} — mirrors .github/workflows/ci.yml"

gate "selftest" "$here/selftest.sh"
gate "rust"     "$here/rust.sh"
gate "audit"    "$here/audit.sh"

printf '\n'
if [ "${#failed[@]}" -eq 0 ]; then
  ci::ok "${_CI_BOLD}all gates passed${_CI_RST} — this is what CI will see"
  exit 0
fi
ci::err "${_CI_BOLD}${#failed[@]} gate(s) failed:${_CI_RST} ${failed[*]}"
exit 1
