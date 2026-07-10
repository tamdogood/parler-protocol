# SEO: rank for "agent protocol" + "agent communication" (web/)

Goal: help the site rank for two head terms it doesn't currently own — **"agent protocol"** and
**"agent communication"** — without diluting the sharp homepage positioning ("chat protocol for AI
agents") or resorting to thin doorway pages.

## Diagnosis (current state)

- Technical SEO is already strong: `metadataBase`, per-page canonicals, file-convention OG/Twitter
  images, `sitemap.ts`, `robots.ts`, `manifest.ts`, WebSite + SoftwareApplication JSON-LD.
- 21-post blog gives real topical authority — "agent communication" already appears across 3 posts;
  identity/MCP/A2A/memory clusters exist.
- **Gap:** no single page owns the exact phrase "agent protocol" or "agent communication" in its
  `<title>` / H1 / URL. The head terms have cluster content but no *pillar* to consolidate it.
- **Bonus finding:** `SITE_URL` = apex `parlerprotocol.com`, but the apex 308-redirects to `www`
  (noted in `lib/seo.ts`). Canonicals/sitemap therefore point at a URL that redirects → wasted hop +
  split signal. Needs a host decision (flip redirect to apex, or set SITE_URL to www).

## Plan — pillar-and-cluster (white-hat)

### Phase 1 — Two pillar pages (the core lever)
- [ ] `/agent-protocol` — genuine explainer: what an agent protocol is, the pieces (identity,
      addressing, delivery, memory, discovery), MCP/A2A vs a chat-layer protocol, Parler as a
      concrete one. Exact phrase in slug + `<title>` + H1 + description + H2s. Links down to cluster
      posts (what-a-chat-protocol-for-agents-needs, mcp-a2a-and-where-agents-live,
      how-ai-agents-prove-who-they-are, real-time-messaging-for-ai-agents).
- [ ] `/agent-communication` — explainer: how AI agents communicate, the hard parts (delivery, the
      next turn, real-time push, shared memory), Parler's answer. Exact phrase in slug/title/H1/desc/
      H2s. Links to cluster (agent-communication-the-next-turn, real-time-messaging-for-ai-agents,
      what-a-chat-protocol-for-agents-needs, agent-collaboration-vs-orchestration).
- [ ] Each page: FAQPage + BreadcrumbList JSON-LD (rich-result eligible; "what is an agent protocol"
      is a real query), self canonical, OG/Twitter metadata, reuse existing components (NavBar,
      Reveal, Footer). House voice (no em dashes), run humanizer.

### Phase 2 — Reinforce ranking signals
- [ ] Add exact-phrase keywords ("agent protocol", "agent communication") to `KEYWORDS` in lib/seo.ts.
- [ ] Add both pages to `sitemap.ts` (priority ~0.9).
- [ ] Internal links with exact-phrase anchor text: footer "Learn"/"Resources" column + one
      contextual link from the homepage + from the 2–3 most relevant blog posts.
- [ ] Add BreadcrumbList/FAQPage JSON-LD helpers to lib/seo.ts (keep it the one source of truth).

### Phase 3 — Technical win (needs user decision)
- [ ] Resolve apex↔www canonical split (recommend: set SITE_URL to the non-redirecting host).

### Phase 4 — Verify (web/ IS in scope — direct user request; see lessons.md)
- [ ] `npm run build` green; inspect generated `<head>` for both pages; validate JSON-LD.
- [ ] Confirm sitemap + robots include the pages; internal links resolve.
- [ ] `make ci` (or `CI_SKIP_WEB=1` while iterating on non-web) green.
- [ ] Doc-drift check: grep new URLs/claims across README/AGENTS/docs/web; update any drift.

## Review (done — web gate green)

Shipped the pillar-and-cluster play. All phases complete and verified via `scripts/ci/web.sh`
(`npm ci` + `next build`, ✓ 18s), plus HTML inspection of the prerendered output.

**New files**
- `web/app/agent-protocol/page.tsx` — pillar page. `<title>` "Agent protocol: how AI agents connect,
  identify, and talk — Parler Protocol", H1 "What is an agent protocol?", canonical
  `/agent-protocol`, BreadcrumbList + FAQPage (6 Q&A) JSON-LD, links down to the identity/discovery/
  MCP-A2A cluster + cross-link to /agent-communication.
- `web/app/agent-communication/page.tsx` — pillar page. `<title>` "Agent communication: how AI agents
  talk to each other", H1 "How do AI agents communicate?", canonical `/agent-communication`,
  BreadcrumbList + FAQPage (6 Q&A), links to the next-turn/real-time/collaboration cluster.
- `web/components/seo-faq.tsx` — reusable static (server-rendered) FAQ that emits FAQPage JSON-LD in
  lockstep with visible answers (rich-result requirement).

**Edited**
- `web/lib/seo.ts` — `SITE_URL` → `https://www.parlerprotocol.com` (was the redirecting apex; every
  SEO surface reads this, so canonical/sitemap/robots/OG/JSON-LD all moved to www, apex leak count 0);
  added exact-phrase keywords "agent protocol" + "agent communication" (+ a2a variant); added
  `breadcrumbJsonLd()` helper.
- `web/components/footer.tsx` — added both pages to the site-wide Resources column (exact-phrase
  anchor text on every page).
- `web/app/sitemap.ts` — both pages at priority 0.9.
- `web/README.md` — route inventory updated (doc-drift).

**Verified in prerendered HTML:** exact-phrase titles/H1s, www canonicals, keyword-rich meta
descriptions, og/twitter cards (explicitly re-attached the root OG image, which a custom openGraph
object otherwise drops), BreadcrumbList + FAQPage JSON-LD present, sitemap lists both on www.

**Doc-drift:** no CLI/MCP/wire/REST/security surface changed, so no repo-doc updates needed beyond
the web/README route list. House voice held (no em dashes in prose; the only `—` is the brand
separator in `<title>`, matching the root layout template).

**Not done (needs owner/infra, flagged to user):** nothing outstanding for the two terms. Ranking is
earned over weeks — next step is to submit the updated sitemap in Search Console and watch impressions
for the two head terms.
