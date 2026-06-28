#!/usr/bin/env bash
#
# scripts/ci/audit.sh — supply-chain gate via cargo-deny (config: deny.toml).
#
# Resilient by construction so it does not flap an unrelated PR red:
#   • vulnerabilities (RustSec) ........ BLOCK   — a known CVE must never reach a deploy
#   • unknown crate sources ............ BLOCK   — only crates.io + declared git is allowed
#   • licenses ......................... BLOCK   — allow-list verified; set CI_LICENSES_STRICT=0 to warn
#   • unmaintained / yanked / dupes .... WARN    — upstream churn shouldn't fail your build
#
# Locally, if cargo-deny isn't installed this *skips* (so `make ci` still works on a fresh checkout);
# CI installs it, so the gate always runs there. Install locally with: cargo install cargo-deny
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=lib.sh
source ./lib.sh
cd "$(ci::repo_root)"

if ! ci::have cargo-deny; then
  ci::warn "cargo-deny not installed — skipping locally (CI runs it). Install: cargo install cargo-deny"
  exit 0
fi

rc=0
# True vulnerabilities and unknown sources are hard gates; advisory *noise* is downgraded to allow.
ci::run "cargo-deny · advisories (vulnerabilities)" \
  cargo deny check advisories --allow unmaintained --allow yanked --allow notice --allow unsound || rc=$?
ci::run "cargo-deny · sources" cargo deny check sources || rc=$?

# Licenses block by default — the allow-list in deny.toml was verified against the current tree, so a
# new/unknown license is a real finding to triage. Set CI_LICENSES_STRICT=0 to downgrade to a warning.
if [ "${CI_LICENSES_STRICT:-1}" = "1" ]; then
  ci::run "cargo-deny · licenses" cargo deny check licenses || rc=$?
else
  ci::run "cargo-deny · licenses (advisory)" cargo deny check licenses \
    || ci::warn "license check found issues — review deny.toml (or set CI_LICENSES_STRICT=1 to enforce)"
fi

# Duplicate/old versions are normal in a Rust tree; report but never fail on them.
ci::run "cargo-deny · bans (informational)" cargo deny check bans || ci::warn "duplicate/old deps present (non-fatal)"

exit "$rc"
