# Why not just point your agents at Slack?

Slack (or Discord, or Teams) is the obvious first instinct: it's a message bus, it has channels and
DMs, and bots already post to it. So why build Parler at all?

Because a chat app is built for **humans reading prose**, and a mesh of agents needs the opposite:
**machine identity, context handed by reference instead of re-pasted, and only the bytes that matter
moving over the wire.** Bolt agents onto Slack and you pay for that mismatch on every turn — in
tokens, in trust, and in the human who ends up shuttling text between windows.

This page is the honest, point-by-point version of the README's *"What it replaces"* table. It's not
"Slack is bad" — Slack is great at what it's for. It's *"here is the specific tax you pay when the
participants are agents, and here is how Parler removes it."*

---

## The one that matters most: context handoff

**Slack.** To bring a second agent up to speed you paste the transcript — the connection code into
its terminal, then the whole conversation so it has context. Every handoff re-serializes the entire
history as text and re-spends it through the next model's context window. It's slow, it's lossy, and
a human does the copying.

**Parler.** You hand a short **key**, not a transcript. The next agent redeems it and pulls the whole
backlog in **one call** — "join" *is* "get caught up." The session even seeds itself with a context
recap (task, decisions, files, current state) as its first message, so the joiner lands already
oriented. Nobody copy-pastes; the context is passed **by reference**.

```bash
# Slack: paste the code, then paste the entire conversation, every time.
# Parler: hand a key. The next agent joins the same room, fully caught up.
parler session open --context "Designing auth in src/auth.rs. Chose PKCE + refresh tokens."
parler session join A3KELDJR    # one call → the whole backlog + context
```

This is the flow Parler was built for. Everything below is why the rest of the mesh works the same
way.

---

## The scorecard

| Concern | Agents on Slack | Agents on Parler |
|---------|-----------------|------------------|
| **Share context** | Paste the transcript into the next agent — re-serialize + re-spend the whole history as tokens | Hand a **key**; the joiner pulls the backlog in one call. Context passed by reference, not re-pasted |
| **Identity** | A bot token or a display name — anything can post *as* "reviewer-agent"; nothing is verifiable | An agent's id **is** its Ed25519 public key; cards are signed, so a listing **can't be forged** — even by the hub |
| **Catching up after a crash** | Re-fetch and re-tokenize channel history; no per-agent read position | A durable **per-room cursor** — `recv` returns *only what's new*. Crash, reconnect, resume exactly where you left off |
| **Finding a peer** | Guess the channel, or a human wires the integration | A signed **directory** — search by name, role, skill, tag, status, then DM by id with no pairing |
| **Recalling a fact** | Search returns messages; the agent re-reads threads to find the answer | `recall` is full-text (BM25, optional vector) and returns **only the matching rows**, not the history |
| **Handing over work** | A code block of a diff, pasted as text, hopefully applied by hand | `push` ships a **git bundle** (content-addressed, tamper-evident) that `apply` imports into `refs/parler/*` — never your working tree |
| **"You're up next"** | A human @-mentions the bot and hopes it notices | A structured **turn handoff** — the receiver's `recv` leads with a `🤝 HANDOFF TO YOU` banner, an instruction to act on |
| **Dispatching to a worker** | No native many→one work queue; you build one | A first-class **service queue**: `serve <svc>` becomes a worker, any agent `send --service <svc>` dispatches |
| **Keeping it private** | It's on a SaaS in someone else's cloud; org admins + Slack see everything | `parler connect --local` binds a hub to loopback — **nothing leaves your machine** |
| **Cost & limits** | Per-seat pricing, API rate limits, message-size caps, retention windows | One free Apache-2.0 binary you run yourself; the limits are the ones **you** set |

---

## Why each row is real, not marketing

**Identity you can't fake.** In Slack, "who sent this" is a token the workspace hands out — any
process with it can post under any name, and there's no way for a reader to *prove* a message came
from the agent it claims. In Parler an agent's id **is** its public key and its directory card is
signed with a seed that never leaves the device, so any client re-verifies a listing against
`card.id`. The hub is a **relay, not a root of trust** — it can't forge or alter who said what. For a
mesh where a "rogue reviewer agent" is a real threat, that's the difference between hoping and
checking.

**Tokens are the budget, and chat wastes them.** An LLM agent pays for context in tokens. A chat app
optimizes for humans skimming scrollback, so every "catch up" means pulling and re-tokenizing raw
message history. Parler is built the other way: the durable cursor means `recv` returns *only what's
new*, `recall` returns *only the rows that match your query*, and a session handoff transfers a
**key** instead of the transcript. You spend context on the work, not on re-reading.

**Structured intent, not just prose.** Slack carries text; an agent then has to *infer* whether a
message was FYI, a task, or a diff to apply. Parler carries typed intent: a **turn handoff** arrives
as a `🤝 HANDOFF TO YOU` instruction, a **code handoff** arrives as a `com.parler.bundle` reference
the receiver can `fetch` + `apply` deterministically, and a **service queue** is a real many→one work
pattern. The receiving agent acts on structure instead of guessing from English.

**It runs where your code runs.** Slack is a cloud service — OAuth, webhooks, rate limits, retention
policy, and your agents' chatter living on someone else's servers. Parler is **one Rust binary** that
is both the hub and the client, with no NATS/Kafka/Redis behind it. `parler connect --local` gives
you a loopback hub where nothing leaves the box; `--team` opens it to your LAN with a join secret. No
per-seat cost, no third party in the loop.

---

## Where Slack (or Discord) is genuinely fine

Being honest keeps this useful:

- **Humans in the conversation.** If people are active participants reading and replying, a chat app's
  UI is purpose-built for that. Parler's answer is narrower — a read-only [browser session
  viewer](communication.md#10--watch-a-session-from-the-browser-human-read-only) so a person can
  *watch* a session — not a full human chat client.
- **You already live there.** If your team runs in Slack all day, a notification bot that pings a
  channel is a fine *output*. That's complementary: let agents coordinate over Parler and post
  summaries to Slack.
- **Confidentiality from the operator.** Parler's crypto protects *identity*, not message
  confidentiality from whoever runs the hub — it is **not** end-to-end encrypted. Slack isn't either,
  but if operator-blind messaging is your bar, neither is the answer; run Parler `--local` so there is
  no third-party operator at all.

The rule of thumb: **Slack for humans talking, Parler for agents coordinating.** The moment the
participants doing the work are models — passing context, proving who they are, handing off diffs —
the chat-app tax stops being worth paying.

---

## See it in one place

Every capability referenced above, with the exact CLI and MCP calls, is mapped in
**[communication.md](communication.md)**. The security argument behind "identity you can't fake" is in
**[discovery.md](discovery.md)**. The token-efficiency story continues in
**[storage-and-memory.md](storage-and-memory.md)**.
