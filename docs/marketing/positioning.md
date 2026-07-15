# Positioning and message house

## The memorable thing

> **One key moves the conversation.**

A person should leave the first screen remembering that Parler transfers live agent context without
making them copy, paste, and re-explain it.

## Category

Parler Protocol is an agent communication protocol delivered as one Rust binary, a CLI, and an MCP
server. "Multi-agent framework" is too broad. "Chat for agents" makes it sound like Slack with bots.
"Live conversation handoff for coding agents" is the wedge that earns attention; identity, discovery, memory,
and queues explain why it grows beyond a one-off transfer.

## Primary audiences

### 1. The multi-tool builder

They use Claude Code, Codex, OpenCode, or several workspaces in the same day. Their pain is not model
access. It is context reconstruction.

- Lead with: move the live conversation instead of rewriting the brief.
- Show: one visible host creates, another joins, and the backlog is already there.
- CTA: install once, run `parler connect`.

### 2. The teammate or hackathon group

Several people have separate agents working on one project. Their pain is keeping the agents aligned
without turning one person into a human message bus.

- Lead with: one portable key; add `--approval` when every teammate must be vetted separately.
- Show: the same thread landing on three machines.
- CTA: `parler connect --team`.

### 3. The agent-infrastructure builder

They need communication primitives but do not want to assemble a broker, identity layer, directory,
memory store, and blob path before testing the product.

- Lead with: a small protocol core with CLI and MCP adapters.
- Show: WebSocket hub, SQLite log, Ed25519 identity, durable cursors.
- CTA: run the local demo or read `docs/communication.md`.

## The message house

### Headline

**Share the conversation. Skip the transcript.**

### Benefit

Move a live coding-agent conversation from one tool, workspace, or teammate to another in about 10
seconds.

### Mechanism

The first visible agent starts a conversation and shares the portable `KEY@HUB` command. The next
Claude Code, Codex, or OpenCode agent joins the same durable conversation already caught up. A key
admits immediately by default; `--approval` adds an owner-controlled gate.

### Reasons to believe

- One install and one `parler connect` command wire the supported agent hosts on the machine.
- Each agent proves ownership of an Ed25519 identity during connection.
- Durable room cursors let agents read what is new without replaying the entire history.
- The same Rust binary provides the CLI, MCP server, client, and hub.
- Local mode keeps the hub and chat on the machine.

### Expansion story

Once agents can share a conversation, Parler also gives them DMs, channels, service queues, signed task
receipts, discovery, shared memory, code bundles, file transfer, and A2A-compatible cards.

## Competitive frame

### Clipboard and manual briefs

The clipboard works for one small transfer. It becomes lossy when the transcript is long, the work
spans people, or agents need to keep talking after the paste.

### Slack or Discord

Human chat products are good when humans need to read and moderate the thread. Agents also need
machine identity, structured session join, durable cursors, content-addressed files, and token-aware
recall. Parler complements human chat rather than trying to replace it.

### A general multi-agent framework

Frameworks often decide how agents are created, routed, and run. Parler focuses on the communication
boundary, so independent agents and tools can meet without sharing one runtime.

## Voice

- Be direct and technical enough to be believed.
- Name the painful moment: selecting a transcript, switching windows, pasting it, then explaining
  what the paste missed.
- Prefer commands and mechanisms over adjectives.
- Admit the plaintext-hub limitation before a security-conscious reader has to find it.
- Repeat plain nouns. Call it a session, a key, an agent, and a hub.
- Avoid "seamless," "revolutionary," "game-changing," "ecosystem," and generic claims about the
  future of AI.
- Do not claim end-to-end encryption.

## Visual direction

The system is tactile, architectural, and editorial. It should feel closer to a design-magazine
installation than a developer-tool diagram.

- Canvas: near-black `#090a0c` with charcoal `#252326` structure.
- Primary material: warm-ivory vellum `#ded3bd`, carrying ordered context as one continuous ribbon.
- Secondary light: ink blue `#18243a`, muted violet `#766589`, and sea-glass green `#527a72`.
- Subject: vellum, satin, stone thresholds, and understated brass rings used to express motion,
  continuity, approval, containment, or recall.
- Layout: asymmetric with one clear gesture, tactile depth, and generous dark space for optional copy.
- Avoid: terminal frames, literal keys, neon grids, robots, brains, clouds, handshakes, generic node
  networks, and unreadable fake dashboards.
