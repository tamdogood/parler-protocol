---
name: write-blog
description: >-
  Write and ship a blog post for the Parler Protocol website (web/) that reads like a human
  wrote it and pulls SEO traffic to the repo and hub. Use when a contributor wants
  to draft, edit, or publish a blog post, add a post to the site, write about
  Parler Protocol for search/traction, or make an existing draft sound less like AI.
  Enforces the house voice (no em dashes), picks a non-cannibalizing angle, wires
  the post into Next.js, and runs the humanizer pass before shipping.
license: Apache-2.0
compatibility: claude-code
allowed-tools:
  - Read
  - Write
  - Edit
  - Grep
  - Glob
  - Bash
  - AskUserQuestion
---

# Write a Parler Protocol blog post

You are helping a contributor write a blog post for the Parler Protocol website. The bar: it
reads like a person wrote it, it ranks for a search cluster no other post already owns,
and it ships wired correctly into the Next.js site with the build green.

This skill exists so every contributor writes in the same voice, picks non-overlapping
topics, and passes the same anti-AI-slop gate, whether or not they already know the house
rules.

## The one rule that gets a post rejected

**No em dashes (—) or en dashes (–). Ever.** This repo is hand-styled and the house
voice bans them. Use a period, a comma, or "so"/"because"/parentheses instead. A single
dash in a draft is a hard fail. There is a scanner: run `bash check.sh <file>` (in this
skill's folder) on any draft before you call it done.

## Workflow

Do these in order. Don't skip the angle check or the humanizer pass.

### 1. Read the ground truth first

Before writing a word, read what already exists so you match voice and don't cannibalize
keywords:

- `web/lib/blog.ts`, the `POSTS` array. This is the **live registry of every published
  post and the angle it owns.** Read every `slug`, `title`, `dek`, and `tags`.
- One or two full posts in `docs/blog/*.md` (the prose sources) to absorb the voice.
- `reference/angles.md` (in this skill folder) for how to pick an angle that doesn't
  collide, plus the topics still untapped.
- `reference/voice.md` for the house voice and the anti-AI checklist, self-contained.

### 2. Pick and pitch the angle

Every post must **own a distinct search cluster.** Two posts fighting for the same
keywords cannibalize each other. From `POSTS` and `reference/angles.md`, pick an angle no
shipped post already owns, then pitch it back to the contributor in one line before
drafting: the working title, the one-sentence thesis, and the 3-5 keyword phrases it
targets. Confirm before you write the body. If they gave you a topic that collides with an
existing post, say so and propose the adjacent-but-distinct angle instead.

### 3. Outline

A Parler Protocol post has a spine, not a listicle. Structure:

- **A lead (dek + opening) that names a concrete, specific problem** the reader has. Not
  "AI agents are transforming collaboration." Something like "Two agents can talk about a
  change all day. Handing over the change itself, byte for byte, is a different problem."
- 4-7 sections, each with a claim and **real code, real commands, or a real number from
  this repo** backing it. Read the actual source (`crates/`, `web/`, `docs/`) and quote it
  accurately. Made-up APIs are worse than no code.
- A "what this is NOT" or honest-limitations beat. The voice is a little contrarian and
  admits what's deferred. That honesty is what makes it not read like marketing.
- A close that lands the thesis without a generic "in conclusion, the future is bright"
  wrap-up. End on a concrete thing the reader can do or check.

### 4. Draft

Write the prose in `docs/blog/<slug>.md` first (plain markdown, the repo's source of
truth). Follow `reference/voice.md` as you write, so there's less to fix later. Keep it
honest, concrete, and grounded in repo code. Vary sentence length. Have an opinion.

### 5. Humanize (mandatory)

Run the repo's humanizer on the draft: invoke `/humanizer` (committed at
`.claude/commands/humanizer.md`) on `docs/blog/<slug>.md`. If it isn't available, apply
the checklist in `reference/voice.md` yourself. Strip: significance inflation, promotional
adjectives, the rule of three, "-ing" tail analyses, negative parallelisms ("it's not X,
it's Y"), synonym cycling, hedging, and every dash. For headline/dek/CTA polish you may
also pull in `/direct-response-copy`.

Then run `bash check.sh docs/blog/<slug>.md` and fix anything it flags.

### 6. Wire it into the site

Follow `reference/wiring.md` exactly. In short: add the `POSTS` entry in `web/lib/blog.ts`,
create the body component `web/components/blog/<slug>.tsx` using the prose primitives from
`components/blog/prose.tsx`, and register `slug` to `<Component />` in
`web/app/blog/[slug]/page.tsx`'s `BODIES` map. Sitemap, RSS, OG/Twitter cards, and
JSON-LD auto-derive from `POSTS`, so there's no extra wiring. Add internal links to 1-2
related posts (helps SEO and keeps readers on-site).

### 7. Verify before done

From `web/`:

```
npm run build          # must be green
```

Then smoke it: `next start` and confirm the post page, the cover image, the `/blog` index,
`/sitemap.xml`, and `/rss.xml` all return 200 and the new slug appears. Re-run
`bash check.sh` on the final markdown. Only then is it done.

## Guardrails

- Never invent Parler Protocol APIs, flags, or numbers. Read the source and quote it.
- Never weaken the house voice to sound "professional." Concrete and a little contrarian
  beats polished and generic every time.
- Don't ship a post whose angle overlaps a published one. Adjust the angle instead.
- Covers can be SVG served via a plain `<img>`. On-brand palette: black `#000`,
  electric-blue `#3b9eff`, violet `#9281f7`, green `#3ecf8e`, graphite hairlines. If you
  can't make a good cover, reuse an existing one rather than shipping something ugly.

## Note on this skill's own files

The scanner will flag `SKILL.md` and the `reference/` files because they name the banned
characters `(—)` `(–)` and use `→` as teaching notation. That's expected. The scanner is
for blog drafts in `docs/blog/` and the `.tsx` bodies, not for this skill's documentation.
