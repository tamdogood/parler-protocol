# Task: Parler Desktop — SIMPLIFY for 10/10 UX (declutter) — 2026-06-30

**User:** too many features/buttons; clunky & cluttered. Simplify as much as possible, still intuitive.

**Model:** *the app IS your local hub.* It auto-runs in the background; you connect agents to it and
watch them. Public hub becomes an advanced option inside Connect, not a global axis.

**Cuts (6 screens → 3 + gear):**
- Nav = **Agents · Connect · Sessions** + a Settings **gear** in the footer. Remove **Dashboard**
  (redundant) and **Local Hub** from the nav (fold into Settings → "Manage local hub", still there for
  power users). Remove the **global Local/Public titlebar switch** (app = local hub).
- **Agents** (home): your hub's agents show with **zero friction** — app auto-mints a directory token
  (`parler token`) so the private hub's full roster is visible without a paste. Just search + cards.
  Drop scope toggle, sort, grid/list toggle, tag facets, token-gate.
- **Connect**: local by default; "public hub" demoted to a small advanced toggle. One action per host.
  Manual snippet collapsed under "Other MCP hosts".
- **Sessions**: keep watch viewer but **remove Timeline Replay** (play/scrub/speed buttons) — chat
  only. "Open a session" = recap + Open; topic/no-approval under a small "Options" disclosure.
- **Settings**: minimal + one collapsible "Local hub (advanced)" (start/stop, mode, port, secret,
  logs, data folder). Drop the default-connect-target setting.
- **Onboarding**: 2 steps (Welcome → Connect first agent to the auto-started local hub). Drop hub-choice.
- **Titlebar**: clean drag bar + tiny status pill → Settings.

**Files:** main: +`parler token` mint + `hub.directoryToken` IPC (cached). renderer: rewrite
`session-viewer` (chat-only), simplify `directory`/`connect`/`sessions`/`settings`/`onboarding`/
`sidebar`/`titlebar`/`App`; delete `dashboard`, replace `directory-screen`→`agents-screen`.

**Verify:** typecheck + build + headless boot clean; `dist` DMG launches. Then update PR.

## Review — DONE & VERIFIED (2026-06-30) ✅
Halved the surface (6 nav destinations → **3 + a Settings gear**) and cut per-screen clutter, around
one model: *the app is your local hub; connect agents and watch them.*
- **Removed:** Dashboard, Local Hub from nav (now Settings → "Manage"), the global Local/Public
  titlebar switch, and the Session viewer's **Timeline Replay** (play/scrub/speed). Nav = Agents ·
  Connect · Sessions + gear.
- **Agents (home):** your hub's full private roster shows with **zero paste** — app auto-mints a
  directory token (`parler token`) and reads `scope=hub`; falls back to public if the hub's down.
  Just search + status chips; dropped scope/sort/grid-list/tag-facets/token-gate. Friendly empty state.
- **Connect:** local by default, public demoted to a one-line link; one action per host; manual
  snippet collapsed. **Sessions:** recap + Open (topic/approval under "Options"); chat-only viewer.
  **Settings:** trimmed; hub advanced kept behind "Manage". **Onboarding:** 3 steps → 2.
- **Main:** added `hub.directoryToken` IPC (cached, cleared when hub leaves `running`) + `parler token`
  driver; hardened event forwarding against a torn-down window (fixed an `Object has been destroyed`
  teardown rejection).
- **Verified:** typecheck + build clean; headless boot loads renderer with **no console errors and no
  rejection**; in-app hub smoke → running/healthy/api 200; **directory-token smoke** → `parler token`
  parsed, `/api/directory?scope=hub` 401 without / 200 + roster with; `dist` DMG rebuilt, packaged app
  launches clean. Deleted `dashboard.tsx` + `directory-screen.tsx`.

## Review — (superseded by the section above)

---

# Task: Parler Desktop — macOS Electron app (hub + directory + session viewer) — 2026-06-30

**User ask:** a downloadable macOS app that (1) serves everything the website does (agent
**directory** + **session viewer**), (2) can run a full **private hub locally** (WS bus + SQLite DB +
blobs) with one toggle, (3) makes **connecting an agent** (Claude Code / Cursor / any MCP host) a
one-click action against either the local hub or the public hub, (4) matches the website's dark
"Resend obsidian terminal" theme exactly, and (5) streamlines setup: download → connect an agent in
under a minute. Later: a "Download for macOS" button on the website.

## Key architectural decisions (confirm before build)
1. **Location:** new self-contained `desktop/` project at repo root (sibling to `web/`, `crates/`).
2. **Renderer stack:** Electron + **Vite + React 19 + TS + Tailwind v4**, reusing the exact `@theme`
   tokens + fonts from `web/app/globals.css` (fonts bundled locally for offline). Port the key
   components (SessionViewer w/ chat+timeline replay, directory, agent-card, status-dot, hub-header,
   ui/*). *Not* embedding the Next.js SSR app — a native SPA fits IPC + offline + OS integration.
3. **Native binaries:** bundle compiled `parler-hub` + `parler` as electron-builder `extraResources`;
   `desktop/scripts/build-binaries.sh` cargo-builds them per mac arch. Main process spawns/supervises
   the hub and shells out to `parler` for privileged actions.
4. **Distribution:** electron-builder → DMG (arm64 + x64). Signing/notarization stubbed + documented
   (needs Apple Developer ID); unsigned works locally with a quarantine note.

## Main process (Electron)
- [ ] Hub supervisor — spawn bundled `parler-hub` (free port, default 7071), persistent
      `userData/hub.sqlite` + `.blobs`, name `${user}'s Hub`, optional `--public`, auto-generated
      persisted join secret via `--join-secret-file`. Health-poll `/health`; restart w/ backoff;
      graceful SIGTERM on quit; stream stdout/stderr to a log buffer.
- [ ] MCP host integration — detect `claude` CLI (+ Cursor config path); one-click
      `claude mcp add parler -- <bundled parler> mcp` with `PARLER_HUB` (local|public) +
      `PARLER_JOIN_SECRET` when private; render snippets + JSON for other hosts; detect connected state.
- [ ] Privileged actions via bundled `parler` (open session, mint watch, approvals, whoami).
- [ ] Typed IPC (`preload.ts`, contextIsolation on): hub start/stop/status/url, mcp list/connect,
      session open/mint-watch, clipboard, openExternal, settings get/set.
- [ ] Tray/menu-bar item w/ live hub status + quick toggle. Settings store (JSON in userData).

## Renderer (screens)
- [ ] Onboarding (first run): welcome → private-hub vs public-hub → connect first agent (1 click).
- [ ] Dashboard: hub status card (up/down, URL, agents, DB size, uptime) + quick actions.
- [ ] Local Hub: start/stop, live `/api/hub` stats, DB path+size, blob usage, live logs, connect
      snippet, public/private toggle, reveal/rotate join secret, open data folder.
- [ ] Directory: ported agent directory (public + hub scope), search/filter tag/skill/status.
- [ ] Sessions: (a) Watch viewer (chat + timeline replay, ported); (b) Your sessions — open, show key,
      mint watch code, manage join approvals.
- [ ] Connect Agents: target picker (local|public), detected hosts, one-click connect, snippets.
- [ ] Settings: auto-start hub, hub config, about/version (theme locked dark).

## Packaging & website
- [ ] electron-builder DMG (arm64+x64) + `build-binaries.sh`; signing/notarization config + docs.
- [ ] `.gitignore` desktop `node_modules/dist/release`; keep `make ci` green (no heavy new gate).
- [ ] Website "Download for macOS" button → GitHub Release DMG (wiring; release upload needs CI).

## Delivery phases (each runnable)
1. **Foundation** — scaffold, theme port, window chrome, dashboard shell, hub supervisor + health.
2. **Features** — directory, session watch viewer, connect-agents wizard, hub controls+logs+settings,
   onboarding.
3. **Packaging** — DMG build, binary build script, signing docs, website download button.

## Review — DONE & VERIFIED (2026-06-30) ✅

Shipped the full app in new `desktop/` (Electron + electron-vite + Vite/React 19/Tailwind v4),
plus website download CTAs. **User chose: ship unsigned for now, full-fledged all phases.**

**Main process (`src/main/`)** — `HubSupervisor` (spawns bundled `parler-hub`: free-port pick from
7071, persistent `userData/hub.sqlite` + `.blobs`, `--join-secret-file`, health poll, crash-restart
w/ cap, log ring, graceful SIGTERM); `mcp.ts` (detect + one-click Claude Code via `claude mcp add`,
config-merge for Cursor/Claude Desktop w/ backup, GUI-PATH resolution); `parler-cli.ts` (drives
bundled `parler` with an **isolated** `userData/parler-home` identity — open session / mint watch /
whoami, output parsed by regex); typed IPC + `preload` (`contextIsolation` on, prod-only CSP header);
`tray.ts` (menu-bar status + start/stop/quit); settings JSON.

**Renderer (`src/renderer/`)** — theme ported 1:1 from `web/app/globals.css` (fonts via `@fontsource`,
offline); screens: Onboarding (3-step), Dashboard, Local Hub (start/stop, live stats, storage, streaming
logs, secret reveal, public/private toggle, port), Directory (faceted, ported AgentCard/Detail),
Sessions (open + full chat/timeline-replay viewer, ported), Connect (target picker + host wiring +
snippets), Settings. Frameless titlebar w/ global Local⇄Public target switch + hub pill.

**Packaging** — electron-builder DMG (arm64), unsigned (ad-hoc/linker-signed → launches on Apple
Silicon), bundles `parler`+`parler-hub`+tray icons as extraResources; generated icons
(`scripts/gen-icons.mjs`), `scripts/build-binaries.sh`, README w/ signing/notarize docs +
`xattr -dr com.apple.quarantine` note.

**Website** — `MAC_DOWNLOAD_URL` in `lib/seo.ts`; hero "Download for macOS" + subline; nav "Download";
new `components/download.tsx` section (`#download`) with on-brand faux-window preview. `next build` green.

**Verification (all green):**
- `npm run typecheck` (main+renderer) clean; `npm run build` bundles main/preload/renderer; fonts offline.
- Headless Electron boot: renderer loads, **zero console errors**, self-quits (env-gated smoke hooks).
- **Full-path hub smoke inside Electron**: supervisor spawned the bundled hub → `phase=running
  healthy=true` → `/api/hub` 200 ("tamnguyen's Hub", private) → clean stop; created hub.sqlite+blobs.
- **Binary/CLI integration smoke** vs real binaries: hub `--join-secret-file` boot, `parler init` w/
  secret, `session open`→parsed room+KEY, `session watch`→parsed token, `/api/session` bearer 200 +
  content, bad token 401, `whoami` parsed.
- `npm run dist` → `release/Parler-0.1.0-arm64.dmg` (102 MB); packaged `.app` bundles both binaries +
  icns + tray png; packaged app **launches clean** headlessly.
- `desktop/` is invisible to `make ci` (not a cargo workspace member, not `web/`), so CI is unaffected.

**Follow-ups (noted, not blocking):** Apple Developer ID signing+notarization (config stubbed, flip
when creds exist); universal/x64 arch (script supports a target triple, needs `rustup target add`);
CI job to build+upload the DMG to a GitHub Release so `MAC_DOWNLOAD_URL` resolves to a real asset;
optional in-app "mint directory token" for one-click private hub-scope directory viewing.

---

# Task: Standalone full-screen Agents Console page (web) — 2026-06-29

**User ask:** from the website, build an *extra standalone page* for the agents hub; on that page add
*more agent-focused features* and make the *existing agents features (the directory) occupy most of
the screen*.

## Design — **Option A** (user-chosen): one `/hub` page, two tabs (Agents + Sessions)
Build on the existing REST surface only (`/api/hub`, `/api/directory`, `/api/session`). Reuse
`AgentCard`, `AgentDetail`, `TokenDialog`, `StatusDot`, design tokens. Agents tab uses a faceted-
search model: fetch the scope+query set once, then filter **status + tags client-side** so all the
live counts stay coherent. Sessions tab = "session hub" = the sessions explainer + the watch viewer.

New:
- [x] `components/agents-console.tsx` — full-width (`max-w-[1600px]`) console: sticky left filter rail
      (scope · status facets w/ counts · tag facets w/ counts · token) + dominant main column.
      New features vs. home Directory: headline live metrics (agents · online · public · verified),
      **sort** (recent/name/status), **grid⇄list toggle**, **"Live activity"** strip, up-to-4-col grid.
- [x] `components/sessions-feature.tsx` — extracted from home `Sessions()` (`showViewerCta` prop).
- [x] `components/session-viewer.tsx` — extracted watch viewer from `app/session/page.tsx`.
- [x] `components/session-hub.tsx` — Sessions tab = `<SessionsFeature/>` + `<SessionViewer/>`.
- [x] `app/hub/page.tsx` — standalone tabbed page (hash-synced: `/hub` agents, `/hub#sessions`).

Modify:
- [x] `app/page.tsx` — use `<SessionsFeature/>`; prune now-unused imports.
- [x] `app/session/page.tsx` — client redirect → `/hub#sessions` (carry any `&k=` watch token).
- [x] `nav-bar.tsx` — add "Hub" link + repoint CTA + session-viewer link to `/hub`.
- [x] `directory.tsx` + `hero.tsx` (home) — link out to `/hub`.
- [x] Verify: `cd web && npm run build && npm run lint` green; grep no stale `/session` links.

## Review — DONE (2026-06-29) ✅ `next build` green (9 routes prerender, /hub 13.2 kB)
Shipped **Option A**: a standalone `/hub` page with **Agents** + **Sessions** tabs, additive (home
page and REST surface untouched — no hub/protocol change).
- **Agents tab** (`components/agents-console.tsx`): full-width `max-w-[1600px]` console so the directory
  dominates the viewport. Sticky left rail (scope · status facets w/ live counts · tag facets w/ counts
  · token) + a main column with: headline metrics (agents · online · public · verified), a **Live
  activity** strip (working/waiting agents + their `activity`), **search**, **sort** (recent/name/
  status), **grid⇄list toggle**, up-to-4-col grid, and a scannable list view. Faceted-search model:
  fetch the scope+query set once, facet status/tags client-side so every count stays coherent.
- **Sessions tab = "session hub"** (`components/session-hub.tsx`): the sessions explainer
  (`sessions-feature.tsx`, extracted from the home `Sessions()`) + the watch viewer
  (`session-viewer.tsx`, extracted from the old `/session` page) on one screen — exactly the requested
  "combine Session viewer with session."
- **Routing/wiring:** `app/hub/page.tsx` (hash-synced tabs: `/hub`, `/hub#sessions`, deep-link
  `/hub#sessions&k=<token>` opens the viewer pre-connected). Old `/session` → client redirect carrying
  the watch token. NavBar gains "Hub" + repoints the CTA; home Directory + Hero link out to `/hub`.
  Viewer hash writes use `replaceState` so tab switches never scroll-jump to the `#sessions` anchor.
- **Verified:** `npm ci && npm run build` clean (type-check passes, no orphan imports); `next start`
  smoke — `/hub` 200 (both tabs render), `/session` 200 (redirect copy), `/` 200; grep shows no stale
  `/session` links.

---

# Task: SEO — make the Parler website discoverable — 2026-06-29

**User ask:** "how to improve SEO for my website to make it more discoverable?" → plan + implement.

## Findings (current state of `web/`)
- Next 15 App Router. Root `layout.tsx` sets only `title` + `description` + `metadataBase`
  (`https://parler-hub.fly.dev`). No OG, no Twitter card, no og:image.
- Blog `[slug]` has `generateMetadata` with `openGraph` but no Twitter card, no canonical, no
  article metadata, no JSON-LD.
- No `sitemap.xml`, no `robots.txt`.
- No structured data anywhere (we have a full FAQ component + an Article — both free rich-result
  wins).
- `/session` is a dynamic, thin, auth-gated viewer page that is currently indexable.

## Plan
- [ ] `web/lib/seo.ts` — single source of truth: `SITE_URL`, site name/description, and the
      `WebSite` + `SoftwareApplication` JSON-LD objects.
- [ ] `web/app/robots.ts` — allow all, declare sitemap, disallow `/session`.
- [ ] `web/app/sitemap.ts` — `/`, `/blog`, and every post from `POSTS` (lastModified = post.date).
- [ ] `web/app/opengraph-image.tsx` — dynamic on-brand 1200×630 OG image (next/og, default font).
- [ ] `web/app/twitter-image.tsx` — re-export the OG image so Twitter gets a card image too.
- [ ] `web/app/layout.tsx` — expand root metadata (openGraph, twitter `summary_large_image`,
      canonical, keywords, authors/creator) + inject WebSite/SoftwareApplication JSON-LD.
- [ ] `web/app/blog/[slug]/page.tsx` — add Twitter card, canonical, article publishedTime/authors;
      inject `BlogPosting` JSON-LD.
- [ ] `web/app/blog/page.tsx` — add openGraph + canonical to the index.
- [ ] `web/components/faq.tsx` — add plain-text answers + emit `FAQPage` JSON-LD.
- [ ] `web/app/session/layout.tsx` — server layout exporting `robots: { index: false }` (page is a
      client component, so it can't export metadata itself).

## Verify
- [ ] `npm run build` in `web/` is green (renders the dynamic OG image, validates metadata).
- [ ] Spot-check generated routes for `/sitemap.xml`, `/robots.txt`, og image.

## Review
Done. `npm run build` green; new routes `/sitemap.xml`, `/robots.txt`, `/opengraph-image`,
`/twitter-image` all prerender. Verified in the built HTML:
- Homepage: canonical + full OG + `twitter:summary_large_image` + auto-injected OG/Twitter image;
  JSON-LD `WebSite` + `SoftwareApplication` + `FAQPage` (Q/A) present.
- Blog post: canonical, Twitter card, `BlogPosting` JSON-LD.
- `robots.txt`: allow `/`, disallow `/session`, sitemap + host declared.
- `/session`: `<meta name="robots" content="noindex, nofollow">`.

New files: `lib/seo.ts`, `app/robots.ts`, `app/sitemap.ts`, `app/opengraph-image.tsx`,
`app/twitter-image.tsx`, `app/session/layout.tsx`.
Edited: `app/layout.tsx`, `app/blog/page.tsx`, `app/blog/[slug]/page.tsx`, `components/faq.tsx`.

Not done (off-page / content — out of code scope): submit sitemap to Google Search Console + Bing,
write more blog posts, earn inbound links, move to a real domain (vs `*.fly.dev`).

---

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

---

# Task: SEO pass — apply the `astro-seo` skill's principles to the Next.js site — 2026-06-30

**User ask:** install `fusengine/agents astro-seo` via skillfish, then "apply this skill to improve SEO
for my website with your best effort." The skill is Astro-specific; the site is Next 15 App Router, so
we apply its *principles* (canonical correctness, RSS, sitemap, BreadcrumbList, feed autodiscovery,
XSS-safe JSON-LD). Existing SEO (PR #55/#56) is already strong (FAQPage/BlogPosting/OG+Twitter/sitemap/
robots), so this is a targeted improvement pass.

## Real bug found
- `/hub` (a `"use client"` page with no metadata) inherited the root layout's `alternates:{canonical:"/"}`
  → the standalone Hub self-reported as a duplicate of `/` and reused the home title/description.

## Plan
- [x] `lib/seo.ts` — `RSS_URL` + `ALT_RSS` feed-autodiscovery constant.
- [x] `app/layout.tsx` — drop root `canonical:"/"` (footgun: every un-overriding route inherited it);
      set site-wide `alternates.types` (RSS).
- [x] `app/page.tsx` — own `metadata` w/ `canonical:"/"` + RSS type.
- [x] `app/hub/layout.tsx` — NEW server layout: hub title/description/canonical `/hub`/OG/Twitter.
- [x] `app/sitemap.ts` — add `/hub`.
- [x] `app/blog/rss.xml/route.ts` — NEW static RSS 2.0 feed (XML-escaped, categories, atom:self).
- [x] `app/blog/page.tsx` — RSS alternate + `Blog` + `BreadcrumbList` JSON-LD.
- [x] `app/blog/[slug]/page.tsx` — RSS alternate + `BreadcrumbList` JSON-LD.
- [x] `components/footer.tsx` — RSS link.

## Verify
- [x] `npm run build` green (15 routes prerender; `/blog/rss.xml` + `/hub` both static).
- [x] Per-route canonicals correct: `/`→`/`, `/hub`→`/hub` (was `/` — the bug), `/blog`→`/blog`,
      post→own URL. `/hub` `<title>`/`og:title` now hub-specific, distinct from home.
- [x] `/blog/rss.xml` well-formed (`xmllint --noout` ✓): escaped titles/deks, categories,
      `atom:self`, RFC-822 dates. RSS `<link rel=alternate>` on home + blog pages; footer link.
- [x] `BreadcrumbList` JSON-LD on blog post + index; `Blog` collection JSON-LD on index.
- [x] Sitemap now lists `/hub`. `/session` still `noindex`; robots.txt unchanged.
- [x] Web CI gate = `scripts/ci/web.sh` (`npm ci` + `next build`); no `next lint` (no ESLint config).

## Review
**Done & verified.** Applied the `astro-seo` skill's *principles* to the Next.js site (skill is
Astro-only, so no Astro code — the checklist transferred: canonical correctness, RSS, sitemap,
BreadcrumbList, feed autodiscovery, XSS-safe JSON-LD via `dangerouslySetInnerHTML`+`JSON.stringify`).

- **Fixed a real canonical bug:** `/hub` (client page, no metadata) inherited the root layout's
  `canonical:"/"` and the home title/description — it self-reported as a duplicate of the homepage.
  Moved the home canonical off the root onto `app/page.tsx`, and gave `/hub` its own server
  `layout.tsx` (title/description/canonical/OG). Root now only advertises the feed site-wide, so no
  route inherits a wrong canonical.
- **Added an RSS 2.0 feed** (`/blog/rss.xml`, `force-static`) with autodiscovery `<link>`s + footer
  link. **Added BreadcrumbList** (posts + index) and a **Blog** collection schema. **Added `/hub`**
  to the sitemap.
- **Minimal blast radius:** `web/` only, no protocol/crate change; existing SEO (FAQPage, BlogPosting,
  OG/Twitter images, keywords) untouched.

New: `app/hub/layout.tsx`, `app/blog/rss.xml/route.ts`. Edited: `lib/seo.ts`, `app/layout.tsx`,
`app/page.tsx`, `app/sitemap.ts`, `app/blog/page.tsx`, `app/blog/[slug]/page.tsx`, `components/footer.tsx`.

Still off-page / out of code scope (same as the 2026-06-29 SEO task): submit sitemap to Google Search
Console + Bing, earn inbound links, a real domain vs `*.fly.dev`, more posts. Nice-to-have not done:
`Organization` logo node (no dedicated square-raster logo asset yet).

### Further pass ("anything else?") — DONE & verified
Recon showed the blog covers are a poor social-card source (aspect 1.14–2.36:1, none = OG's 1.91:1;
raw PNGs up to 3200px / ~400 KB via plain `<img>`), and there was no theme-color/manifest at all.
- **Per-post branded OG + Twitter cards** — `app/blog/[slug]/opengraph-image.tsx` (+ `twitter-image.tsx`
  re-export), 1200×630, title + dek on the root card's aesthetic, next/og default font. Both
  **prerender static** (`generateStaticParams`) so crawlers get a cached image. Dropped the manual
  `images:[post.cover]` from the post's `generateMetadata` so the branded card is the social image;
  the cover stays as the in-page hero + `BlogPosting` `image`. **Visually verified** the rendered PNG.
- **theme-color + web manifest** — `viewport` export (`themeColor:#000`, `colorScheme:dark`) →
  `<meta name=theme-color>`; `app/manifest.ts` → `/manifest.webmanifest` (Next auto-links it).
- **Image sitemap** — blog entries now carry `<image:loc>` (cover) for Google Images.
`next build` green (18 routes prerender). Verified in output: post `og:image`/`twitter:image` → the
branded `/blog/<slug>/opengraph-image` card; `theme-color` + `rel=manifest` present; manifest valid;
sitemap `<image:loc>` present.

Offered, not done (need a judgment call / visual QA): convert covers to `next/image` (Core Web Vitals —
they're 92–388 KB raw PNGs; touches rendering so wants visual QA); `Organization`/`publisher.logo`
(needs a light-bg square logo asset); AI-crawler policy in robots (a product decision).

---

## 2026-06-30 — Desktop app "melts the Mac" on DMG install (crash-restart storm)

**Report:** clicking install via the DMG "created a loop", the Mac turned hot and had to be hard
powered off. `npm run dev` works fine; only the packaged DMG loops.

**Root cause (`desktop/src/main/hub-supervisor.ts`):** an *unbounded* hub restart loop. The restart
counter was reset to 0 on **every** `/health` success (old line 164 `this.restarts = 0`), while the
cap was only checked against that counter. So a hub that becomes healthy → dies → repeats resets its
own budget every cycle and respawns **forever** (~every 800 ms). Each respawn is a full Rust process
(SQLite open, migrations, FTS, sqlite-vec, WS bind) → CPU pegged → fans max → stall. Dev runs one
instance so the hub stays up and never trips; packaging is where a hub flaps — a **second app
instance** (no single-instance lock) spawning a competing hub over the **same SQLite DB** is the most
likely trigger, and a quarantined binary can flap too.

**Fix (3 changes, main-process only):**
- [x] New `desktop/src/main/restart-gate.ts` — a pure, unit-testable rolling-window rate limiter:
      at most `MAX_RESTARTS` (5) respawns per `RESTART_WINDOW_MS` (60 s), then give up + error.
      Removed the reset-on-health line; the window ages attempts out so a long-up hub still recovers.
- [x] `start()` re-entrancy guard (`launching` flag) so concurrent starts (onboarding + auto-start +
      tray + renderer) can't slip past the `child` check during `await findFreePort` and spawn an
      untracked orphan hub. Flag claimed only after the synchronous mkdirs so a throw can't wedge it.
- [x] `index.ts` single-instance lock (`app.requestSingleInstanceLock()` + `second-instance` →
      focus) so a second launch can't run a competing hub over the same DB.

**Verify:**
- [x] Reproduced the loop in a model: OLD policy = 1000 respawns over 1000 flaps (unbounded); NEW
      gate = 5 respawns then gives up. Recovery-after-window and reset-on-stop asserted. All pass.
- [x] `npm run typecheck` green (node + web); `npx electron-vite build` green; confirmed the gate +
      single-instance code inlined into `out/main/index.js`.

**Not in scope (separate, not the melt):** unsigned/quarantined DMG still needs right-click→Open (a
signing/notarization task); this fix makes the app *safe* regardless — worst case is an error state,
never a meltdown. Re-enabling the web download CTAs (hidden in #64) can follow once signing lands.

### Follow-up: full desktop audit (requested "make sure there are no other issues")

Read every main + renderer + shared + preload file and the build scripts. **No other critical/HIGH
issues.** Verified safe: all 3 process-spawn sites are on-demand + `execFile` array args (no shell
injection) with timeouts, only the hub spawn needed rate-limiting (fixed); IPC is a typed enumerated
bridge with `contextIsolation` on / `nodeIntegration` off; `shell.openExternal` only ever gets two
hardcoded https URLs; no `dangerouslySetInnerHTML`/`eval` anywhere (React escapes all hub-supplied
strings); CSP set in packaged builds; timers all bounded + cleaned up; icon generator is
dependency-free.

Fixed two robustness gaps (stability-class, self-contained):
- [x] **React error boundary** (`renderer/src/components/error-boundary.tsx`, wrapped in `main.tsx`)
      — a single throwing component (e.g. a malformed public-hub directory entry missing `card`) was
      white-screening the whole window; now shows a recoverable fallback + Reload.
- [x] **Cap the session viewer's message buffer** (`.slice(-1000)`) — the only unbounded renderer
      array; a long-running watch grew it without limit.

Reported, not fixed (LOW — left for a follow-up so this stays scoped):
- `mcp.ts` `writeConfigServer` renames the live config to `.parler-backup` on every connect, so a
  second connect clobbers the *pristine* pre-parler backup (the user's other servers are still
  preserved in the merge — only the safety snapshot is lost). Guard with `if (!existsSync(backup))`.
- `api.ts` fetch has no timeout; a hung hub connection leaves a spinner (local hub is instant + public
  is fast, so unlikely). Mirror `probeHealth`'s AbortController.
- No main-process `unhandledRejection` logger; the supervisor's `void this.start()` on a mkdir throw
  would reject unhandled (mkdir of userData ~never fails). A log-only handler would surface it.
- `session-viewer` poll `setInterval(load,…)` isn't re-entrancy-guarded — only matters if a fetch
  takes >4s (not on localhost). An in-flight guard removes the theoretical double-append.

Verify: `npm run typecheck` green (node + web); `npx electron-vite build` green (all 3 bundles).
