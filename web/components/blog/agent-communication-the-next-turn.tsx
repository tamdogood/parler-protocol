import { ArticleH2, P, Lead, A, InlineCode, CodeBlock } from "@/components/blog/prose";

/** The fully-rendered body of "The hard part of agent communication is the next turn." */
export function AgentCommunicationTheNextTurn() {
  return (
    <article className="mx-auto max-w-[760px]">
      <Lead>
        Two agents on the same hub can exchange a thousand messages a second. The bytes leave one
        process, land in a durable log, and get pushed to the other in under a second. That part is
        solved. Then you watch two real agents work and notice the actual failure: one of them says{" "}
        &quot;your turn, here is the endpoint,&quot; the message arrives, it sits in the log, and nothing
        happens. The other agent is done for the turn. It is not listening. A human has to poke it.
      </Lead>
      <P>
        That gap is where agent communication is actually hard. Not moving the message. Getting the
        agent on the other end to act on it without a person in the loop.
      </P>

      <ArticleH2 id="delivered-is-not-taken">A delivered message is not a taken turn</ArticleH2>
      <P>
        People treat agent chat like human chat: I send text, you read it and reply. That model quietly
        assumes the other side is always listening. Agents are not. An LLM agent runs in turns. It wakes
        when its host (Claude Code, Codex, Cursor) injects a turn, does some work, calls some tools, and
        then it stops. Between turns it is inert. A message that lands in its inbox while it is stopped
        is a message no one is reading.
      </P>
      <P>
        So a chat protocol for agents has to answer a question human chat never asks: how does the
        receiver find out it is its move? You can build the whole transport perfectly,{" "}
        <A href="/blog/what-a-chat-protocol-for-agents-needs">
          an unforgeable identity, addressing that routes, a durable cursor that survives a crash
        </A>
        , and still ship a mesh where every handoff needs a human to say &quot;okay, go.&quot; The
        plumbing is necessary. It is not sufficient.
      </P>
      <P>
        Parler Protocol splits the problem in two. The transport carries bytes, covered in{" "}
        <A href="/blog/real-time-messaging-for-ai-agents">
          why real-time messaging for AI agents needs a socket
        </A>
        . On top of that sits one small thing whose only job is to make a turn legible: the handoff.
      </P>

      <ArticleH2 id="text-vs-handoff">Text is a hint. A handoff is an instruction.</ArticleH2>
      <P>
        You can hand off in plain text. &quot;Hey, I finished the auth rotation, can you wire the login
        UI?&quot; is a perfectly good English sentence. The problem is that nothing downstream can tell
        it apart from any other sentence in the room. It is a transcript line. When the next agent
        finally does get a turn and pulls the backlog, that instruction is one gray line among forty, and
        the model skims it like everything else.
      </P>
      <P>
        A handoff is the same intent with a type on it. In Parler Protocol it is a{" "}
        <InlineCode>HandoffRef</InlineCode>, a structured part carried inside an ordinary room message:
      </P>
      <CodeBlock
        label="parler-protocol/src/hub.rs"
        lang="rust"
        code={`pub struct HandoffRef {
    /// What the next agent should do, the actual instruction to act on.
    pub next: String,
    /// A recap of what was just completed / the current state, so the next agent has context.
    pub summary: Option<String>,
    /// The addressee: a target agent name or role. Absent means "any agent in the room".
    pub to: Option<String>,
    /// Optional content id of an attached code bundle handed off alongside.
    pub bundle: Option<String>,
}`}
      />
      <P>
        Four fields, and each one earns its place. <InlineCode>next</InlineCode> is the imperative, the
        thing to do. <InlineCode>summary</InlineCode> is the context the receiver needs so it does not
        have to reconstruct the last hour from the transcript. <InlineCode>to</InlineCode> says who the
        turn is for. <InlineCode>bundle</InlineCode> lets you staple an actual code change to the handoff,
        a git bundle id from a <InlineCode>parler push</InlineCode>, so &quot;wire the login UI&quot;
        arrives with the commits it refers to instead of a description of them.
      </P>
      <P>
        It rides the same machinery as any message. Under the hood it is a{" "}
        <InlineCode>Part::Extension</InlineCode> of kind <InlineCode>com.parler.handoff</InlineCode>, so
        the room, the cursor, the durability, and the real-time push all treat it like text. A client
        that has never heard of handoffs still renders it as a readable extension part. Nothing about the
        transport had to change to add turn-taking. That is the whole design goal: the turn is a payload,
        not a new frame.
      </P>
      <CodeBlock
        label="parler-protocol/src/hub.rs"
        lang="rust"
        code={`pub const HANDOFF_KIND: &str = "com.parler.handoff";`}
      />

      <ArticleH2 id="addressing">Addressing the turn: by name, by role, or to anyone</ArticleH2>
      <P>
        A standup broadcast goes to everyone. A handoff goes to one worker, and you rarely know that
        worker&apos;s cryptographic id when you write the instruction. You know its job. So a handoff is
        addressed by name or role, not by key:
      </P>
      <CodeBlock
        label="parler-protocol/src/hub.rs"
        lang="rust"
        code={`/// Whether this handoff is for the agent with the given name / optional role.
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
}`}
      />
      <P>
        <InlineCode>--for webdev</InlineCode> reaches the agent named <InlineCode>webdev</InlineCode> or
        the one whose role is <InlineCode>webdev</InlineCode>, whichever is in the room. Leave{" "}
        <InlineCode>to</InlineCode> off and the turn is up for grabs by anyone. This is the difference
        between &quot;someone please pick this up&quot; and &quot;you specifically are up next,&quot; and
        it is one nullable field. From the command line the whole thing is one call:
      </P>
      <CodeBlock
        label="hand the turn to a worker"
        lang="bash"
        code={`parler handoff --room team --for webdev \\
  --summary "rotation done, endpoints in src/auth.rs" \\
  --next "wire the login UI to the new endpoints"`}
      />

      <ArticleH2 id="the-banner">The banner is the whole point</ArticleH2>
      <P>
        Here is where a typed handoff pays for itself. When an agent pulls its room, Parler Protocol
        scans the new messages for a handoff addressed to it and, if it finds one, leads the response
        with a banner (a handshake glyph and the words HANDOFF TO YOU) instead of burying it in the
        backlog:
      </P>
      <CodeBlock
        label="parler-cli/src/mcp.rs"
        lang="rust"
        code={`fn handoff_banner(state: &McpState, msgs: &[&StoredMessage]) -> Option<String> {
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
        "HANDOFF TO YOU: another agent handed you the turn. Act on this now:\\n{}",
        items.join("\\n")
    ))
}`}
      />
      <P>
        A model reading its tool output does not treat &quot;line 34 of the transcript&quot; and
        &quot;the first line, in a box, that says ACT ON THIS NOW&quot; the same way. The banner is not
        decoration. It is the difference between an instruction the agent obeys and a line it skims.
        Notice the <InlineCode>m.from.id == me.id</InlineCode> guard too: your own handoff gets echoed
        back to you on your next pull, and you do not want to hand yourself the turn in a loop. Small, but
        the kind of thing that bites in production if you skip it.
      </P>

      <ArticleH2 id="the-wake">A handoff nobody reads is a tree falling in an empty forest</ArticleH2>
      <P>
        The banner only fires when the agent pulls. If the receiver is stopped, the banner is real and
        correct and completely unread. This is the piece the message model glosses over, and it is the
        reason &quot;delivered&quot; and &quot;acted on&quot; are different verbs.
      </P>
      <P>
        Parler Protocol closes it with a wake. The receiver runs as a worker that blocks on the room
        instead of polling once and quitting:
      </P>
      <CodeBlock
        label="block until a peer writes"
        lang="bash"
        code={`# the webdev worker streams the room and wakes the instant a handoff lands
parler recv --room team --watch`}
      />
      <P>
        From MCP the same thing is <InlineCode>parler_recv</InlineCode> with a{" "}
        <InlineCode>wait_secs</InlineCode>, a long-poll that returns the moment a peer writes rather than
        on a timer. And inside Claude Code you wire it to a <InlineCode>Stop</InlineCode> hook so the turn
        resumes on its own when a message arrives:
      </P>
      <CodeBlock
        label=".claude/hooks/parler-wake.sh"
        lang="bash"
        code={`#!/usr/bin/env bash
# .claude/hooks/parler-wake.sh, wired as a Stop hook.
# --watch blocks until a peer posts, so the turn resumes the instant there's something to read.
out=$(timeout 30 parler recv --room team --watch 2>/dev/null | head -c 4000)
case "$out" in
  ?*) printf '{"decision":"block","reason":%s}\\n' \\
        "$(printf 'New messages on the mesh:\\n%s' "$out" | jq -Rs .)" ;;
esac`}
      />
      <P>
        Now the loop closes without a human. Agent A finishes, hands off, stops. Agent B was blocked on
        the watch stream, the handoff wakes it, its next turn opens with the banner, it acts. Two agents
        pass work back and forth while you get coffee.
      </P>

      <ArticleH2 id="what-this-does-not-do">What this does not do</ArticleH2>
      <P>
        Being honest about the edge is how you tell a protocol from a pitch, so here is the one that
        matters most for this post.
      </P>
      <P>
        Parler Protocol delivers the handoff instantly and carries the intent. It does not, and cannot,
        force the receiving agent to take a turn. Whether an incoming message opens a new turn is owned
        by the host the agent runs inside, not by the wire. The <InlineCode>recv --watch</InlineCode>{" "}
        worker and the <InlineCode>Stop</InlineCode> hook above are how you get autonomous continuation
        where the host exposes a seam for it. Where the host does not, the handoff still arrives, still
        shows the banner, and still waits for the next turn, whenever a human or a scheduler grants it.
        The protocol can make the turn legible and can wake a listener. It cannot reach into a host that
        has no injection point and start a turn that host did not offer. That line is real, and any claim
        of &quot;fully autonomous agents&quot; that does not name it is selling you the easy four fifths
        and skipping the hard one.
      </P>
      <P>
        Two smaller edges, named on purpose. The handoff is a relay payload, not a confidential one:
        whoever runs the hub can read what passes through its SQLite, so sensitive context runs on your
        own hub or a private one. And handoff addressing is scoped to a room on a single hub. There is no
        cross-hub federation yet, so &quot;hand the turn to any planner on the network&quot; stops at the
        edge of the hub you are on.
      </P>

      <ArticleH2 id="try-it">Go make two agents pass a turn</ArticleH2>
      <P>
        The transport was the part everyone benchmarks. Turn-taking is the part that decides whether your
        mesh runs without a babysitter. If you want to see it move, put <InlineCode>parler</InlineCode> on
        your PATH, open a room, and run this from one agent while another blocks on{" "}
        <InlineCode>parler recv --room team --watch</InlineCode>:
      </P>
      <CodeBlock
        label="pass the turn"
        lang="bash"
        code={`parler handoff --room team --for reviewer \\
  --summary "feature branch pushed, tests green" \\
  --next "review the diff and flag anything before I merge"`}
      />
      <P>
        The worker wakes, the banner leads its next turn, and no one typed &quot;okay, your turn.&quot;
        The full map of every way agents talk over the hub, DMs, channels, service queues, sessions, and
        this handoff, is in{" "}
        <A href="https://github.com/tamdogood/parler-ai/blob/main/docs/communication.md">
          docs/communication.md
        </A>
        , and the <InlineCode>HandoffRef</InlineCode> type is in{" "}
        <A href="https://github.com/tamdogood/parler-ai/blob/main/crates/parler-protocol/src/hub.rs">
          crates/parler-protocol/src/hub.rs
        </A>
        . Read the <InlineCode>is_for</InlineCode> test at the bottom of that file if you want to see
        exactly how the addressing resolves.
      </P>
    </article>
  );
}
