# Picking an angle that doesn't cannibalize

The blog's job is SEO traction to the repo (`tamdogood/parler-protocol`) and the hub
(`parler-hub.fly.dev`). That only works if each post owns a **distinct search cluster.**
Two posts chasing the same keywords split their own ranking. So the first rule of a new
post is: pick an angle no shipped post already owns.

## How to check what's taken

`web/lib/blog.ts` `POSTS` is the live registry. Read every `title`, `dek`, and `tags`
before pitching. Each shipped post owns one spine:

- **Code handoff / git bundles.** How agents move a change byte-for-byte as a content-
  addressed blob. (`how-agents-hand-off-code`)
- **Team sessions / share context.** Several people, each with an agent, on one repo;
  share a live session with one key. (`share-your-agent-context-with-your-team`)
- **Rust debugging war stories.** TLS panic, private-hub-that-wasn't, approval-gate
  bypass, crash loop, blocked async runtime. Spent the join-secret + approval-gate hooks.
  (`bugs-that-hid-until-production`)
- **Agent memory field guide (2026).** CoALA taxonomy, benchmarks, consolidation, the
  single-player critique. (`ai-agent-memory-in-2026`)
- **MCP + A2A vs. where agents live.** The protocols standardize talking, not a persistent
  place to meet. (`mcp-a2a-and-where-agents-live`)
- **Memory without a vector DB.** One SQLite file, FTS5 + sqlite-vec, RRF fusion.
  (`agent-memory-without-a-vector-database`)
- **Architecture deep dive.** Wire protocol, nkey identity, cursors.
  (`stop-copy-pasting-between-ai-agents`)

Always re-read `POSTS` at write time; this list is a snapshot and the array is the truth.

## Angles still untapped (good starting points)

- **A pure security / trust deep dive.** Nkey identity, private-by-default, watch tokens.
  Note: the war-stories post already spent the join-secret and approval-gate stories, so a
  security post needs a different spine (e.g. the identity/trust model end to end).
- **SQLite-as-a-server / ops.** The async-blocking anecdote is spent, but retention,
  Litestream, the 1-writer + N-reader pool, and streaming blobs are open.
- **Loop engineering / the autonomous agent building itself.** `/loop /work-next`, the
  backlog-queue + verify-gate harness. (`docs/loop-engineering.md`)
- **A2A interoperability in practice.** Projecting signed cards into A2A Agent Cards.
  (`docs/a2a-interop.md`)
- **Why not just point agents at Slack/Discord.** The honest case. (`docs/vs-slack.md`)

## When to write a checklist / listicle instead

Usually don't. The house format is a spine with a thesis, not a listicle. A "top N tools"
post ranks briefly and ages badly, and it doesn't fit the voice. If a contributor wants
one, steer them to a spine post on the same topic instead.

## Keyword discipline

- One primary phrase per post; put it in the `title`, the `dek` (meta description), the
  first H2, and the `tags`.
- Pick phrases a real Rust/agent developer would type into a search box (an exact error
  message, a concept like "agent code handoff", a comparison like "MCP vs A2A"), not
  brand-y phrases nobody searches.
- Internal-link to 1-2 sibling posts so ranking flows between them instead of competing.
