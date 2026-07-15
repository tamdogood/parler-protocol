# Real-time messaging for AI agents needs a socket, not a request

The two protocols everyone reaches for when agents need to talk, MCP and A2A, are shaped like a function call. The agent asks, the server answers, the exchange is over. That shape is right for calling a tool and wrong for the thing right next to it: a message a peer sends you that you never asked for. If an agent can only hear back on a channel it opened itself, another agent has no way to reach it. Real-time messaging is the part the request stops at.

This post is about that gap and the one design choice that closes it: the connection stays open, and the hub pushes. It is the transport layer under [what a chat protocol for agents actually needs](/blog/what-a-chat-protocol-for-agents-needs), shown with the actual frames from Parler Protocol. The interesting part is not that pushing is fast. It is that the push is allowed to fail without losing a single message, because a durable cursor sits under it.

## Real-time messaging for AI agents is a push problem, and a request can't push

Line up the models an agent already speaks. An MCP tool call is a request: the agent calls `search`, the server returns rows, done. An A2A task is a request that can stream progress back, but the initiator is still the one who opened it. Both share one property. The only party who can put bytes on the wire is the one who spoke first, and they only hear back on the socket they opened.

Now say a second agent, on the other side of the room, wants to tell yours "I finished the migration, your turn." There is no request for it to answer. Your agent is not calling anything right now. It is sitting between tool calls, or waiting on a build, or idle. A request/response API has no verb for "a message arrived that you did not ask for." The message has nowhere to land.

You can fake it by polling. Have the agent call `pull` every second and check for new messages. It works, and Parler Protocol keeps polling as a floor for exactly the case where nothing better is available. But polling once a second means up to a second of latency on every handoff and a request per second per agent per room whether or not anyone said anything. For a mesh of agents passing a task back and forth, that is the wrong default. Real-time messaging wants the message to arrive the instant it is sent, and that means the hub, not the agent, decides when to speak.

## So the socket stays open, and the hub pushes

Parler Protocol runs each agent over one long-lived WebSocket to the hub. The agent opts into push with a single frame, and from then on the hub sends messages the moment they land, unsolicited.

```rust
/// Ask the hub to **push** new room messages to this connection as [`ServerFrame::Delivery`]
/// frames (sub-second delivery), for every room the agent belongs to now or joins later. A
/// standing intent that ends when the connection closes.
Subscribe,
```

The hub acks with `Subscribed`, and after that a message from any peer arrives as a `Delivery` frame that is not a reply to anything:

```rust
/// A **pushed** room message, sent unsolicited (not in reply to any op) to a subscribed member
/// the instant a peer's [`ClientFrame::Send`] lands. It is never echoed to the message's own
/// author, and it does **not** advance the recipient's durable cursor.
Delivery {
    message: StoredMessage,
},
```

One WebSocket now carries two kinds of traffic: the replies to what the agent asked, and the pushes it did not ask for. The client has to keep those straight, because a push can land in the middle of waiting for a reply. The connector does it by buffering: while reading the reply to a request, any `Delivery` that interleaves gets set aside into an inbox instead of being mistaken for the answer.

```rust
async fn recv(&mut self) -> Result<ServerFrame> {
    while let Some(msg) = self.ws.next().await {
        match msg.map_err(|_| disconnected())? {
            WsMessage::Text(t) => match serde_json::from_str::<ServerFrame>(&t)? {
                ServerFrame::Delivery { message } => self.buffer_push(message),
                frame => return Ok(frame),
            },
            WsMessage::Close(_) => return Err(disconnected()),
            _ => continue,
        }
    }
    Err(disconnected())
}
```

That is the whole real-time path. Send lands at the hub, hub fans it out to every subscribed member as a `Delivery`, each client demultiplexes it from its reply stream. Sub-second, no polling.

## Push is best-effort. The cursor is the truth.

Here is the design decision that matters, and it is the opposite of what "real-time" usually implies. The push is allowed to be lossy. If a subscriber's socket is slow or half-closed when a message fans out, the hub drops that push and does not retry it. Read the `Subscribe` doc again, the part after the ellipsis:

> Best-effort: a push the hub can't deliver (slow/closed socket) is simply dropped. The durable per-room cursor still returns it on the next `Pull`, so push never changes the delivery guarantee, only latency.

The guarantee does not live in the push. It lives under it. Every message is appended to the hub's SQLite with a monotonic `seq`, and every agent has a per-room cursor: the highest `seq` it has read. To receive is to ask for everything past your cursor. The `Delivery` frame is a shortcut that wakes you sooner; it deliberately does not advance your cursor. You still `Pull` to advance and dedup. The push is a doorbell, not the mail.

That inversion is what makes the open socket safe. A real-time system that leans on the push being reliable has to answer "what if the push is lost" with retries, acks, and a delivery daemon. Parler Protocol answers it with "then you read it on the next pull, at its seq, in order." A dropped push costs latency, never a message. The unread count is just the rows past your cursor. You did not build a delivery-guarantee layer on top of the socket; the socket is a fast path over a guarantee that was already there.

There is even a small buffer bound on the client so a flood of pushes to an idle agent cannot grow without limit:

```rust
/// (agent idle between tool calls) accumulate here; past this we drop the oldest. Harmless by
/// design. The durable cursor re-delivers anything dropped on the next pull.
```

Dropping the oldest buffered push is fine for the same reason. The cursor still has it.

## A dropped socket is invisible: reconnect and resume

An open connection is a connection that will eventually drop. A laptop sleeps, a proxy culls an idle socket, the hub redeploys. Because the cursor and the room membership both live server-side, none of that loses state. The client treats a lost socket as one specific, recoverable error and rebuilds:

```rust
/// Rebuild the transport against the same identity + hub and restore the push subscription, so an
/// idle-timeout disconnect is invisible to the caller.
async fn reconnect(&mut self) -> Result<()> {
    let identity = self.identity.as_ref()
        .ok_or_else(|| anyhow::anyhow!("cannot reconnect without a local identity"))?;
    let client =
        HubClient::connect(&self.hub_url, identity, &self.name, self.role.as_deref()).await?;
    self.transport = Box::new(client);
    if self.subscribed {
        self.subscribed = self.transport.subscribe().await.unwrap_or(false);
    }
    Ok(())
}
```

Reconnect re-runs the nkey handshake on a fresh socket, restores the push subscription, and returns. The agent's next `Pull` picks up exactly where the old socket left off, because the cursor never moved. It does not re-read what it already saw and it does not re-pair. The hub even closes idle authenticated sockets on purpose, after 30 minutes by default (`DEFAULT_IDLE_TIMEOUT_SECS = 1800`), precisely because reconnect is cheap and holding thousands of dead sockets open is not.

The one hole this opens is a send that was in flight when the socket dropped. Did it land before the drop, or not? Retrying blind would double-post. So a `Send` can carry a `client_id`, an idempotency key the sender reuses on the retry, and the hub enforces `(room, author, client_id)` unique:

```rust
/// `client_id` is an optional idempotency key the sender generates once per logical send and
/// reuses on a transparent retry-after-reconnect: the hub enforces `(room, author, client_id)`
/// unique, so a retry whose first attempt already landed returns the original message's id/seq
/// instead of double-posting.
```

Send, lose the socket, reconnect, retry with the same key. If the first attempt landed, you get its `seq` back instead of a duplicate. The reconnect is invisible to the code above it, which is the point.

## Long-poll: real-time without holding a subscription

Not every agent host can sit in a loop reading pushes off a socket. An MCP tool call has to return. So there is a second real-time path that needs no standing subscription: a `Pull` with `wait_secs` becomes a long-poll. When the backlog is empty the hub parks the request instead of answering empty, and completes it the instant a message lands, or when the timer fires.

```rust
/// `wait_secs` turns this into a **long-poll**: when the backlog is empty the hub parks the
/// request (bounded ≤ 60s, counted as connection activity) and completes it the moment a message
/// lands in `room`, or the timer fires (an empty `Pulled`).
```

A `parler_recv` with a wait budget gets a message within milliseconds of it being sent, from one request, without a push subscription. The client keeps the socket alive under a long wait by splitting the budget into 25-second chunks and sending a `Ping` between them, so a half-open connection is caught within one interval rather than hanging for the full wait. The chunk is jittered by 0 to 5 seconds per agent so a fleet that all start waiting at once does not beat in lockstep against a shared proxy:

```rust
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(25);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
```

Push and long-poll are the same guarantee seen from two angles. Both wake the agent fast; neither is the thing that makes delivery reliable. That is still the cursor.

## What this is not

Real-time here means low latency, not a service-level guarantee. The push is explicitly best-effort, so "sub-second" is the common case, not a promise the protocol will keep under a slow socket. If you need a hard latency bound, this is the wrong layer to look for it in.

It is also not a message queue with consumer groups. A service room lets many-to-one work, a job dropped for whichever worker is free, but there is no partitioned consumer offset management, no exactly-once, no dead-letter queue. It is a chat transport with a durable log, not Kafka. If you are reaching for consumer-group semantics, reach for a broker.

And the socket does not decide when the agent acts. The `Delivery` frame arrives instantly; whether the receiving agent handles it now or after its current turn is owned by the host it runs inside, not by the protocol. The wire carries the message in real time. Turn-taking is a layer above it.

Finally, this all stays inside one hub. Push and long-poll both operate within a hub; there is no gossip between hubs and no federation yet. And the hub is a relay it can read: the cryptography protects who sent a message, not its contents from whoever runs the SQLite. For private context you run your own hub, which is one binary.

## Read the frames yourself

The push path is three frames and a buffer. `Subscribe`, `Subscribed`, `Delivery`, and an inbox that keeps them out of the reply stream. The long-poll path is one field, `wait_secs`, on the `Pull` you already send. All of it is in one file, [`crates/parler-protocol/src/hub.rs`](https://github.com/tamdogood/parler-protocol/blob/main/crates/parler-protocol/src/hub.rs), next to the durable cursor that makes the whole thing safe to lose.

If you want the four guarantees this transport carries, identity, addressing, delivery, and continuity, that is [what a chat protocol for agents actually needs](/blog/what-a-chat-protocol-for-agents-needs). If you want why MCP and A2A left this layer to you in the first place, that is [MCP and A2A standardized how agents talk, not where they live](/blog/mcp-a2a-and-where-agents-live). To watch push drive a visible turn, run `parler connect`, then open `parler conversation` in Claude Code, Codex, or OpenCode and join it from a second host with the printed `KEY@HUB` command. Send a message from one agent and the other sees it in its normal UI before you can switch windows.
