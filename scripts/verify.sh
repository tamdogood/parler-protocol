#!/usr/bin/env bash
#
# verify.sh — the single, reliable feedback signal for the autonomous loop.
#
# Loop engineering lives or dies on one thing: a fast, deterministic gate the agent can run after
# every change to learn "am I done / did I break something?". This script IS that gate. It mirrors
# `.github/workflows/ci.yml` exactly so "green locally" == "green in CI" — no surprises after a push.
#
# Contract: exits 0 and prints `VERIFY: PASS` only when every gate passes; otherwise exits non-zero
# and prints `VERIFY: FAIL (<stage>)`. The loop greps for those lines.
#
# IMPORTANT: never run `cargo fmt` here. This repo is hand-formatted; a repo-wide format would
# rewrite every file. (See tasks/lessons.md.)
#
# Usage:
#   scripts/verify.sh              # full gate: Rust (build · clippy · test)
#   scripts/verify.sh --rust-only  # accepted for backward compatibility; identical to the default

set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

# Match the Makefile: find cargo even when it isn't on PATH (common under launchd/cron).
CARGO="$(command -v cargo 2>/dev/null || echo "$HOME/.cargo/bin/cargo")"

# CI denies warnings; do the same so clippy/rustc warnings fail the gate locally too.
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"
export CARGO_TERM_COLOR=always

# --rust-only is still accepted (the loop passes it) but no longer changes anything: the website
# moved to its own repo, so this gate is Rust-only in both forms.
fail() { echo "VERIFY: FAIL ($1)"; exit 1; }

echo "→ [1/3] cargo build --workspace --all-targets --locked"
"$CARGO" build --workspace --all-targets --locked || fail "rust-build"

echo "→ [2/3] cargo clippy --workspace --all-targets --locked -- -D warnings"
"$CARGO" clippy --workspace --all-targets --locked -- -D warnings || fail "clippy"

echo "→ [3/3] cargo test --workspace --locked"
"$CARGO" test --workspace --locked || fail "rust-test"

echo "VERIFY: PASS"
