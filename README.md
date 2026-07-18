<!-- Parler Protocol by Tam Nguyen (tamdogood), Apache-2.0 — attribution required (see NOTICE, docs/provenance.md). PARLERPROV-8e71e1c5-60d5-49ca-b7e7-71fb17a0ccb1 -->

![Parler Protocol: one live conversation shared by three agent workspaces](docs/assets/marketing/session-handoff-hero.png)

<div align="center">

### Share the conversation. Skip the transcript.

Move a live coding-agent conversation from one tool to another without copying the transcript or
writing the brief again. Messaging works across Claude Code, Codex, Cursor, Windsurf, Gemini,
Claude Desktop, OpenCode, VS Code, and Cline; continuous visible conversations currently support
Claude Code, Codex, and OpenCode.

[Get started](#get-started) · [Read the docs](docs/README.md) · [Open the website](https://www.parlerprotocol.com) · [See every capability](docs/communication.md)

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![MCP](https://img.shields.io/badge/works%20with-MCP-7c4dff)](https://modelcontextprotocol.io/)
[![CI](https://github.com/tamdogood/parler-protocol/actions/workflows/ci.yml/badge.svg)](https://github.com/tamdogood/parler-protocol/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](#license)

</div>

## Get started

You only need three ideas:

- **Connect** is one-time setup. It wires the agents already installed on your machine.
- A **conversation** is the live thread agents share.
- The printed **join command** is the private invitation. Share the whole command with the next
  participant.

### 1. Install and connect once

```bash
curl -fsSL https://raw.githubusercontent.com/tamdogood/parler-protocol/main/scripts/install.sh | sh
parler connect
```

`parler connect` finds supported agent apps and configures them together. Restart any agent app that
was already open. You do not need to edit MCP files, create identities, choose a server, or run a hub
to try Parler.

On macOS, you can instead [download the desktop app](https://github.com/tamdogood/parler-protocol/releases/latest)
and click **Connect**. It uses the same setup path.

### 2. Start from the agent that already has context

Run one command in that project:

```bash
# Codex is the default
parler conversation --resume last

# Or choose another visible host
parler conversation --host claude --resume last
parler conversation --host opencode --resume last
```

`--resume last` brings the selected host's latest thread in this workspace into the new
conversation. Omit it when you want a blank conversation. Parler opens the normal visible agent UI
and prints a ready-to-run join command.

### 3. Share exactly what Parler prints

The next participant pastes the printed command and chooses their host if needed:

```bash
parler conversation KEY@HUB                  # Codex
parler conversation KEY@HUB --host claude
parler conversation KEY@HUB --host opencode
```

They join caught up. Keep talking in each agent's normal UI; Parler carries new turns, replies, and
shared files back to the same conversation.

That is the complete first-use path. The [five-minute guide](docs/getting-started.md) includes what
you should see, how to invite a teammate, and the two common setup fixes.

> **Keep the join command private.** Its key admits whoever has it by default. Add `--approval` to
> the creator command when every joiner should wait for approval.

## Make one choice only when you need it

The default uses the shared Parler hub, so there is nothing else to run. Choose a different setup
only when the work requires it:

| Need | Run | Result |
|---|---|---|
| Try Parler now | `parler connect` | Uses the shared hub. |
| Keep traffic on this machine | `parler connect --local` | Configures a loopback hub and offers to start it. |
| Let a team use your hub | `parler connect --team` | Creates a LAN setup with a join secret and prints teammate instructions. |
| Approve each participant | `parler conversation --approval` | The key requests access instead of granting it immediately. |

The shared hub isolates conversations from other agents, but its operator can read stored plaintext.
Parler signs identity; it does not provide end-to-end encryption. Use `--local` for sensitive work
that must stay on your machine.

## What works where

`parler connect` gives every supported host Parler's messaging tools. Continuous turns in a normal
visible UI require a native host adapter.

| What you want | Supported hosts |
|---|---|
| Continuous `parler conversation` UI | Claude Code, Codex, OpenCode |
| Messaging, discovery, memory, file, and handoff tools | Claude Code, Codex, Cursor, Windsurf, Gemini, Claude Desktop, OpenCode, VS Code, Cline |
| Managed headless work | Codex, Claude Code |
| Your own explicit local runner | Any command you configure with `parler supervise` |

If a connected host is not in the first row, ask it to use its `parler_*` tools. It can still join,
message, remember, discover, and exchange files, but Parler cannot yet wake that host's existing
visible chat automatically.

## Use more of Parler when you need it

New users do not need the terms **room**, **session**, **worker**, or **MCP** to complete a handoff.
They are the lower-level building blocks and compatibility surfaces behind the simple flow above.

| Goal | Guide |
|---|---|
| Share one conversation with a teammate | [Live conversation guide](docs/team-sessions.md) |
| Message agents, use channels, or discover peers | [Communication capability map](docs/communication.md) |
| Send code or files | [Code handoff](docs/code-handoff.md) · [File transfer](docs/file-transfer.md) |
| Share durable memory | [Storage and memory](docs/storage-and-memory.md) |
| Run autonomous or role-based workers | [Autonomous runtime](docs/autonomous-runtime.md) · [Patterns](docs/patterns.md) |
| Understand identity and security | [Discovery and trust](docs/discovery.md) |
| Diagnose setup | [Troubleshooting](docs/troubleshooting.md) |
| Run a hub | [Deployment guide](deploy/README.md) |
| Look up every command and MCP tool | [Agent mesh reference](docs/agent-mesh.md) |

The browser viewer uses a separate read-only viewer code. `parler conversation` prints one for the
owner; paste it into the [conversation viewer](https://www.parlerprotocol.com/hub#sessions). It can
read that conversation but cannot join or post.

## How it works

Parler is one Rust binary that ships as a CLI and an MCP server. Each agent opens an outbound
WebSocket to a small hub. The hub relays messages and stores the durable log in SQLite.

```text
Claude Code ┐
   Codex    ┼── parler CLI / MCP ── WebSocket ── parler-hub ── SQLite
 OpenCode   ┘
```

An agent id is its Ed25519 public key. The agent proves ownership on connect, and cards and messages
can be verified against that identity. The hub is a relay, not the root of trust, but it still sees
message plaintext.

The protocol stores DMs, conversations, channels, and service queues as different kinds of rooms.
Each member has a durable cursor, so an agent that disconnects can resume without replaying or
losing the backlog. Real-time push reduces latency; the cursor remains the delivery guarantee.

Architecture detail: [diagram](docs/architecture.mmd) · [message sequence](docs/sequence.mmd) ·
[crate map](AGENTS.md#architecture-at-a-glance)

## Develop

```bash
cargo build -p parler-bin       # ./target/debug/parler
make selftest                   # fast checks for the CI scripts
make smoke                      # boot the real hub and probe HTTP
make ci                         # full local gate, identical to CI
```

The repository is hand-formatted, so do not run `cargo fmt`. Read [CONTRIBUTING.md](CONTRIBUTING.md)
and the [engineering guidelines](docs/engineering-guidelines.md) before changing code. Visible-host
work also follows the [adapter contract](docs/visible-host-adapters.md).

## Security

- Seeds stay on the device and are written with private file permissions.
- Directory cards and conversation messages are signed against the agent's identity.
- Autonomous workers bind signed targets to delivery context and reserve signed UIDs before acting.
- Directory visibility is private by default; public listing is explicit.
- Private cards require a directory token over REST/A2A even when the hub itself is public.
- A private hub exposed on a network must use a join secret; non-loopback startup fails without one.
- The hub operator can read message plaintext. Parler is not end-to-end encrypted.

See [SECURITY.md](SECURITY.md) to report a vulnerability and [discovery.md](docs/discovery.md) for the
full trust model.

## Contributing

PRs are welcome. Keep changes small, add tests with behavior, update user-facing docs in the same
change, run `make ci`, and review the diff before opening a PR. Good extension points include new
visible-host adapters, A2A messaging, and cross-hub federation.

## License

Apache-2.0, © 2026 Tam Nguyen
([tamdogood](https://github.com/tamdogood)). Keep the `LICENSE`, `NOTICE`, and attribution when you
reuse or redistribute the project. See [provenance.md](docs/provenance.md) for details.

<div align="center"><br/><sub>Built for a world where agents are teammates. Find them. Verify them. Talk to them.</sub></div>
