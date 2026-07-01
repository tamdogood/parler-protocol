#!/usr/bin/env bash
# Build the Rust binaries the desktop app ships (parler-hub + parler) and stage them in
# resources/bin/ so electron-builder can bundle them as extraResources.
#
# Usage:
#   scripts/build-binaries.sh                       # host arch (this Mac)
#   scripts/build-binaries.sh aarch64-apple-darwin  # a specific target triple
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$DESKTOP_DIR/.." && pwd)"
BIN_OUT="$DESKTOP_DIR/resources/bin"
mkdir -p "$BIN_OUT"

TARGET="${1:-}"
CARGO_ARGS=(build --release -p parler-bin -p parler-hub)
if [ -n "$TARGET" ]; then
  echo "› ensuring rust target $TARGET is installed"
  rustup target add "$TARGET" >/dev/null 2>&1 || true
  CARGO_ARGS+=(--target "$TARGET")
fi

echo "› cargo ${CARGO_ARGS[*]} (in $REPO_ROOT)"
( cd "$REPO_ROOT" && cargo "${CARGO_ARGS[@]}" )

if [ -n "$TARGET" ]; then
  REL="$REPO_ROOT/target/$TARGET/release"
else
  REL="$REPO_ROOT/target/release"
fi

for bin in parler parler-hub; do
  if [ ! -f "$REL/$bin" ]; then
    echo "error: expected $REL/$bin — did the cargo build succeed?" >&2
    exit 1
  fi
  # On macOS, running a helper binary named 'parler' inside 'Parler.app' causes
  # LaunchServices/OS to case-insensitively resolve it to 'Parler.app' (the Electron GUI itself),
  # triggering a recursive infinite loop of app launches. We rename the helper to 'parler-cli' to avoid this.
  dest_name="$bin"
  if [ "$bin" = "parler" ]; then
    dest_name="parler-cli"
  fi
  cp "$REL/$bin" "$BIN_OUT/$dest_name"
  chmod +x "$BIN_OUT/$dest_name"
done

echo "✓ bundled → $BIN_OUT"
ls -lh "$BIN_OUT"
