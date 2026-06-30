# Task: Seamless private hub — "docker compose up, agents talk in no time" — 2026-06-29

**User ask:** make the **private** (self-hosted) hub as easy to stand up as the public hub. "As easy
as running a docker to the database, and the agents can just talk to each other in no time." Goal is
adoption — setup must be one command on the operator side and a copy-paste snippet on the agent side,
**symmetric with the public hub** and **without weakening the security model**.

## Today's asymmetry (the gap)
- Public hub onboarding = `claude mcp add parler -- parler mcp` (URL baked in; MCP self-bootstraps).
- Private hub: `deploy/` is titled "Deploy the **public** hub"; both recipes (Fly, VPS+Caddy) assume
  public + a domain + TLS. "Private" is a one-line footnote ("drop `--public`"). There is **no**
  one-command private recipe, and the runtime image is **distroless (no shell)** so a wrapper script
  can't generate a secret. A LAN-reachable private hub *should* set a join secret (security invariant),
  but inventing + distributing one by hand is friction.

## North-star experience (symmetric, one command each side)
```
# Operator, one box:
docker compose -f deploy/private/docker-compose.yml up -d
#   → boot log prints the exact connect line, with the auto-generated secret:
#     PARLER_HUB=ws://localhost:7070 PARLER_JOIN_SECRET=<gen> claude mcp add parler -- parler mcp
# Each agent:
PARLER_HUB=ws://<host>:7070 PARLER_JOIN_SECRET=<gen> claude mcp add parler -- parler mcp
```

## Design decisions
- **Auto-generated, persistent join secret via a file** (the key enabler). New flag
  `--join-secret-file` / env `PARLER_HUB_JOIN_SECRET_FILE`: read the secret from the file; if absent,
  generate a strong one (reuse the hub's existing token generator), persist it `0600`, reuse on later
  boots. Precedence: explicit `--join-secret` value > file > none. **Binary default is unchanged**
  (no secret unless asked) — this is opt-in and only the private compose sets it. Solves seamless +
  secure-by-default + distroless (no shell needed) in one small, testable helper.
- **Mode-aware landing page + boot banner.** The boot banner (stdout = operator-only) prints the
  ready-to-paste connect line *with the real secret*. The `GET /` page is world-reachable, so it must
  **never print the secret** — for a private hub it shows the snippet with a `PARLER_JOIN_SECRET=<your-
  join-secret>` placeholder and points the operator at the boot log / secret file. Map a `0.0.0.0`
  bind → `localhost` for display so the snippet is copy-pasteable on the common same-machine case.
- **`deploy/private/`** — hub-only compose (no Caddy/domain/TLS), private mode, `7070:7070`, named
  volume, `PARLER_HUB_JOIN_SECRET_FILE=/data/join-secret`. Reuses `deploy/Dockerfile`.
- **Out of scope:** `web/` (private directory viewing already works via tokens); a prebuilt GHCR image
  (truest `docker run`, but touches release/CD + registry namespace — offer as a follow-up).

## Steps
- [x] Hub lib: `secret::resolve_join_secret` + `random_secret` (generate-if-absent, persist `0600`,
      reuse). 6 unit tests.
- [x] `main.rs`: `--join-secret-file` arg; precedence (explicit > file > none); private connect banner.
- [x] `server.rs`: `landing_html(requires_secret)` — private copy + `PARLER_JOIN_SECRET=<placeholder>`
      (structurally can't leak the real secret); `0.0.0.0`/`[::]`→`localhost` in `display_hub_url`. Tests.
- [x] `deploy/private/docker-compose.yml` (hub-only, `command: []` ⇒ private, secret-file) + README.
- [x] Docs: README "Option C"; reframed `deploy/README.md`; AGENTS pointer row.
- [x] `CI_SKIP_WEB=1 make ci` green; booted the real binary twice (generate→persist `0600`→reuse +
      banner with the live secret); compose resolves to `command: []`; public compose still `--public`.

## Review
**Done & verified.** Private-hub onboarding is now symmetric with the public hub: one command on the
box, one copy-paste line per agent — and the hub hands you that exact line.

- **Operator:** `docker compose -f deploy/private/docker-compose.yml up -d --build`. Boots PRIVATE,
  auto-generates + persists a join secret (`/data/join-secret`, `0600`, stable across restarts), and
  prints `PARLER_HUB=… PARLER_JOIN_SECRET=… claude mcp add parler -- parler mcp` in its log.
- **Agent:** paste that line. (`parler mcp` already self-bootstraps; client already reads
  `PARLER_JOIN_SECRET`.) Nothing else.
- **Security held / strengthened:** the world-reachable `GET /` never receives the secret (no param —
  shows a placeholder + "find it in the boot log"); the real secret only hits operator stdout/logs +
  the `0600` file. Private hubs now require a secret by default (was an open "drop --public" footnote).
- **Minimal blast radius:** binary default unchanged (feature is opt-in via `--join-secret-file`); no
  new runtime deps (tempfile is dev-only, already in-workspace); reused the shared Dockerfile + landing
  template. `parler-protocol` untouched, so no cross-crate ripple.

**Verification:** `CI_SKIP_WEB=1 make ci` → "all gates passed"; live binary proof (boot1 generated
`Pd9TW…RTgV`, persisted `0600`; boot2 reused the identical secret); `docker compose config` confirms
private=`command:[]`, public=`command:[--public]`.

**Follow-up SHIPPED — prebuilt GHCR image (`docker run …` in seconds, no compile):**
- `.github/workflows/release-image.yml` — multi-arch (amd64+arm64) build→push to
  `ghcr.io/<owner>/parler-hub` on a `v*` tag or manual dispatch. **No secrets, fork-safe** (pushes to
  the runner's own lowercased namespace via `GITHUB_TOKEN` + `packages: write`); tags via
  `docker/metadata-action` (`latest` / semver / `MAJOR.MINOR` / short-SHA). actionlint + selftest clean.
- **Made the image private-by-default** (the right posture for a published image — a bare `docker run`
  must not open a world-joinable hub). `deploy/Dockerfile` `CMD ["--public"]`→`CMD []`; default name
  →"Parler Hub". Kept the live Fly hub public **safely** via the *existing* `PARLER_HUB_PUBLIC` env
  (added `PARLER_HUB_PUBLIC = "true"` to `fly.toml` — verified `=true`→public, bare→private, `--public`
  arg→public on the real binary). Public compose unaffected (already passes `--public` explicitly).
- `deploy/private/docker-compose.yml` now `image: ghcr.io/tamdogood/parler-hub:latest` + `build:`
  fallback (`--build` from a clone). README/deploy/private + docs/ci-cd.md document the `docker run`
  path. Both composes verified via `docker compose config` (private=`command:[]`+secret, public=`--public`).
- Caveat: Docker daemon was down locally so the *image build* itself runs in CI; the Dockerfile delta
  is only the `CMD`/`ENV` lines (build otherwise identical to the proven Fly build) and the binary's
  mode selection is directly proven. `CI_SKIP_WEB=1 make ci` green.
