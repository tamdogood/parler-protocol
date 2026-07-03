#!/bin/sh
# Parler installer — one command, no Rust toolchain.
#
#   curl -fsSL https://raw.githubusercontent.com/tamdogood/parler-ai/main/scripts/install.sh | sh
#
# Downloads the prebuilt `parler` binary for this OS/arch from the latest GitHub Release, verifies its
# SHA-256, and drops it on your PATH. Then: `parler connect` wires every AI agent on the machine.
#
# Overrides (env): PARLER_VERSION (default: latest), PARLER_INSTALL_DIR (default: ~/.local/bin),
# PARLER_REPO (default: tamdogood/parler-ai).
set -eu

REPO="${PARLER_REPO:-tamdogood/parler-ai}"
VERSION="${PARLER_VERSION:-latest}"
INSTALL_DIR="${PARLER_INSTALL_DIR:-$HOME/.local/bin}"

info() { printf '  %s\n' "$*"; }
err() { printf 'error: %s\n' "$*" >&2; exit 1; }

# --- detect target triple ---------------------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin)
    case "$arch" in
      arm64|aarch64) target="aarch64-apple-darwin" ;;
      x86_64) target="x86_64-apple-darwin" ;;
      *) err "unsupported macOS arch: $arch" ;;
    esac ;;
  Linux)
    case "$arch" in
      x86_64|amd64) target="x86_64-unknown-linux-gnu" ;;
      *) err "no prebuilt Linux binary for $arch yet — build from source: cargo install --git https://github.com/$REPO parler-bin" ;;
    esac ;;
  *) err "unsupported OS: $os — build from source: cargo install --git https://github.com/$REPO parler-bin" ;;
esac

# --- resolve download URL ---------------------------------------------------------------------
tarball="parler-${target}.tar.gz"
if [ "$VERSION" = "latest" ]; then
  base="https://github.com/$REPO/releases/latest/download"
else
  base="https://github.com/$REPO/releases/download/$VERSION"
fi

# --- fetch helper (curl or wget) --------------------------------------------------------------
fetch() { # fetch <url> <out>
  if command -v curl >/dev/null 2>&1; then
    curl -fSL --proto '=https' --tlsv1.2 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$2" "$1"
  else
    err "need curl or wget to download"
  fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Parler · installing the $target binary"
info "from $base/$tarball"
fetch "$base/$tarball" "$tmp/$tarball"

# --- verify checksum if the release ships one -------------------------------------------------
if fetch "$base/$tarball.sha256" "$tmp/$tarball.sha256" 2>/dev/null; then
  want="$(awk '{print $1}' "$tmp/$tarball.sha256")"
  if command -v sha256sum >/dev/null 2>&1; then
    got="$(sha256sum "$tmp/$tarball" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    got="$(shasum -a 256 "$tmp/$tarball" | awk '{print $1}')"
  else
    got=""
  fi
  if [ -n "$got" ] && [ "$got" != "$want" ]; then
    err "checksum mismatch — refusing to install (wanted $want, got $got)"
  fi
  [ -n "$got" ] && info "checksum ok"
fi

# --- install ----------------------------------------------------------------------------------
tar -xzf "$tmp/$tarball" -C "$tmp"
[ -f "$tmp/parler" ] || err "archive did not contain a 'parler' binary"
mkdir -p "$INSTALL_DIR"
install -m 0755 "$tmp/parler" "$INSTALL_DIR/parler" 2>/dev/null || {
  cp "$tmp/parler" "$INSTALL_DIR/parler" && chmod 0755 "$INSTALL_DIR/parler"
}

# --- verify it actually runs (catches a wrong-arch or truncated download now, not later) -------
if ! "$INSTALL_DIR/parler" --version >/dev/null 2>&1; then
  err "installed to $INSTALL_DIR/parler but it won't run — the download may be corrupt or built for a different architecture"
fi

echo
echo "✓ installed parler → $INSTALL_DIR/parler"

# --- make sure the very next step ('parler connect') can be found -----------------------------
case ":$PATH:" in
  *":$INSTALL_DIR:"*)
    echo
    echo "Next: wire every agent on this machine —"
    echo "  parler connect"
    ;;
  *)
    # Not on PATH: hand the user an exact fix for their shell plus a full-path fallback, so they
    # never hit a bare "command not found" right after a successful install.
    rc="$HOME/.profile"
    case "${SHELL:-}" in
      *zsh) rc="$HOME/.zshrc" ;;
      *bash)
        if [ "$os" = Darwin ]; then rc="$HOME/.bash_profile"; else rc="$HOME/.bashrc"; fi ;;
    esac
    echo
    echo "⚠  $INSTALL_DIR isn't on your PATH yet, so 'parler' won't be found."
    echo "   Add it (then open a new terminal):"
    echo "     echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> \"$rc\""
    echo
    echo "   …or run it now by full path:"
    echo "     $INSTALL_DIR/parler connect"
    ;;
esac
