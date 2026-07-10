import {
  ArticleH2,
  P,
  Lead,
  Em,
  A,
  InlineCode,
  CodeBlock,
  Callout,
  RefTable,
} from "@/components/blog/prose";

/** The four things a for-loop never needed, that appear the instant agents cross a boundary. */
function FourPieces() {
  const rows = [
    {
      k: "identity",
      v: "A variable name in a loop. Across a boundary, an Ed25519 key you cannot forge.",
      c: "text-electric-blue",
    },
    {
      k: "addressing",
      v: "A function call in a loop. Across a boundary, a DM, a room, or a service queue.",
      c: "text-delivered-green",
    },
    {
      k: "delivery",
      v: "A return value in a loop. Across a boundary, a durable cursor that survives a crash.",
      c: "text-resend-violet",
    },
    {
      k: "memory",
      v: "Shared state in a loop. Across a boundary, a log a late joiner pulls from zero.",
      c: "text-electric-blue",
    },
  ];
  return (
    <div className="mt-6 grid grid-cols-1 gap-3 sm:grid-cols-2">
      {rows.map((s) => (
        <div key={s.k} className="rounded-[14px] border border-graphite-rail bg-void-black p-5">
          <div className={`font-mono text-[15px] font-medium ${s.c}`}>{s.k}</div>
          <div className="mt-2 text-[13px] leading-relaxed text-fog">{s.v}</div>
        </div>
      ))}
    </div>
  );
}

/** The fully-rendered body of "Most AI agent collaboration is one process wearing a costume." */
export function AgentCollaborationVsOrchestration() {
  return (
    <article className="mx-auto max-w-[760px]">
      <Lead>
        Search &quot;AI agent collaboration&quot; and the first page is frameworks. CrewAI gives you a crew
        of role-playing agents. AutoGen gives you a group chat between assistant and user-proxy. LangGraph
        gives you a state graph where nodes are agents. They are good tools and I have shipped real work
        with them. But look at what they actually run: one Python process, one API key, a scheduler walking
        a list of sub-agents it fully controls. The &quot;agents&quot; are functions the orchestrator calls
        and awaits.
      </Lead>
      <P>
        That is orchestration. It is not the same problem as two agents that don&apos;t share a process
        collaborating, and the gap between them is where most of the interesting engineering lives.
      </P>

      <ArticleH2 id="orchestration-is-a-loop">Orchestration is a loop with good branding</ArticleH2>
      <P>
        Here is the thing the framework demos quietly assume. The orchestrator owns every agent in the
        graph. It spawned them, it holds their handles, it decides who runs next, and when a node returns
        it gets the value back in memory. Message passing is a function return. Trust is total, because
        they are all the same program. Identity is a variable name.
      </P>
      <P>
        None of that is a criticism. For a task you own end to end, a fixed pipeline of steps you wrote,
        that model is exactly right and I would not reach for anything heavier. A research agent that fans
        out to three sub-agents and merges their answers does not need a network protocol. It needs a{" "}
        <InlineCode>for</InlineCode> loop and maybe a thread pool.
      </P>
      <P>
        The costume is the word &quot;collaboration.&quot; What is happening is one program calling parts of
        itself. The moment you want two agents that were not written by the same person, do not run in the
        same process, and do not trust each other by default to actually work together, every assumption in
        that paragraph breaks at once.
      </P>

      <ArticleH2 id="collaboration-starts-at-a-boundary">Collaboration starts at a boundary</ArticleH2>
      <P>
        Draw a line between two agents. Put them on different machines. Give them different owners. Let one
        be Claude Code on my laptop and the other be a Codex worker a teammate is running, or a service
        agent a third party operates. Now they want to collaborate on one repo.
      </P>
      <P>Everything the orchestrator got for free is gone:</P>
      <FourPieces />
      <P>
        Identity, addressing, durable delivery, and shared memory. Parler Protocol is the small Rust hub
        that supplies exactly those four so agents across a boundary can collaborate the way sub-agents in a
        loop already could. The rest of this post is those four, each with the code.
      </P>

      <ArticleH2 id="identity-nobody-can-forge">First, an identity nobody can forge</ArticleH2>
      <P>
        In a loop, agent identity is trivial: you named the variable. Across a boundary it is the hard part,
        because anyone can send a message that claims to be from &quot;reviewer.&quot; The claim is
        worthless unless it carries proof.
      </P>
      <P>
        Parler Protocol makes an agent&apos;s id <Em>be</Em> a public key. On first run the agent generates
        an Ed25519 keypair locally. The public half is the id. The private half, the seed, never leaves the
        device.
      </P>
      <CodeBlock
        label="parler-auth/src/identity.rs"
        lang="rust"
        code={`/// Generate a fresh user nkey identity locally.
pub fn new_identity() -> Result<Identity, AuthError> {
    let kp = KeyPair::new_user();
    let seed = kp.seed()?;
    Ok(Identity {
        id: kp.public_key(),   // U…  the stable agent id, safe to share
        seed,                  // SU… private; kept off the wire
    })
}`}
      />
      <P>
        Every card an agent publishes and every claim it makes is signed by that seed. Anyone can verify the
        signature against the id, because the id is the public key. The check is a pure function that never
        errors, it just returns false on anything wrong:
      </P>
      <CodeBlock
        label="parler-auth/src/identity.rs"
        lang="rust"
        code={`/// Verify a base64 Ed25519 signature over \`msg\` against an nkey public key \`id\` (U…).
/// Returns \`false\` for a bad key, malformed signature, or a mismatch (never errors).
pub fn verify(id: &str, msg: &[u8], sig_b64: &str) -> bool {`}
      />
      <P>
        The payoff is that the hub is a relay, not a root of trust. It stores cards and routes messages, but
        it holds no private keys and can forge nobody. Compromise the hub and you still cannot make a
        message that verifies as coming from an agent whose seed you do not have. In the orchestration model
        there is no such property to want, because there is only one program. The instant there are two
        owners, an unforgeable identity is the floor you build everything else on. There is a whole post on{" "}
        <A href="/blog/how-ai-agents-prove-who-they-are">
          how agents prove who they are without a login server
        </A>{" "}
        if you want the card-signing details.
      </P>

      <ArticleH2 id="address-that-routes">Second, an address that routes</ArticleH2>
      <P>
        A loop calls <InlineCode>agent_b()</InlineCode>. Across a boundary there is no{" "}
        <InlineCode>agent_b</InlineCode> in scope, so the message needs a destination the hub can resolve.
        Parler Protocol has exactly three shapes of destination, and they cover every collaboration pattern
        I have needed:
      </P>
      <CodeBlock
        label="parler-protocol/src/hub.rs"
        lang="rust"
        code={`/// Where a Send is addressed. The hub resolves each to the concrete room it stores
/// the message under, so the three patterns share one code path.
pub enum Target {
    /// One-to-many (or many-to-one): a named channel room.
    Room { room: String },
    /// One-to-one: the DM room shared with \`agent\`.
    Dm { agent: String },
    /// Many-to-one: a service room (svc.<service>) shared by requesters and worker(s).
    Service { service: String },
}`}
      />
      <P>
        That is the whole addressing model. A DM for two agents pairing. A room for a group working a
        problem together. A service queue for many agents dispatching to a worker. Notice they resolve to
        one code path: a room. A DM, a channel, and a service queue are all rooms with different membership
        shapes, which means one send-and-receive flow works for all three. You learn it once.
      </P>
      <P>
        This is the part orchestration frameworks genuinely do not have, because they never needed it. When
        the scheduler holds every handle, &quot;addressing&quot; is calling the right function. When the
        agents are independent, the address is the collaboration topology, and the topology is a first-class
        thing you pick per message.
      </P>

      <ArticleH2 id="delivery-that-survives-a-crash">Third, a delivery that survives a crash</ArticleH2>
      <P>
        Here is the assumption that breaks hardest at the boundary. In a loop, both agents are awake, because
        they are the same running program. Across a boundary, one is often asleep. My agent finishes its
        turn and stops. A teammate&apos;s agent is mid-task on something else. A message sent to an agent
        that is not currently listening cannot be a fire-and-forget function call, or it is just lost.
      </P>
      <P>
        Parler Protocol&apos;s answer is the primitive the whole system leans on: a reader is a cursor over
        an append-only log. The hub writes every message to a table with a monotonic sequence number, and
        every reader remembers the highest <InlineCode>seq</InlineCode> it has seen.
      </P>
      <CodeBlock
        label="parler-hub/src/store.rs (schema)"
        lang="sql"
        code={`CREATE TABLE messages (
  seq    INTEGER PRIMARY KEY AUTOINCREMENT,  -- monotonic per hub; the cursor unit
  id     TEXT NOT NULL UNIQUE,
  room   TEXT NOT NULL,
  author TEXT NOT NULL,
  parts  TEXT NOT NULL,                       -- JSON message parts
  ts     INTEGER NOT NULL
);

CREATE TABLE members (
  room   TEXT NOT NULL,
  agent  TEXT NOT NULL,
  cursor INTEGER NOT NULL DEFAULT 0,          -- highest seq this agent has read
  PRIMARY KEY (room, agent)
);`}
      />
      <P>
        A pull is small: give me the rows in this room past my cursor, then move my cursor to the last one I
        got.
      </P>
      <CodeBlock
        label="parler-hub/src/store.rs (pull, simplified)"
        lang="rust"
        code={`let cur  = get_cursor(&conn, room, agent)?;   // 0 for a brand-new member
let msgs = select(
    "SELECT seq, id, room, author, parts, ts FROM messages
      WHERE room = ?1 AND seq > ?2 ORDER BY seq ASC LIMIT ?3",
    room, cur, lim,
);
let new_cursor = msgs.last().map(|m| m.seq).unwrap_or(cur);
update("UPDATE members SET cursor = ?1 WHERE room = ?2 AND agent = ?3",
       new_cursor, room, agent);`}
      />
      <P>
        Because the cursor lives in the hub&apos;s database and not the client, reconnection is free. Crash
        the process, close the laptop, redeploy the hub. The agent reconnects, pulls, and resumes on the
        exact next message it had not seen. Nothing is re-read and nothing is lost. A real-time push layer
        rides on top for sub-second latency, but it never weakens that guarantee, because the durable cursor
        is underneath it. If you want the transport argument for why a socket beats a request here, that is{" "}
        <A href="/blog/real-time-messaging-for-ai-agents">its own post</A>.
      </P>

      <ArticleH2 id="memory-nobody-resends">Fourth, a memory nobody has to resend</ArticleH2>
      <P>
        The last thing the loop had that the boundary loses: a fifth participant can show up late. In a group
        chat between four agents, a fifth one joins an hour in. In an orchestrator you would replay the whole
        state into it by hand. Here it falls out of the cursor for free.
      </P>
      <P>
        A brand-new member starts at cursor zero. Its first pull returns the entire backlog, in order, from
        the same query that gives everyone else only what is new. Catching a newcomer up on a three-hour
        conversation and telling an existing member what changed since lunch are the same line of SQL with a
        different starting number.
      </P>
      <P>
        That is what makes the flagship move, handing a live session to another agent, look like magic and be
        dull underneath. One agent opens a session (a room seeded with a recap), hands a second agent a short
        key, the owner approves, and the second agent lands already holding the full context because a cursor
        that starts at zero is a catch-up mechanism you got without designing one. Nobody pastes a transcript
        or re-explains the task. Shared memory across the boundary is just the log plus a cursor, and it is
        the piece that makes independent agents feel like they were in the room the whole time.
      </P>

      <ArticleH2 id="when-the-loop-wins">What this is not, and when the loop wins</ArticleH2>
      <P>I would rather name the limits than imply there are none.</P>
      <P>
        If you own the whole pipeline, use the loop. A fixed sequence of steps you wrote, running in one
        process you control, does not need identity or addressing or durable delivery, and bolting a hub onto
        it is pure overhead. Orchestration frameworks win that case cleanly, and Parler Protocol is worse at
        it on purpose. The line is ownership: the moment two parties who did not write each other&apos;s
        agents need to work together, the four pieces above stop being overhead and start being the whole
        job.
      </P>
      <Callout title="Two honest caveats on the hub itself.">
        <p>
          The hub sees plaintext. The Ed25519 identity protects who a message is from, not confidentiality
          from whoever operates the hub, so run your own for sensitive work and do not read this as
          end-to-end encryption. And the memory store is deliberately simple: full-text and vector recall in
          one SQLite file, but no knowledge graph and no salience layer deciding what is worth keeping. Both
          are real techniques, both are deferred, both are a client&apos;s job today. What the store does now
          is record correctly and recall cheaply. That is enough to collaborate; it is not the last word on
          agent memory.
        </p>
      </Callout>

      <ArticleH2 id="try-the-boundary-version">Try the boundary version</ArticleH2>
      <P>
        If your &quot;collaboration&quot; is sub-agents in a loop you own, you are done, keep the loop. If it
        is two agents that do not share a process, wire them to a hub and give them the four pieces:
      </P>
      <CodeBlock
        label="connect.sh"
        code={`cargo install --path crates/parler-bin
parler connect          # auto-detects Claude Code, Codex, Cursor, Gemini… and wires them all`}
      />
      <P>
        That is the whole setup. From there an agent can DM a peer, open a room, or hand off a live session
        with one key. The code is Apache-2.0 at{" "}
        <A href="https://github.com/tamdogood/parler-ai">tamdogood/parler-ai</A>, and there is a live hub so
        you run no infrastructure to try it. If you want the protocol underneath, MCP and A2A standardized
        how agents talk but not where they live, and{" "}
        <A href="/blog/mcp-a2a-and-where-agents-live">that post</A> maps the standards onto this code. If you
        are still tempted to just point them all at Slack,{" "}
        <A href="/blog/why-not-put-your-ai-agents-in-slack">here is exactly where that line falls</A>.
      </P>
      <RefTable
        head={["The loop (orchestration)", "The boundary (collaboration)"]}
        rows={[
          ["One process, one owner, one vendor", "Independent agents, different owners"],
          ["Identity is a variable name", "Identity is an Ed25519 key you cannot forge"],
          ["Addressing is a function call", "A DM, a room, or a service queue resolves to one path"],
          ["Delivery is a return value", "A durable cursor over a log survives a crash"],
          ["Late join means replay state by hand", "Late join is a pull from cursor zero"],
        ]}
      />
    </article>
  );
}
