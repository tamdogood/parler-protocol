#!/usr/bin/env bash
#
# scripts/ci/selftest.sh — test the test system.
#
# The pipeline's logic lives in shell, so the shell is unit-tested like any other code: every CI
# script is syntax-checked and executable, the lib.sh step-runner is exercised on both its success
# and failure paths, and the things the scripts *configure* (workflows, deny.toml) are sanity-checked.
# Dependency-free: shellcheck / TOML parsing are used only when present (CI provides them).
#
# Runs fast (no cargo, no network), so it goes first in `make ci` and in the CI "pipeline" job.
set -uo pipefail   # no -e: assertions must all run so the report is complete
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"
root="$(ci::repo_root)"

pass=0; fail=0
ok()   { pass=$((pass + 1)); ci::ok "$1"; }
bad()  { fail=$((fail + 1)); ci::err "$1"; }
# assert a command SUCCEEDS / FAILS, quietly
want_ok()   { local d="$1"; shift; if "$@" >/dev/null 2>&1; then ok "$d"; else bad "$d"; fi; }
want_fail() { local d="$1"; shift; if "$@" >/dev/null 2>&1; then bad "$d (expected non-zero)"; else ok "$d"; fi; }

scripts=(lib.sh rust.sh audit.sh smoke.sh all.sh selftest.sh)

ci::group "every CI script is present, syntactically valid, and (except lib.sh) executable"
for s in "${scripts[@]}"; do
  want_ok   "exists: $s"      test -f "$here/$s"
  want_ok   "valid bash: $s"  bash -n "$here/$s"
  [ "$s" = "lib.sh" ] && continue   # lib.sh is sourced, not run, so no +x required
  want_ok   "executable: $s"  test -x "$here/$s"
done
ci::endgroup

ci::group "lib.sh step-runner behaves (the choke point every gate flows through)"
want_ok   "ci::have finds sh"            ci::have sh
want_fail "ci::have rejects bogus cmd"   ci::have __parler_no_such_cmd__
want_ok   "ci::run propagates success"   ci::run "selftest/true"  true
want_fail "ci::run propagates failure"   ci::run "selftest/false" false
want_ok   "ci::repo_root is the repo"    test -f "$root/Cargo.toml"
ci::endgroup

ci::group "pipeline configuration is sane"
# Workflows: dependency-free sanity (actionlint does the rigorous parse in CI). Catches the classic
# YAML footguns — empty file, a literal tab, or a missing top-level 'jobs:'.
for wf in "$root"/.github/workflows/*.yml; do
  [ -e "$wf" ] || continue
  name="$(basename "$wf")"
  want_ok "workflow non-empty: $name" test -s "$wf"
  if grep -qP '\t' "$wf" 2>/dev/null || grep -q "$(printf '\t')" "$wf"; then bad "workflow has no tabs: $name"; else ok "workflow has no tabs: $name"; fi
  want_ok "workflow has jobs: $name" grep -q '^jobs:' "$wf"
done
# deny.toml parses as TOML, when a parser is available (python 3.11+ stdlib).
if [ -f "$root/deny.toml" ]; then
  if ci::have python3 && python3 -c 'import tomllib' 2>/dev/null; then
    want_ok "deny.toml is valid TOML" python3 -c 'import tomllib,sys; tomllib.load(open(sys.argv[1],"rb"))' "$root/deny.toml"
  else
    ci::warn "no python tomllib — skipping deny.toml parse (CI's cargo-deny validates it)"
  fi
fi
ci::endgroup

# Lint the scripts with ShellCheck when it's installed locally; CI always runs it as its own step.
if ci::have shellcheck; then
  ci::group "shellcheck"
  for s in "${scripts[@]}"; do want_ok "shellcheck: $s" shellcheck -x --severity=warning "$here/$s"; done
  ci::endgroup
else
  ci::warn "shellcheck not installed — skipping locally (CI runs it). Install: brew install shellcheck"
fi

printf '\n'
if [ "$fail" -eq 0 ]; then ci::ok "selftest: ${pass} checks passed"; exit 0; fi
ci::err "selftest: ${fail} failed, ${pass} passed"; exit 1
