# Parler public hub — production image.
#
# Build context is the REPO ROOT (the whole Cargo workspace), so build/deploy from there:
#   docker build -f deploy/Dockerfile -t parler-hub .
#   fly deploy --config deploy/fly.toml      # Fly runs the build from the repo root
#
# Multi-stage: a glibc Rust builder compiles the single `parler-hub` binary (rusqlite's `bundled`
# feature builds SQLite from C, so we want a real toolchain), then we ship it on distroless/cc —
# just glibc + libgcc, no shell, no package manager.

# ---- builder ----
FROM rust:1-bookworm AS builder
WORKDIR /src
COPY . .
# Cargo.lock is committed, so this is a reproducible release build of just the hub.
RUN cargo build --release --locked -p parler-hub

# ---- runtime ----
FROM gcr.io/distroless/cc-debian12
COPY --from=builder /src/target/release/parler-hub /usr/local/bin/parler-hub

# 12-factor config (override per deploy). The DB lives on a mounted volume so the directory survives
# restarts.
ENV PARLER_HUB_ADDR=0.0.0.0:7070 \
    PARLER_HUB_DB=/data/hub.sqlite \
    PARLER_HUB_NAME="Parler Hub"

EXPOSE 7070
ENTRYPOINT ["/usr/local/bin/parler-hub"]
# Private by default — a bare `docker run` of this image must NOT stand up a world-joinable hub. Opt
# into a public, world-readable directory with `--public` (the public compose does) or the equivalent
# env `PARLER_HUB_PUBLIC=true` (the Fly reference instance does, in fly.toml).
CMD []
