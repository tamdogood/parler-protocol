# What a chat protocol for agents actually needs

Search "chat protocol for agents" and the top results define two message types and stop. A request and a response, a decorator to handle each, done. That is a message format. It is the easy fifth of the problem. The hard four fifths is everything the format sits inside: an identity nobody can forge, an address that says where a message goes and not just what it says, an acknowledgement that survives a crash, and a way for a fifth agent to join a running conversation already caught up.

This post takes the anatomy apart, one part at a time, and shows the real wire types from Parler Protocol next to the popular alternatives. The point is not that one is right. It is that "chat protocol" is doing a lot more work than the ranked tutorials admit.

## A chat protocol for agents is not a message format

Look at what the leading results actually specify. Fetch.ai's chat protocol, the one most tutorials teach, is two Pydantic models. A `ChatMessage` carries text, a `ChatAcknowledgement` carries the id of the message it confirms, and a decorator routes each to a handler. ASI:One's agent chat protocol tutorial builds on the same `uagents` library and adds `StartSessionContent` and `EndSessionContent` for lifecycle. That is the whole surface: text in, text out, an ack, a start and an end.

It is a clean design and it works. But notice what it assumes rather than defines. It assumes you already know who sent the message and that the sender is who they claim. It assumes the message reached the right place. It assumes that if the receiver was offline, something durable held the message until it came back. Those assumptions are the protocol. The message model is the part that was never hard.

So here is the frame for the rest of this post. A chat protocol for agents is four guarantees wearing a message format:

- **Identity** you can check without trusting a server.
- **Addressing** that distinguishes a broadcast from a direct message from a job for whoever is free.
- **Delivery** that a reader can resume after a crash without re-reading or losing a line.
- **Continuity**, so an agent that shows up late gets the context instead of a blank room.

Take them in order.

## Identity: the sender id is a public key, not a claim

In the tutorial protocols, a message's sender is a field. The framework fills it in from the connection, and you trust it because you trust the framework and the registry it talked to. Compromise the registry and a message can say it came from anyone.

Parler Protocol makes the id unforgeable by making it the key. An agent's id is its Ed25519 public key, generated locally, and the seed never leaves the device. The identity record it publishes, the `AgentCard`, is signed by that seed, so any client re-verifies the card against the id itself.

```rust
/// A2A-inspired identity record for an endpoint or agent.
pub struct AgentCard {
    /// Unique, stable for the lifetime of this connection (the agent's nkey public key).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    pub kind: EndpointKind,
    /// The role this participant plays (planner, reviewer, ...).
    pub role: Option<String>,
    // ... skills, tags, protocol version
}
```

That one comment on `id` is the whole trust model. Because the id is the public key, the card's signature is checkable by anyone holding the card, and the hub that stores it can route and log traffic without ever being able to impersonate a participant. There is no certificate authority, no login server, no chain to build. The full end-to-end version of that argument, with the signing and verify code, is in [how AI agents prove who they are](/blog/how-ai-agents-prove-who-they-are).

The lesson for the protocol: identity is not a field you fill in. It is a property of the id, or it is a claim you are choosing to believe.

## Addressing: one message, three ways to route it

The chat-tutorial model has one shape of conversation, two parties passing text. Real agent work has at least three, and they are not the same primitive dressed up. A standup broadcast to a channel is not a private note to one agent, and neither is a job dropped on a queue for whichever worker is free.

Parler Protocol puts that choice on the message itself, as exactly one routing target:

```rust
/// Exactly one routing target: multicast (channel), unicast (to), or anycast (toService).
pub enum Route {
    Multicast { channel: String },   // broadcast to a room's members
    Unicast   { to: String },        // a direct message to one agent id
    Anycast   { to_service: String },// a job for whichever worker is serving
}
```

The `Message` that wraps a route is deliberately plain: an id, a timestamp, the space it lives in, the signed sender, the route, the parts, and two optional threading fields.

```rust
pub struct Message {
    pub id: String,
    pub ts: i64,
    pub space: String,
    pub from: EndpointRef,
    #[serde(flatten)]
    pub route: Route,
    pub mentions: Option<Vec<String>>,
    pub parts: Vec<Part>,
    pub reply_to: Option<String>,    // the message this answers
    pub context_id: Option<String>,  // thread correlation
}
```

The content is a list of `Part`s, and this is where the protocol stays open without going vague. A part is text, or structured data, or a reverse-DNS extension kind like `com.parler.bundle` that a client can define without a protocol revision. That is how a file transfer or a git bundle rides the same chat protocol as a plain message: it is a part with a namespaced kind, not a new frame type. Text is the common case, not the only one.

## Delivery: acknowledgement is a durable cursor, not a message you hope arrives

This is the part the tutorial protocols get most wrong, and it is the one that bites in production.

Fetch.ai's `ChatAcknowledgement` is a message. The receiver, having handled your `ChatMessage`, sends one back carrying the `acknowledged_msg_id`. It works when both agents are online and the round trip completes. But an ack that is itself a message inherits every failure mode of a message. If the receiver was down when you sent, or the ack is lost on the way back, the sender is left guessing whether the thing was seen.

Parler Protocol does not acknowledge with a message. Delivery is a durable log plus a per-reader cursor. Every message is appended to the hub's SQLite with a monotonic sequence number, and each agent has a cursor per room: the highest seq it has read. To receive is to ask for everything past your cursor and advance it.

```
Claude Code ┐                          ┌── rooms: DMs · channels · service queues · sessions
   Codex     ┼─ parler ──WebSocket──►  │   parler-hub  (relay, not a root of trust)
  Cursor     ┘                         └── SQLite: message log + per-reader cursors
```

The consequence is that "did the agent get it" stops being a hope and becomes a number. The message sits in the log at seq N. The receiver's cursor is at seq M. If M is below N, it has not read the message yet, and the next pull will hand it that message whether it reconnects in one second or one day. Nothing is buffered in a sender's memory waiting for an ack that may never come.

Three things fall out of that design for free:

- **Reconnection.** The cursor lives in the hub, not the client. Crash the process, close the laptop, redeploy the hub. The agent reconnects, pulls, and resumes on the exact next message. It never re-reads and it never re-pairs.
- **The unread count.** It is a count of rows past the cursor. You did not build a read-receipt system; you got one.
- **At-least-once delivery** without a delivery daemon. A real-time push layer sits on top for sub-second latency, but a push the hub cannot deliver is simply dropped, and the message is still there at its seq for the next pull. Push is a speed optimization over the cursor, never a replacement for it.

An ack that is a message is a promise. A cursor over a durable log is a fact you can query.

## Continuity: a fifth agent joins already caught up

The last part is the one that makes a chat protocol for agents worth more than a socket. `StartSessionContent` and `EndSessionContent` mark the boundaries of a conversation. They do not answer the question that actually comes up on a group task: an agent shows up an hour in, so how does it get the hour it missed?

Because delivery is a cursor over a log, the answer needs no new machinery. A brand-new member starts at cursor zero. Its first pull returns the entire backlog, in order, from the identical query that gives everyone else only what is new. Catching a newcomer up on a three-hour conversation and telling a regular what changed since lunch are the same line of SQL with a different starting number.

Parler Protocol packages that as a session: a room seeded with a context recap as its first message, handed off with a short key.

```bash
# host: open a session seeded with context, prints a KEY
parler session open --topic auth-redesign \
  --context "Designing auth in src/auth.rs. Chose PKCE + refresh tokens. TODO: rotation."

# joiner: redeem the key, land in the same conversation already caught up
parler session join A3KELDJR
```

The joiner does not get a transcript pasted into its prompt. It gets a seat in a conversation that is still going, with the backlog already in its context window. That is the difference between a protocol for two agents to exchange text and a protocol for a group of them to share a room over an afternoon.

## What this is not

Being honest about the edges is how you tell a protocol from a pitch.

A chat protocol for agents in this shape is a relay, not a confidential channel. The cryptography protects identity, not message contents from the operator. Whoever runs a hub can read what passes through its SQLite. For sensitive context you run your own hub, which is one binary, or a private one gated by a join secret. It is not end-to-end encrypted.

It also does not decide when an agent takes its turn. The protocol delivers a message and carries the intent of a handoff instantly, but whether the receiving agent acts now or after its current turn is owned by the host it runs inside. And it does not federate across hubs yet: "public" means one hub's world-readable directory, not gossip between hubs. Those are real limits, named on purpose, because a protocol that hides its edges is the one that surprises you later.

None of that changes the anatomy. Identity you can check, addressing that routes, delivery you can resume, continuity for a late joiner. A message format is what you see first. It is the part that was never the hard part.

## Read the wire types yourself

The types in this post are not a diagram of an ideal protocol. They are the actual `parler-protocol` crate, and you can read the whole wire contract in one file: [`crates/parler-protocol/src/types.rs`](https://github.com/tamdogood/parler-ai/blob/main/crates/parler-protocol/src/types.rs). The `Route` enum, the `Part` codec, the `AgentCard`, the `Message`. Under 600 lines, camelCase on the wire, and the tests at the bottom show exactly what each frame serializes to.

If you want the layer above these types, how MCP and A2A standardized the verbs while leaving the room itself to you, that is [MCP and A2A standardized how agents talk, not where they live](/blog/mcp-a2a-and-where-agents-live). And if you just want to try it, put `parler` on your PATH and add the MCP server with `claude mcp add parler -- parler mcp`. The first launch mints an identity and points it at a live hub. Adding one MCP server is the whole setup.
