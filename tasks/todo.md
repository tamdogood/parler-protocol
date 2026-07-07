# SEO audit + update: rank for "chat protocol for agents" / "agent file transfers"

## Audit findings
1. **CRITICAL — canonical host is wrong.** `web/lib/seo.ts` set `SITE_URL =
   https://parler-hub.fly.dev` (the *hub server*), but the marketing site lives at
   `https://www.parlerprotocol.com`. That misdirected every canonical tag, the sitemap,
   robots.txt (host + sitemap), OpenGraph/Twitter `url`, and all JSON-LD `url` to a host that
   serves a different app and 404s on `/blog/*`. Fixing the one constant fixes them all.
2. **HIGH — the home (money) page never targeted "chat protocol for AI agents".** Its `<title>`
   and H1 lead with "share context"; the tagline phrase lived only in metadata, not visible copy.
3. **HIGH — "agent file transfers" had ZERO on-site coverage** despite the shipped
   `com.parler.file` feature (`parler send-file` / `parler fetch`). No keyword, section, example,
   FAQ, or post.
4. **MEDIUM — KEYWORDS list** missing both target phrases.

## Changes (on-page, honest — the features are real)
- [x] `lib/seo.ts`: `SITE_URL` → `https://www.parlerprotocol.com`; add file-transfer to the
      site description; prepend target keywords.
- [x] `app/page.tsx`: home `<title>` + description lead with "the chat protocol for AI agents"
      and mention file/code transfer; broaden the Hardening "transfers" card to name file transfer.
- [x] `components/hero.tsx`: open the supporting copy with "Parler is the chat protocol for AI
      agents" so the exact phrase is visible above the fold (and the hero finally says what it *is*).
- [x] `components/examples.tsx`: add a "Send a file" tab with the real `send-file`/`fetch` commands.
- [x] `components/faq.tsx`: fold "chat protocol for AI agents" + file/code transfer into the first
      answer; add a dedicated "Can agents send each other files?" Q&A (feeds FAQPage schema).

## Verify — DONE
- [x] `npm ci` + `next build` → 50/50 pages, no type/lint errors.
- [x] Generated `robots.txt`, `sitemap.xml`, home `<title>`, `<link canonical>`, `og:url`, and all
      JSON-LD `url` now point at `https://www.parlerprotocol.com` (verified in `.next/` output).
- [x] "chat protocol for AI agents" renders in the home `<title>` + hero body; "Send a file" tab +
      `parler send-file` + the file-transfer FAQ render on the home page and in FAQPage JSON-LD.
- [x] The only remaining `parler-hub.fly.dev` refs are `wss://parler-hub.fly.dev` (the live hub
      endpoint agents dial) — correct, left untouched.

## Follow-up — DONE: dedicated blog post for "agent file transfers"
- Shipped `how-ai-agents-send-each-other-files` via /write-blog. Angle: "a file is bytes, and
  base64-in-chat taxes size + context tokens; put bytes on the content-addressed blob path." Owns
  the "agent file transfer" cluster; links to (and is reciprocally linked from) the code-handoff
  post so they don't cannibalize.
- Wired: `docs/blog/*.md` source, `web/components/blog/*.tsx` body (prose primitives only),
  `web/lib/blog.ts` POSTS entry, BODIES map + import in `app/blog/[slug]/page.tsx`, on-brand SVG
  cover, repo-to-post backlink from `docs/file-transfer.md`.
- Verified: `next build` green (52 pages); post `<title>`/description(=dek)/canonical(apex)/
  og:image/twitter:image/BlogPosting JSON-LD all emit; slug in sitemap + /blog index card + cover
  200. Scanner clean except the verbatim `parler recv` line (📎 + the em dash the CLI itself
  prints), matching the shipped code-handoff precedent.

## Distribution (still open — outward-facing, needs your go)
- `/x-tweet` thread teaching the base64-tax insight, linking the post (not the homepage).
- Answer the real question where it's asked (HN / r/rust / r/LocalLLaMA) with the post as the
  fuller answer. Not done autonomously.
