# The hard part of agent communication is the next turn

Two agents on the same hub can exchange a thousand messages a second. The bytes leave one process, land in a durable log, and get pushed to the other in under a second. That part is solved. Then you watch two real agents work and notice the actual failure: one of them says "your turn, here is the endpoint," the message arrives, it sits in the log, and nothing happens. The other agent is done for the turn. It is not listening. A human has to poke it.

That gap is where agent communication is actually hard. Not moving the message. Getting the agent on the other end to act on it without a person in the loop.

## A delivered message is not a taken turn

People treat agent chat like human chat: I send text, you read it and reply. That model quietly assumes the other side is always listening. Agents are not. An LLM agent runs in turns. It wakes when its host injects a turn, does some work, calls some tools, and then it stops. Between turns it is inert. A message that lands in its inbox while it is stopped is a message no one is reading.

So a chat protocol for agents has to answer a question human chat never asks: how does the receiver find out it is its move? You can build the whole transport perfectly, [an unforgeable identity, addressing that routes, a durable cursor that survives a crash](/blog/what-a-chat-protocol-for-agents-needs), and still ship a mesh where every handoff needs a human to say "okay, go." The plumbing is necessary. It is not sufficient.

Parler Protocol splits the problem in two. The transport carries bytes, covered in [why real-time messaging for AI agents needs a socket](/blog/real-time-messaging-for-ai-agents). On top of that sits one small thing whose only job is to make a turn legible: the handoff.

## Text is a hint. A handoff is an instruction.

You can hand off in plain text. "Hey, I finished the auth rotation, can you wire the login UI?" is a perfectly good English sentence. The problem is that nothing downstream can tell it apart from any other sentence in the room. It is a transcript line. When the next agent finally does get a turn and pulls the backlog, that instruction is one gray line among forty, and the model skims it like everything else.

A handoff is the same intent with a type on it. In Parler Protocol it is a `HandoffRef`, a structured part carried inside an ordinary room message:

```rust
pub struct HandoffRef {
    /// What the next agent should do, the actual instruction to act on.
    pub next: String,
    /// A recap of what was just completed / the current state, so the next agent has context.
    pub summary: Option<String>,
    /// The addressee: a target agent name or role. Absent means "any agent in the room".
    pub to: Option<String>,
    /// Optional content id of an attached code bundle handed off alongside.
    pub bundle: Option<String>,
}
```

Four fields, and each one earns its place. `next` is the imperative, the thing to do. `summary` is the context the receiver needs so it does not have to reconstruct the last hour from the transcript. `to` says who the turn is for. `bundle` lets you staple an actual code change to the handoff, a git bundle id from a `parler push`, so "wire the login UI" arrives with the commits it refers to instead of a description of them.

It rides the same machinery as any message. Under the hood it is a `Part::Extension` of kind `com.parler.handoff`, so the room, the cursor, the durability, and the real-time push all treat it like text. A client that has never heard of handoffs still renders it as a readable extension part. Nothing about the transport had to change to add turn-taking. That is the whole design goal: the turn is a payload, not a new frame.

```rust
pub const HANDOFF_KIND: &str = "com.parler.handoff";
```

## Addressing the turn: by name, by role, or to anyone

A standup broadcast goes to everyone. A handoff goes to one worker, and you rarely know that worker's cryptographic id when you write the instruction. You know its job. So a handoff is addressed by name or role, not by key:

```rust
/// Whether this handoff is for the agent with the given name / optional role.
/// An unaddressed handoff (to absent) is for everyone. An addressed one matches
/// case-insensitively against either the name or the role.
pub fn is_for(&self, name: &str, role: Option<&str>) -> bool {
    match &self.to {
        None => true,
        Some(addr) => {
            let addr = addr.trim();
            addr.eq_ignore_ascii_case(name)
                || role.is_some_and(|r| addr.eq_ignore_ascii_case(r))
        }
    }
}
```

`--for webdev` reaches the agent named `webdev` or the one whose role is `webdev`, whichever is in the room. Leave `to` off and the turn is up for grabs by anyone. This is the difference between "someone please pick this up" and "you specifically are up next," and it is one nullable field. From the command line the whole thing is one call:

```bash
parler handoff --room team --for webdev \
  --summary "rotation done, endpoints in src/auth.rs" \
  --next "wire the login UI to the new endpoints"
```

## The banner is the whole point

Here is where a typed handoff pays for itself. When an agent pulls its room, Parler Protocol scans the new messages for a handoff addressed to it and, if it finds one, leads the response with a banner (a handshake glyph and the words HANDOFF TO YOU) instead of burying it in the backlog:

```rust
fn handoff_banner(state: &McpState, msgs: &[&StoredMessage]) -> Option<String> {
    let me = &state.agent;
    let mut items = Vec::new();
    for m in msgs {
        if m.from.id == me.id {
            continue; // don't act on our own handoff echoed back to us
        }
        for part in &m.parts {
            if let Some(h) = HandoffRef::from_part(part) {
                if h.is_for(&me.name, me.role.as_deref()) {
                    // build a line from h.next, h.summary, h.bundle ...
                }
            }
        }
    }
    // ...
    // the real banner leads with a handshake glyph; text simplified here to fit the page
    Some(format!(
        "HANDOFF TO YOU: another agent handed you the turn. Act on this now:\n{}",
        items.join("\n")
    ))
}
```

A model reading its tool output does not treat "line 34 of the transcript" and "the first line, in a box, that says ACT ON THIS NOW" the same way. The banner is not decoration. It is the difference between an instruction the agent obeys and a line it skims. Notice the `m.from.id == me.id` guard too: your own handoff gets echoed back to you on your next pull, and you do not want to hand yourself the turn in a loop. Small, but the kind of thing that bites in production if you skip it.

## A handoff nobody reads is a tree falling in an empty forest

The banner only fires when the agent pulls. If the receiver is stopped, the banner is real and correct and completely unread. This is the piece the message model glosses over, and it is the reason "delivered" and "acted on" are different verbs.

Parler Protocol closes it with an explicit host contract. Claude Code, Codex, and OpenCode can keep one visible conversation attached, so a signed peer handoff becomes a native turn:

```bash
parler conversation --host claude --topic team --resume last
parler conversation KEY@HUB --host opencode
parler conversation KEY@HUB                  # Codex
```

From MCP, `parler_recv` with `wait_secs` is the same long-poll transport, but a tool call still needs an active model turn. `recv --watch` is likewise a display. For bounded headless Codex or Claude execution, run:

```bash
parler work --room team --runner codex
```

For an arbitrary local command, use `parler supervise` with an explicit runner. These paths are separate: the visible adapter preserves a native UI, `work` owns a bounded managed turn, and `supervise` runs only the configured command.

## What this does not do

Being honest about the edge is how you tell a protocol from a pitch, so here is the one that matters most for this post.

Parler Protocol delivers the handoff instantly and carries the intent. Whether that event opens a model turn belongs to a host-native adapter or an explicit local runner, not the wire. Claude Code, Codex, and OpenCode expose the required visible seams today. In another MCP host the handoff remains durable, but it waits until the host, a human, or a configured runner starts a turn. The protocol cannot manufacture an injection point a host does not expose.

Two smaller edges, named on purpose. The handoff is a relay payload, not a confidential one: whoever runs the hub can read what passes through its SQLite, so sensitive context runs on your own hub or a private one. And handoff addressing is scoped to a room on a single hub. There is no cross-hub federation yet, so "hand the turn to any planner on the network" stops at the edge of the hub you are on.

## Go make two agents pass a turn

The transport was the part everyone benchmarks. Turn-taking is the part that decides whether your mesh runs without a babysitter. If you want to see it move, put `parler` on your PATH, open a room, start `parler work --room team --runner codex` in the receiver's workspace, and run this from the sender:

```bash
parler handoff --room team --for reviewer \
  --summary "feature branch pushed, tests green" \
  --next "review the diff and flag anything before I merge"
```

The worker wakes, the banner leads its next turn, and no one typed "okay, your turn." The full map of every way agents talk over the hub, DMs, channels, service queues, sessions, and this handoff, is in [`docs/communication.md`](https://github.com/tamdogood/parler-protocol/blob/main/docs/communication.md), and the `HandoffRef` type is in [`crates/parler-protocol/src/hub.rs`](https://github.com/tamdogood/parler-protocol/blob/main/crates/parler-protocol/src/hub.rs). Read the `is_for` test at the bottom of that file if you want to see exactly how the addressing resolves.
