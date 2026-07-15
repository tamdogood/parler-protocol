# Campaign playbook

Use the channel copy as a starting point, then attach the recommended artwork and a real handoff demo.
The useful thing is the mechanism, not the launch announcement.

## Launch sequence

### Day 1: the problem

Show the clipboard tax. Record a short clip of one agent finishing a task, another tool opening, and
the usual transcript paste that Parler removes.

### Day 2: the handoff

Publish the 30 or 60 second demo with the portable conversation key visible. Use `--approval` in a
team/security version of the recording. Attach the
[square handoff artwork](../assets/marketing/session-handoff-square.png) to the text post.

### Day 3: the honest security model

Explain that identity is cryptographic but message plaintext is visible to the hub operator. Show
`parler connect --local` and the local SQLite file. Pair the post with the
[local/private artwork](../assets/marketing/local-private-wide.png). Candor is part of the product
story.

### Day 4: the team use case

Use the [team session artwork](../assets/marketing/team-session-wide.png). Show one conversation
reaching three visible agents during a hackathon or shared project, with `--approval` when the owner
should vet the team.

### Day 5: the protocol beneath the wedge

Show that conversation handoff sits on DMs, channels, service queues, memory, signed receipts, files, and
code bundles. Use the [shared-memory artwork](../assets/marketing/shared-memory-wide.png),
[signed-identity artwork](../assets/marketing/signed-identity-square.png), or
[code/file handoff artwork](../assets/marketing/code-file-handoff-wide.png) for the feature you lead
with. Link to `docs/communication.md`.

## X launch post

> I got tired of copy-pasting a coding-agent transcript into the next tool, then explaining the part
> the paste missed.
>
> So I built Parler Protocol.
>
> One visible agent starts a live conversation. The next joins with one private command and lands
> already caught up. No hidden worker. No Enter press in the other window.
>
> One Rust binary. CLI + MCP. Local or shared hub.
>
> `parler connect`
>
> https://www.parlerprotocol.com

Attach: [session-handoff-square.png](../assets/marketing/session-handoff-square.png)

## X technical thread

1. Most multi-agent setups still have one hidden component: you. You select the transcript, switch
   windows, paste it, then rebuild whatever context did not survive.
2. Parler's first job is small: keep one live conversation visible across Claude Code, Codex, and
   OpenCode. Agent A starts it and gets a portable `KEY@HUB` command.
3. Possession admits by default. Treat the key like a password, or create with `--approval` so the
   owner admits each joiner separately.
4. After admission, the joiner pulls the same durable room log. A server-side cursor tracks what it has
   read, so later pulls return only what is new.
5. Identity is an Ed25519 public key. The agent proves ownership on connect. The seed stays on its
   machine.
6. Honest limit: the hub sees plaintext. Run `parler connect --local` when the operator should be you.
7. The whole thing is one Rust binary with a CLI and MCP server. Try the 60-second demo:
   https://github.com/tamdogood/parler-protocol

Attach to post 1: [session-handoff-hero.png](../assets/marketing/session-handoff-hero.png)

## LinkedIn post

I kept becoming the message bus between my coding agents.

One tool had the decisions. Another had the codebase. A teammate's agent needed both. The workflow was
select, copy, switch windows, paste, then write a smaller explanation for everything the transcript
did not capture.

Parler Protocol replaces that handoff with a portable conversation key. The next Claude Code, Codex,
or OpenCode agent joins the same live thread already caught up. Add `--approval` when the owner should
vet every joiner.

It is one open-source Rust binary that ships as a CLI and MCP server. It also provides signed agent
identity, discovery, shared memory, files, and service queues over a small WebSocket and SQLite hub.

The security boundary is explicit: the hub can read plaintext, so sensitive work belongs on
`parler connect --local`.

Project and demo: https://www.parlerprotocol.com

Attach: [session-handoff-square.png](../assets/marketing/session-handoff-square.png)

## Hacker News

**Title:** Show HN: Parler Protocol, keep one coding-agent conversation live across tools

**First comment:**

I built Parler because I was copy-pasting context between coding agents and becoming the coordination
layer myself.

The main flow is a visible conversation. One Claude Code, Codex, or OpenCode agent starts it and gets
a portable key. Another supported visible agent, possibly on a teammate's machine, redeems it and
continues in the same native UI with existing room context. Possession admits by default;
`--approval` adds owner-controlled admission.

The implementation is a Rust workspace. The transport is WebSocket JSON, the hub stores rooms and
cursors in SQLite, and agent ids are Ed25519 public keys proven by challenge-response. The same
binary ships as the `parler` CLI, an MCP stdio server, and the hub.

It does not claim end-to-end encryption. The hub sees plaintext, which is why there is a local mode.
I would especially value feedback on the protocol boundary and whether the handoff flow matches how
people actually move between coding tools.

Repo: https://github.com/tamdogood/parler-protocol

## Reddit or community post

**Title:** I built a way to move a live coding-agent conversation between tools without pasting the transcript

I use more than one coding agent and kept repeating the same bad handoff: copy the transcript, paste
it into another tool, then explain the decisions the paste did not make obvious.

Parler Protocol lets a visible Claude Code, Codex, or OpenCode agent start a live conversation and
share one private command. The next visible agent joins immediately with the existing context loaded,
even from a different supported host, and either agent's signed message starts a turn in the other
window. Add owner approval when the key may leave the trusted team.

It is open source, written in Rust, and ships as both a CLI and MCP server. The default hosted hub is
the quickest path, while `--local` keeps the hub and chat on the machine.

I am looking for feedback from people who regularly switch between Claude Code, Codex, Cursor, or
team setups with several agents.

Demo and docs: https://www.parlerprotocol.com

## Product Hunt

**Tagline:** Share the conversation. Skip the transcript.

**Description:** Move a live coding-agent conversation from one tool or teammate to another with a
portable key. Parler Protocol wires supported agent hosts with one command, then gives them conversation
handoff, signed identity, discovery, shared memory, messaging, and files through one open-source Rust
binary.

**Maker comment:**

I built Parler after noticing that every multi-agent workflow still had one manual integration: the
human clipboard. The first thing I wanted was a clean conversation handoff. Start it, share the
portable key, and let the next supported visible agent continue from the same thread. Add approval
when the key may leave the trusted group.

The broader protocol grew from that flow. Agents need a way to prove who they are, find peers, pull
only new context, and exchange code or files without living in one framework. Parler packages those
pieces as a CLI and MCP server on top of a small WebSocket and SQLite hub.

Please try the handoff and tell me where the flow still makes you do coordination work.

## Email announcement

**Subject:** Stop re-briefing your next coding agent

I built Parler Protocol to remove one repetitive part of multi-agent work: carrying the conversation
between tools.

One agent starts a live conversation and gives you a portable key. The next supported visible agent
joins the same thread already caught up. Add `--approval` when you need to vet the joiner. No
transcript paste and no second brief.

Install it once, then connect the supported agents on your machine:

```bash
curl -fsSL https://raw.githubusercontent.com/tamdogood/parler-protocol/main/scripts/install.sh | sh
parler connect
```

See the demo and security model at https://www.parlerprotocol.com.

## Demo scripts

### 15 seconds

1. Show agent A with a live coding conversation.
2. Say: "Normally I would paste this whole thread into the next tool."
3. Show the portable conversation key.
4. Show agent B joining and receiving the existing context.
5. End card: "Share the conversation. Skip the transcript."

### 30 seconds

1. "I am switching coding agents, but I do not want to write the brief again."
2. Run `parler conversation --host claude --resume last` in agent A's workspace.
3. Copy the printed `KEY@HUB` command into an OpenCode or Codex terminal.
4. Show the backlog and a signed peer turn in both native UIs.
5. Optionally repeat with `--approval` to demonstrate owner admission.
6. "Same conversation, different tool. `parler connect`."

### 60 seconds

1. Start with the clipboard problem in one sentence.
2. Run `parler connect --list` to show the supported hosts already wired.
3. Start `parler conversation --host claude --resume last` for agent A.
4. Start agent B with the printed portable key and a different supported host.
5. Show a signed message wake agent B and its response return to agent A.
6. Ask agent B, "What decision did we make and what is next?"
7. Show the correct answer from the shared context.
8. Close with the honest boundary: "The shared hub sees plaintext. Use `--local` when you need the
   hub to stay on this machine."

For a terminal-only recording, use `./scripts/demo-handoff.sh`.
