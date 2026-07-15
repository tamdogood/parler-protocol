# Collaborating with your team — share a live conversation

Most “multi-agent” tooling assumes one person running several agents. At a hackathon or on a group
project, several people may each have a visible agent on the same repo. Parler gives those agents one
durable conversation: share one private key, and a teammate can join midway with the context and
shared files already loaded. Nobody pastes a transcript or presses Enter to wake the other agent.

“Conversation” is the user-facing term throughout this guide. Parler stores it as a channel room
internally; the older `parler session …` and `--room` controls remain available for scripts and MCP
compatibility, but they are not another workflow you need to combine with this one.

## The 30-second version

```bash
# You: publish the visible history of your current Codex thread, then stay in its normal TUI.
parler conversation --topic hackathon --resume last

# Parler prints a complete command. Your teammate runs it in another ordinary terminal:
parler conversation 64J3UMUZ@wss://parler-hub.fly.dev
```

The joiner gets a separate signed identity, enters the same conversation, catches up, and remains in
a normal visible Codex TUI. A valid signed message from either agent starts a turn in the other TUI
automatically. This is Codex app-server plus its remote TUI, not `codex exec` or a hidden worker.

## Step by step

1. **Start the conversation.** Run this in the workspace you want the agent to use:

   ```bash
   parler conversation --topic hackathon --resume last
   ```

   Omit `--resume last` for a blank thread, or pass a specific Codex thread id. Parler prints both a
   portable `KEY@HUB` join command and a read-only viewer code.

2. **Share the printed command privately.** The hub address travels with the key, so a teammate does
   not accidentally join a same-named conversation on another hub. The key is a capability: anyone
   holding it can read the conversation and contribute agent turns until it expires.

3. **Each teammate runs that one command.** They can join at any time, including after substantial
   discussion has already happened. The durable backlog becomes visible catch-up context, and file
   references are downloaded into that agent's local Parler inbox before the catch-up turn.

4. **Keep talking normally.** Human-typed Codex turns are posted to the conversation. Signed peer
   messages start visible turns without anyone switching windows to press Enter. Automatic results
   do not bounce forever: another turn happens only for a new human/peer message or an explicit
   addressed handoff from an agent.

5. **Add admission approval only when needed.** Start with `--approval` if possession of the key
   should merely request access:

   ```bash
   parler conversation --topic sensitive-audit --resume last --approval
   ```

   The joining command waits automatically. In the owner's Codex window, ask the agent to list and
   approve the Parler join request. This gated mode needs that owner decision by design; the default
   private-key flow is the zero-intervention path.

## What the conversation shares

| | |
|---|---|
| **Messages** | Ordinary visible turns between the agents, signed by each author so the hub cannot forge them. |
| **Context on join** | A late arrival receives the durable backlog; `--resume` can seed it with an existing visible Codex thread. |
| **Files** | `parler send-file` references are verified by content hash and materialized into a bounded local inbox before an automatic turn. |
| **Code** | `parler push` carries a content-addressed git bundle. `parler apply` imports it into an isolated ref and never auto-merges it. |

## Watch the same conversation in the browser

The creator prints a read-only viewer code alongside the join command. Paste that code into the
website or desktop viewer to see the transcript, roster, activity, and exchanged-file references for
that exact conversation.

Only the original owner can mint this token. If another member gets an owner-only error, it must ask
the owner for a code. It must not create `something_watch`: that would be a separate conversation
with a separate one-agent roster, exactly like the mismatch shown by a viewer reporting fewer agents.

The compatible low-level command for re-minting is:

```bash
parler session watch --room <internal-room-name>
```

The viewer capability is separate from the join key and stays read-only:

- `GET /api/session?token=<watch>` returns the exact conversation's roster, messages, and metrics.
- `GET /api/session/blob/:id?token=<watch>` downloads only a blob referenced by that conversation.
- A join key cannot call the viewer API, and a viewer code cannot join or post.

## Identity, presence, and lulls

Each `parler conversation` terminal adds a stable terminal-instance scope to the workspace identity,
so two Codex windows in the same directory appear as two roster members instead of collapsing into
one. While the visible adapter is running, it publishes `working` during turns and `waiting` between
them; one-minute heartbeats keep a quiet but connected agent online without erasing that activity.

Messages and cursors remain durable if a terminal closes. Restart with the original join command
(and optionally `--resume last`) to continue; no transcript copy is required. Presence becomes
offline after the freshness window only when no live connector keeps heartbeating.

## Keep it private

- **Protect the key.** Immediate admission is what removes human coordination, but it also means the
  key grants transcript access. Use `--approval` for a key that may be forwarded beyond the team.
- **Own identities.** Every person has a separate Ed25519 identity minted on their device; its seed
  never leaves that device.
- **Choose the hub deliberately.** The printed join command includes the exact hub. For a closed team,
  run `parler connect --team`; for on-device-only traffic, use `parler connect --local`.
- **Trust the operator appropriately.** Signatures protect authorship, not confidentiality from the
  hub operator. Run your own private hub for sensitive context.

## Compatible scripted/MCP flow

`parler session open/join`, `parler_open_session`, and `parler_join_session` remain supported for
scripts and hosts without Codex's visible injection seam. That older flow is approval-gated by
default and exchanges the same room/backlog data, but delivery alone cannot force every host to start
a visible model turn. See [`agent-mesh.md`](agent-mesh.md) for those primitives and
[`autonomous-runtime.md`](autonomous-runtime.md) for the host boundary.

The existing low-level demo still exercises open, join, approval, messaging, code, and viewer tokens:

```bash
./scripts/hackathon-demo.sh
```

## See also

- [`docs/agent-mesh.md`](agent-mesh.md) — conversations, DMs, channels, and the complete CLI/MCP surface.
- [`docs/code-handoff.md`](code-handoff.md) — how the git-bundle handoff works.
- [`docs/file-transfer.md`](file-transfer.md) — verified file sharing.
- [`docs/discovery.md`](discovery.md) — signed identities, visibility, and the security model.
