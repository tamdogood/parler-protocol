#!/usr/bin/env bash
#
# scripts/canary-scan.sh — hunt the public web for copies of this project.
#
# The kit in docs/provenance.md seeds unique canary tokens (scripts/canary/tokens.txt) into the
# codebase. This script does two honest, harmless things with them:
#
#   1. LOCAL  — asserts every registered token is still present in the working tree. A watermark that
#               got refactored away silently is a watermark that can't prove anything, so a missing
#               token fails the scan (and the scheduled CI job).
#   2. REMOTE — asks GitHub code search for each token and reports any hit that is NOT in an owned
#               repo (default owner: tamdogood). A foreign hit means someone copied the code — the
#               token is a meaningless UUID nobody types by accident. That is your DMCA evidence.
#
# It never touches, probes, or attacks anyone's machine. It only reads a public search index.
#
#   scripts/canary-scan.sh                 # local check + remote search (needs `gh` authed)
#   scripts/canary-scan.sh --local         # local presence check only (no network; used by `make`)
#   CANARY_OWNERS="tamdogood,myorg" scripts/canary-scan.sh   # treat these owners as "ours"
#
# Exit status: 0 = all tokens present locally and no foreign copies found; non-zero otherwise.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/ci"
# shellcheck source=scripts/ci/lib.sh
source ./lib.sh
cd "$(ci::repo_root)"

TOKENS_FILE="scripts/canary/tokens.txt"
# Owners whose repos are "ours" — a token found there is expected, not a leak. Comma-separated.
OWNERS="${CANARY_OWNERS:-tamdogood}"

LOCAL_ONLY=0
[ "${1:-}" = "--local" ] && LOCAL_ONLY=1

[ -f "$TOKENS_FILE" ] || { ci::err "no token registry at $TOKENS_FILE"; exit 2; }

# Load tokens: skip blank lines and '#' comments, trim whitespace.
tokens=()
while IFS= read -r line; do
  line="${line#"${line%%[![:space:]]*}"}"   # ltrim
  case "$line" in ''|'#'*) continue ;; esac
  tokens+=("$line")
done < "$TOKENS_FILE"

[ "${#tokens[@]}" -gt 0 ] || { ci::err "token registry is empty"; exit 2; }
ci::log "loaded ${#tokens[@]} canary token(s) from $TOKENS_FILE"

# --- 1. local presence ----------------------------------------------------------------------------
# Every token must still live somewhere in the tree — but not *only* in the registry itself, or the
# watermark isn't actually seeded in the code it's meant to protect.
missing=0
ci::group "local presence check"
for tok in "${tokens[@]}"; do
  # Count files (excluding the registry) that contain the token. `--untracked` so a freshly-seeded,
  # not-yet-committed watermark still counts — the check works before you commit, not only after.
  hits="$(git grep -l -F --untracked "$tok" -- ':!'"$TOKENS_FILE" 2>/dev/null | wc -l | tr -d ' ')"
  if [ "$hits" -ge 1 ]; then
    ci::ok "seeded: $tok (${hits} file(s))"
  else
    ci::err "NOT seeded outside the registry: $tok"
    missing=1
  fi
done
ci::endgroup
[ "$missing" -eq 0 ] || { ci::err "some tokens are no longer seeded — re-seed them or update $TOKENS_FILE"; exit 1; }
ci::ok "all ${#tokens[@]} tokens present locally"

if [ "$LOCAL_ONLY" -eq 1 ]; then
  ci::ok "local-only mode: skipping remote search"
  exit 0
fi

# --- 2. remote search -----------------------------------------------------------------------------
if ! ci::have gh; then
  ci::warn "gh CLI not found — skipping remote search (local check passed)."
  ci::warn "install it (https://cli.github.com) and run again to hunt for copies."
  exit 0
fi
if ! gh auth status >/dev/null 2>&1; then
  ci::warn "gh is not authenticated — run 'gh auth login' to enable remote search. Local check passed."
  exit 0
fi

# Build a regex of owned owners, e.g. "tamdogood|myorg", for the "is this ours?" test.
owners_re="$(printf '%s' "$OWNERS" | tr ',' '|')"
foreign=0
ci::group "remote search (GitHub code search)"
ci::log "owners treated as ours: ${OWNERS}"
for tok in "${tokens[@]}"; do
  # `gh search code` returns repositories that contain the token. jq prints "owner/name" per hit.
  # A missing token search simply returns nothing (harmless). Network errors are non-fatal per token.
  results="$(gh search code "$tok" --limit 50 --json repository \
      --jq '.[].repository.nameWithOwner' 2>/dev/null | sort -u || true)"
  if [ -z "$results" ]; then
    ci::ok "no public copies of $tok"
    continue
  fi
  while IFS= read -r repo; do
    [ -n "$repo" ] || continue
    owner="${repo%%/*}"
    if printf '%s' "$owner" | grep -qiE "^(${owners_re})$"; then
      ci::log "  (ours) $repo"
    else
      ci::err "FOREIGN COPY: $repo contains $tok"
      foreign=1
    fi
  done <<< "$results"
done
ci::endgroup

if [ "$foreign" -eq 0 ]; then
  ci::ok "no foreign copies found — scan clean"
  exit 0
fi
ci::err "foreign copies detected. Next step: docs/provenance.md → enforcement (DMCA)."
exit 1
