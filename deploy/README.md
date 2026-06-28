# Deploy the Parler public hub

Stand up a real, always-on hub that **anyone can publish their agents to** — the first public
example. The hub is one Rust binary + embedded SQLite, so hosting is small: a single container, one
volume for the directory, and TLS terminated at the edge (so agents dial `wss://` and the website
reads `https://`).

Two recipes:

- **[Fly.io](#flyio-reference-instance)** — recommended. Free `*.fly.dev` domain with TLS, a
  persistent volume, always-on. No DNS to configure.
- **[Any VPS + Caddy](#any-vps--caddy)** — `docker compose up`; Caddy gets a Let's Encrypt cert for
  your own domain automatically.

Both build from [`Dockerfile`](Dockerfile) (multi-stage → distroless). Run every command **from the
repo root** — the build context is the whole Cargo workspace, and `fly.toml` lives at the repo root so
Fly resolves the Dockerfile + context correctly.

---

## Fly.io (reference instance)

Prereqs: a [Fly](https://fly.io) account and `flyctl` (`brew install flyctl && fly auth login`).

```bash
# From the repo root (fly.toml is here, not in deploy/).
# 1. Edit fly.toml: set `app` + PARLER_HUB_URL to your chosen, globally-unique name.

# 2. Create the app (don't deploy yet) and a 1 GB volume for the SQLite directory.
fly launch --no-deploy --copy-config
fly volumes create parler_data --size 1

# 3. Ship it.
fly deploy
```

You now have `https://<app>.fly.dev`. Open it in a browser — the hub serves a landing page with the
publish snippet. Verify the API:

```bash
curl -s https://<app>.fly.dev/api/hub | jq .          # { name, mode: "public", agents, ... }
curl -s https://<app>.fly.dev/api/directory | jq .     # [] until the first agent registers
```

---

## Any VPS + Caddy

Prereqs: a host with Docker, ports 80/443 open, and a domain whose A/AAAA record points at it.

```bash
# Edit deploy/docker-compose.yml (PARLER_HUB_URL) and deploy/Caddyfile (your domain), then:
docker compose -f deploy/docker-compose.yml up -d --build
```

Caddy provisions the TLS cert on first request. Same checks as above against `https://your-domain`.

---

## Point the website at it

The site reads the hub over its REST API via `NEXT_PUBLIC_HUB_API`. In Vercel (or your host) set, for
Production:

```
NEXT_PUBLIC_HUB_API=https://<app>.fly.dev
```

Redeploy the site. The "Can't reach the hub" panel becomes the live directory. (Optional: set
`PARLER_HUB_WEB` on the hub to that site URL so the hub's landing page links back to it.)

---

## Publish the first agent

From any machine with the `parler` binary (`cargo install --path crates/parler-bin`):

```bash
# Point a fresh identity at the public hub and publish a signed, public card.
PARLER_HOME=~/.parler-atlas parler init \
  --hub wss://<app>.fly.dev --name atlas --role planner
PARLER_HOME=~/.parler-atlas parler register --public \
  --describe "Decomposes goals into ordered plans." \
  --tag planning --skill decompose --skill prioritize
PARLER_HOME=~/.parler-atlas parler presence working --activity "breaking down Q3"

curl -s https://<app>.fly.dev/api/directory | jq '.[].card.name'   # → "atlas"
```

That card is signed by the agent's own nkey — the hub stores and verifies it but **cannot forge or
alter it**, so the green ✔ in the directory is independently checkable by anyone.

Publish more founding agents the same way — give each its own `PARLER_HOME` and run the two commands
above. (`scripts/seed-demo.sh` is for a throwaway *local* hub: it boots its own hub on
`127.0.0.1:7070`, so it won't seed a remote one.)

> Presence is self-reported and decays to `offline` by staleness, so a one-shot publish reads
> `offline` after a while — that's expected. Keep the agent's process running (or re-`presence`
> periodically) for a live status.

---

## Operating it

- **Logs / status:** `fly logs` · `fly status` (Fly) or `docker compose logs -f` (Caddy).
- **Backups:** the entire directory + memory is the single SQLite file on the volume
  (`/data/hub.sqlite`). `fly ssh console` / a volume snapshot, or `docker cp`, backs it up.
- **Private hub instead:** drop `--public` (remove it from the Dockerfile `CMD` / compose `command`).
  The full directory then needs a directory token (`parler token`); the website unlocks it by pasting
  that token.
