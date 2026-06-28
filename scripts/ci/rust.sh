#!/usr/bin/env bash
#
# scripts/ci/rust.sh — the Rust gate: build, clippy (deny warnings), test, and doc.
#
# Identical to what CI runs, so `make ci` locally fails for the same reasons the cloud would. Pass a
# subset of stages as args to run just those, e.g. `scripts/ci/rust.sh clippy test`.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=lib.sh
source ./lib.sh
cd "$(ci::repo_root)"

# Warnings are hard errors everywhere (build, clippy, AND rustdoc), so a warning can never reach main.
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"
export RUSTDOCFLAGS="${RUSTDOCFLAGS:--D warnings}"

stages=("$@"); [ "${#stages[@]}" -eq 0 ] && stages=(build clippy test doc)

for stage in "${stages[@]}"; do
  case "$stage" in
    build)  ci::run "cargo build"  cargo build  --workspace --all-targets --locked ;;
    clippy) ci::run "cargo clippy" cargo clippy --workspace --all-targets --locked -- -D warnings ;;
    # `--locked` proves Cargo.lock is in sync; the suite includes the in-process-hub e2e + the HTTP
    # smoke contract test. `auth_live` self-skips without a local nats-server, so this is green here.
    test)   ci::run "cargo test"   cargo test   --workspace --locked ;;
    # Broken intra-doc links fail the build — cheap insurance that contributor-facing docs stay valid.
    doc)    ci::run "cargo doc"    cargo doc    --workspace --no-deps --locked ;;
    *) ci::err "unknown rust stage: $stage"; exit 2 ;;
  esac
done
