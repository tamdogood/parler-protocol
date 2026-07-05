# CI/CD & the test system

Parler Protocol is built to take pull requests from many contributors, so the pipeline has one job: **catch
bugs and regressions before they reach the deployed hub** — and be trustworthy and reproducible
enough that contributors actually rely on it.

The guiding idea: **GitHub Actions YAML is not testable, so no logic lives there.** Every gate is a
small shell script in [`scripts/ci/`](../scripts/ci/); the workflows are thin wrappers that call
them. A contributor runs `make ci` and gets the *exact* gates the cloud runs. And because the gates
are ordinary scripts, the pipeline is itself unit-tested — see [Testing the test system](#testing-the-test-system).

```
┌──────────────┐     calls      ┌──────────────────────┐
│ GitHub       │ ─────────────► │ scripts/ci/*.sh      │ ◄──── `make ci` (same scripts, locally)
│ workflows    │                │  rust · web · audit  │
│ (thin YAML)  │                │  smoke · selftest    │
└──────────────┘                └──────────────────────┘
```

## The gates

| Gate | Script | What it proves |
| --- | --- | --- |
| **Pipeline** | `selftest.sh` + shellcheck + actionlint | the test system & workflows are themselves valid |
| **Rust** | `rust.sh` | `cargo build` · `clippy -D warnings` · `cargo test` · `cargo doc -D warnings` |
| **Smoke** | `smoke.sh --boot` | the *real* compiled hub binary boots and serves its HTTP contract |
| **Web** | `web.sh` | `npm ci` + `next build` type-checks and compiles every route |
| **Supply chain** | `audit.sh` (cargo-deny) | no known vulnerabilities, no unexpected dependency sources |
| **Dockerfile** | hadolint | the deploy image definition isn't broken |

The CI workflow ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)) runs these as parallel
jobs, then a single **`CI passed`** job aggregates them. That aggregate is the one check to require
in branch protection — adding a new gate never means reconfiguring branch protection.

Notable choices:

- **`cargo doc` is a gate** (`RUSTDOCFLAGS=-D warnings`). Broken intra-doc links fail CI — this
  already caught a stale link in `parler-auth` on day one.
- **No `cargo fmt` gate.** This repo is deliberately hand-formatted; a repo-wide `cargo fmt` would
  reformat everything. Style is reviewed, not enforced by a tool.
- **Concurrency + timeouts + least-privilege `permissions: contents: read`** on every workflow.

## Local = cloud

```bash
make ci                 # the whole pipeline (what GitHub runs)
make selftest           # just the meta-tests (fast)
make smoke              # boot the real binary and probe it
make audit              # cargo-deny (auto-installs it if missing)
make coverage           # HTML coverage (needs cargo-llvm-cov)
CI_SKIP_WEB=1 make ci   # skip the slow website build while iterating on Rust
```

`all.sh` runs **every** gate even when one fails, then reports the full list — so a contributor fixes
everything in one pass instead of one-error-at-a-time.

## Testing the test system

The pipeline is code, so it's tested like code. [`selftest.sh`](../scripts/ci/selftest.sh):

- syntax-checks (`bash -n`) every script and asserts the executable bits,
- **unit-tests the `lib.sh` step runner** on both its success and failure paths (so a gate can never
  silently "pass" by swallowing a non-zero exit),
- sanity-checks each workflow (non-empty, no tabs, has `jobs:`) and parses `deny.toml`,
- runs shellcheck when present.

The HTTP contract has two coupled tests that must agree: the in-process
[`crates/parler-hub/tests/smoke.rs`](../crates/parler-hub/tests/smoke.rs) (runs in `cargo test`) and
the black-box [`smoke.sh`](../scripts/ci/smoke.sh) (runs against a booted binary in CI and against
the live URL after a deploy).

## Continuous delivery

[`.github/workflows/deploy.yml`](../.github/workflows/deploy.yml) ships the hub to Fly.io on pushes
to `main` that touch shippable paths (and via manual `workflow_dispatch`). It is **resilient and
community-safe**:

1. **Guard** — if there's no `FLY_API_TOKEN` secret (every fork, every contributor), the job no-ops.
   Only the maintainer's repo deploys.
2. **Capture** the currently-deployed image (for rollback).
3. **Deploy** with `flyctl deploy`, which waits on the `fly.toml` health checks and auto-reverts a
   release that never becomes healthy — the first safety net.
4. **Live smoke** — `smoke.sh` runs the same HTTP contract against the public URL. A release that
   builds and passes Fly's health check but breaks the API contract is still caught here.
5. **Rollback** — on any failure, pin the previous image back; if none was captured, fail loudly with
   the manual rollback command.

A separate daily [`audit.yml`](../.github/workflows/audit.yml) re-runs cargo-deny so a CVE published
against an already-merged dependency is caught even with no PR traffic.

### Prebuilt hub image (GHCR)

[`.github/workflows/release-image.yml`](../.github/workflows/release-image.yml) publishes the hub as a
prebuilt multi-arch container image so self-hosting a private hub is `docker run
ghcr.io/<owner>/parler-hub` — seconds, not a from-source compile (see
[`deploy/private/`](../deploy/private/)). It runs on a **`vX.Y.Z` tag** (a released image) or **manual
`workflow_dispatch`** (publishes `:latest`, e.g. for the first image) — *not* on every push to `main`,
since the multi-arch (amd64 + arm64, arm64 via QEMU) build is slow. Notes:

- **No secrets, fork-safe.** It pushes to the runner's own namespace (`ghcr.io/${{
  github.repository_owner }}`, lowercased) with the automatic `GITHUB_TOKEN` + `permissions: packages:
  write` — so a fork publishes to *its* packages with zero setup. Tags come from
  `docker/metadata-action` (`latest`, the semver, `MAJOR.MINOR`, and a short-SHA).
- **The image is private by default** (`deploy/Dockerfile` ships `CMD []`): a bare `docker run` of it
  never opens a world-joinable hub. The public reference instance opts in via `PARLER_HUB_PUBLIC=true`
  in `fly.toml`; the public compose passes the `--public` flag.

### What the maintainer configures once

In the GitHub repo settings (only needed for real deploys; everything else works without it):

| Kind | Name | Purpose |
| --- | --- | --- |
| Secret | `FLY_API_TOKEN` | lets `deploy.yml` ship to Fly (absent ⇒ deploy is skipped) |
| Variable | `FLY_APP` | Fly app name (default `parler-hub`) |
| Variable | `PARLER_HUB_URL` | live URL for the post-deploy smoke (default `https://parler-hub.fly.dev`) |

Recommended branch protection on `main`: require the **`CI passed`** status check and a Code Owner
review; the deploy job uses a `production` GitHub Environment, which you can additionally gate with a
required reviewer for a manual approval step.

## Extending the pipeline

1. Add a script under `scripts/ci/` (source `lib.sh`, use `ci::run` so it's timed and reported).
2. Wire it into `scripts/ci/all.sh` and add a job to `ci.yml` that calls it.
3. Add its filename to the `scripts` list in `selftest.sh` so the meta-tests cover it.
4. `make selftest` → green. `make ci` → green. Open the PR.
