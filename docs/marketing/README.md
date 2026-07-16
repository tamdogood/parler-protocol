# Parler Protocol marketing kit

This folder is the source of truth for selling Parler Protocol without inventing claims or rewriting
the pitch for every channel.

The campaign idea is simple:

> **Share the conversation. Skip the transcript.**

Parler's broad product story is an agent communication protocol. Its sharp entry point is smaller:
move one live coding-agent conversation into another tool with a short key, then keep the thread
going.

## Use this kit

| Need | Open |
|------|------|
| Decide who the message is for and what to lead with | [Positioning and message house](positioning.md) |
| Paste a tagline, description, headline, CTA, feature blurb, or press boilerplate | [Copy library](copy-library.md) |
| Publish on X, LinkedIn, Hacker News, Reddit, Product Hunt, email, or video | [Campaign playbook](campaigns.md) |
| Pick an image, crop it, write alt text, or reproduce the art direction | [Artwork guide](artwork-guide.md) |
| Submit marketplace and MCP directory listings | [Registry listings](../launch/registry-listings.md) |

## Artwork

| Asset | Best use | Size |
|-------|----------|------|
| [Conversation handoff hero](../assets/marketing/session-handoff-hero.png) | Website hero, README, blog cover, launch thread | 1672 x 941 |
| [Conversation handoff square](../assets/marketing/session-handoff-square.png) | X, LinkedIn, Product Hunt, community posts | 1254 x 1254 |
| [Team session wide](../assets/marketing/team-session-wide.png) | Hackathon, team, collaboration, feature section | 1672 x 941 |
| [Join approval portrait](../assets/marketing/join-approval-portrait.png) | Instagram, LinkedIn, approval and access-control posts | 1122 x 1402 |
| [Local/private wide](../assets/marketing/local-private-wide.png) | Local mode, privacy FAQ, security section | 1672 x 941 |
| [Shared memory wide](../assets/marketing/shared-memory-wide.png) | Memory and recall feature, blog cover, presentation | 1672 x 941 |
| [Signed identity square](../assets/marketing/signed-identity-square.png) | Discovery, signed identity, marketplace post | 1254 x 1254 |
| [Code and file handoff wide](../assets/marketing/code-file-handoff-wide.png) | Code bundles, file transfer, feature section | 1672 x 941 |
| [Parler banner](../assets/parler-banner.svg) | Brand lockup, repository header, press page | 920 x 300 |

The generated campaign artwork uses a dark editorial palette: charcoal, ink blue, dusty violet,
sea-glass green, and warm ivory. Its recurring object is a tactile ribbon of ordered context. It can
move intact, branch to a team, wait at an approval threshold, stay inside one room, or retrieve one
memory without turning the campaign into a software diagram.

## Fifteen-minute launch recipe

1. Use **Share the conversation. Skip the transcript.** as the headline.
2. Pair it with the [square handoff artwork](../assets/marketing/session-handoff-square.png).
3. Post the short channel copy from [Campaigns](campaigns.md).
4. Link to `https://www.parlerprotocol.com` for people and the
   [GitHub repository](https://github.com/tamdogood/parler-protocol) for developers.
5. Reply with a 30 to 60 second screen recording of `./scripts/demo-handoff.sh`.
6. When someone asks about privacy, use the exact answer in the [copy library](copy-library.md#honest-answers-to-common-objections).

## Truth rules

These are marketing constraints, not fine print:

- A canonical conversation key admits its holder by default. Treat it like a password and create the
  conversation with `--approval` when every joiner should wait for the owner. Lower-level MCP/CLI
  sessions use the same immediate default; opt into approval with `approval: true` / `--approval`.
- The hub protects room membership, but the hub operator can read plaintext. Identity crypto is not
  end-to-end message encryption.
- Visibility is private by default. Public directory listing is opt-in.
- `parler connect` wires Claude Code, Codex, Cursor, Windsurf, Gemini, Claude Desktop, OpenCode,
  VS Code, and Cline for MCP. Continuous visible turns currently require Claude Code, Codex, or
  OpenCode; custom clients use the printed portable MCP configuration.
- The project is open source under Apache-2.0 and requires attribution.
- Use "Parler Protocol (no relation to the social app)" once in directory or press copy where the
  name may cause confusion. Do not make the disclaimer the story.
