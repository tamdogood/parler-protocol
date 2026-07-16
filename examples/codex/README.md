# Using Parler Protocol with Codex

The recommended Codex workflow keeps the normal TUI visible and shares turns continuously with
Claude Code, OpenCode, or another Codex instance. Use the MCP-only controls later when you need a
scripted room rather than a visible conversation.

## Install and connect

From a release:

```bash
curl -fsSL https://raw.githubusercontent.com/tamdogood/parler-protocol/main/scripts/install.sh | sh
parler connect codex
```

From this checkout:

```bash
cargo install --path crates/parler-bin --force
parler connect codex
```

`parler connect codex` merges a `parler` MCP entry into `~/.codex/config.toml`; it does not replace
other servers. A bare `parler connect` wires every detected host. Use `--local` for on-device-only
traffic or `--team` for a LAN hub with a join secret.

## Start or join a visible conversation

Codex is the default visible host:

```bash
# Create a new conversation from the latest Codex thread in this workspace.
parler conversation --topic auth-redesign --resume last

# In another terminal, join the exact hub carried by the printed key.
parler conversation KEY@HUB
```

Parler starts Codex app-server plus its normal remote TUI, adopts the TUI's native thread, and injects
valid signed peer messages as turns in that same visible conversation. Human-typed Codex turns and
final responses are posted back automatically. Shared file references are materialized before a late
joiner's catch-up turn.

Use `--resume <thread-id>` for a specific thread or omit `--resume` for a new blank Codex thread.
The private key admits its holder immediately by default; add `--approval` when possession should
only request access:

```bash
parler conversation --topic sensitive-review --resume last --approval
```

Codex never fabricates approval for a peer-injected turn. App-server routes those requests to the
bridge connection, where Parler declines escalation or returns an empty grant. Human-started TUI
turns keep Codex's normal approval flow.

## Mix providers

The portable key is host-independent:

```bash
# Creator in Codex
parler conversation --topic release-review --resume last

# Joiner in Claude Code
parler conversation KEY@HUB --host claude

# Another joiner in OpenCode
parler conversation KEY@HUB --host opencode
```

All three adapters share the same signed backlog, files, terminal-scoped identity, presence, durable
cursor, task receipts, and explicit handoff behavior. Their native integration details are documented
in [`../../docs/visible-host-adapters.md`](../../docs/visible-host-adapters.md).

## Compatible low-level controls

For scripts or a host without a visible adapter, use the older room/session controls:

```bash
parler session open --topic review --context "Current decision and files"  # key admits immediately
parler session join KEY@HUB
parler send --room <room> "review this"
parler recv --room <room>
```

The same operations are available through `parler_open_session`, `parler_join_session`,
`parler_send`, and `parler_recv`. Delivery alone does not wake every visible host. Use `parler work
--runner codex` for a bounded managed headless task or `parler supervise --runner '<command>'` for an
explicit local runner; neither is a substitute for the visible conversation adapter.

## Troubleshooting

```bash
parler connect --list
parler doctor
codex --version
```

If `parler conversation` reports missing app-server or remote-TUI support, update Codex. It never
falls back silently to `codex exec`. For hub routing, resume, and identity diagnostics, see
[`../../docs/troubleshooting.md`](../../docs/troubleshooting.md).
