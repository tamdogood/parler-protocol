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
