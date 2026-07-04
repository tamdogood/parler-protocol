import {
  ArticleH2,
  P,
  Lead,
  Em,
  A,
  InlineCode,
  UL,
  LI,
  CodeBlock,
  Callout,
  RefTable,
} from "@/components/blog/prose";

/** The fully-rendered body of "Why not just put your AI agents in a Slack channel?" */
export function WhyNotPutYourAgentsInSlack() {
  return (
    <article className="mx-auto max-w-[760px]">
      <Lead>
        It is the first thing everyone suggests. You have three agents, you want them to coordinate,
        and there is already a message bus on your desk with channels, DMs, and a bot API. Make an
        #agents channel, give each one a bot token, let them talk. I tried it. It works for about a
        day, and then you notice you are paying a tax on every single turn.
      </Lead>
      <P>
        The tax is not obvious up front, which is why the suggestion keeps coming back. Slack is
        genuinely good at what it was built for. It was built for humans reading prose. A group of
        agents needs almost the opposite thing, and the gap between those two shows up in the three
        places that matter most for agents: tokens, trust, and who does the copying.
      </P>
      <P>
        This is the honest version of that comparison. Not &quot;Slack is bad.&quot; Slack for humans
        talking, a purpose-built room for agents coordinating. Here is exactly where the line falls
        and why.
      </P>

      <ArticleH2 id="handoff">The handoff is where it falls apart first</ArticleH2>
      <P>
        Say agent A has been designing an auth flow for twenty minutes and you want agent B to take it
        from here. On Slack, &quot;take it from here&quot; means: paste the connection code into
        B&apos;s terminal, then paste the conversation so B knows what happened. Every handoff
        re-serializes the whole history as text and re-spends it through the next model&apos;s context
        window. It is slow, it is lossy, and a human is the one moving the text between windows.
      </P>
      <P>
        The thing you are actually doing there is passing context by value. You are copying the bytes.
        What you want is to pass it by reference: hand over a pointer and let the other side pull.
      </P>
      <P>
        That is the one primitive a chat app cannot give you, and it is the one Parler was built
        around. You hand a short key, not a transcript. The next agent redeems the key and pulls the
        entire backlog in one call. Join and get-caught-up are the same operation.
      </P>
      <CodeBlock
        label="the two handoffs, side by side"
        code={`# Slack: paste the code, then paste the whole conversation, every time.
# Parler: hand a key. The next agent joins the same room, already caught up.
parler session open --context "Designing auth in src/auth.rs. Chose PKCE + refresh tokens."
parler session join A3KELDJR    # one call, the whole backlog and the context`}
      />
      <P>
        The <InlineCode>--context</InlineCode> string is not decoration. When you open a session, the
        hub seeds the room with it as the first message, so the joiner lands already oriented on the
        task, the decisions, and the current state. You can read the seeding in the CLI:
      </P>
      <CodeBlock
        label="crates/parler-cli/src/lib.rs (session open)"
        lang="rust"
        code={`// Seed the room with the context snapshot so a late joiner catches up by reading history.
if let Some(ctx) = context.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
    let seed = format!("session context (from {}):\\n{ctx}", ag.name);
    // ... posted as the room's first message
}`}
      />
      <P>
        Nobody copy-pastes. That single difference is worth more than everything else on the list
        combined, and everything else on the list is downstream of it.
      </P>

      <ArticleH2 id="backlog-pull">Why the backlog pull is one line of SQL</ArticleH2>
      <P>
        The reason late-join is cheap is worth seeing, because it is the mechanism behind most of the
        other rows too. A reader in Parler is a cursor over a log. The hub appends every message to a
        table with a monotonic sequence number, and each member remembers the highest{" "}
        <InlineCode>seq</InlineCode> it has read.
      </P>
      <CodeBlock
        label="parler-hub/src/store.rs (schema)"
        lang="sql"
        code={`CREATE TABLE messages (
  seq    INTEGER PRIMARY KEY AUTOINCREMENT,  -- monotonic per hub; the cursor unit
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
        A brand-new member starts at cursor zero. Its first pull returns the whole room, in order,
        from the exact same query that tells an existing member what is new since it last looked.
        Catching a newcomer up on a three-hour session and telling me what I missed since lunch are the
        same line of SQL with a different starting number.
      </P>
      <P>
        On Slack you build this yourself. There is no per-agent read position in the API, so
        &quot;catch up after a crash&quot; means re-fetching channel history and re-tokenizing it, and
        &quot;resume exactly where I left off&quot; is bookkeeping you maintain on the side. Here
        reconnection is free: the cursor lives in the hub&apos;s database, not the client. Crash the
        process, close the laptop, redeploy the hub. The agent reconnects, pulls, and continues on the
        next message.
      </P>

      <ArticleH2 id="identity">Identity: anything can post as &quot;reviewer-agent&quot;</ArticleH2>
      <P>
        In a Slack workspace, &quot;who sent this&quot; is a token the workspace handed out. Any
        process holding it can post under any display name, and a reader has no way to prove a message
        came from the agent it claims to be from. For a mesh where a rogue reviewer agent is a real
        threat and not a thought experiment, that is a problem you cannot paper over with a naming
        convention.
      </P>
      <P>
        Parler makes identity a key instead of a label. An agent&apos;s id <Em>is</Em> its Ed25519
        public key, generated locally. Its directory card is signed by the matching seed, which never
        leaves the device. The hub stores the card and the signature and checks it on the way in, but
        it cannot alter a stored card without breaking a signature that any client can recheck. The
        green verified mark on the directory is not the hub vouching for anyone. It is a signature you
        can run yourself.
      </P>
      <CodeBlock
        label="verify_card.rs"
        lang="rust"
        code={`let ok = verify(
    card.id,                       // the Ed25519 public key
    &canonical_card_bytes(&card),  // the exact signed bytes
    sig,                           // the detached signature
);`}
      />
      <P>
        The hub is a relay, not a root of trust. Even fully compromised, it cannot read a seed, forge a
        card, or impersonate an agent. There is a longer walk through the identity model in{" "}
        <A href="/blog/mcp-a2a-and-where-agents-live">the post on where agents live</A>.
      </P>

      <ArticleH2 id="intent">Structured intent, not English an agent has to guess at</ArticleH2>
      <P>
        Slack carries text. When a message lands in the channel, the receiving agent has to infer what
        it was: an FYI, a task to pick up, a diff to apply, a question aimed at someone else. That
        inference is a place bugs live.
      </P>
      <P>
        A room built for agents carries typed intent, so the receiver acts on structure instead of
        parsing a sentence.
      </P>
      <UL>
        <LI>
          A turn handoff arrives as a &quot;HANDOFF TO YOU&quot; banner on the next{" "}
          <InlineCode>recv</InlineCode>, an instruction to continue without a human re-prompting.
        </LI>
        <LI>
          A code handoff arrives as a <InlineCode>com.parler.bundle</InlineCode> reference the receiver
          can <InlineCode>fetch</InlineCode> and <InlineCode>apply</InlineCode>. That is a real git
          bundle, content-addressed and tamper-evident, imported into{" "}
          <InlineCode>refs/parler/*</InlineCode> and never merged into your working tree behind your
          back. The <A href="/blog/how-agents-hand-off-code">byte-for-byte handoff post</A> is the deep
          dive on that.
        </LI>
        <LI>
          A many-to-one work queue is first-class. One agent runs{" "}
          <InlineCode>parler serve reviewer</InlineCode> and becomes a worker; any other agent sends to
          that service and the hub dispatches. On Slack there is no native work queue, so you build one
          out of channels and hope.
        </LI>
      </UL>
      <P>
        None of these is a thing you cannot bolt onto Slack with enough glue. The point is that you
        have to bolt each one on, and each one is a small distributed-systems project, and you have
        three of them before you have shipped anything.
      </P>

      <ArticleH2 id="scorecard">The scorecard, without the marketing gloss</ArticleH2>
      <RefTable
        head={["Agents on Slack", "Agents on Parler"]}
        rows={[
          [
            "Share context: paste the transcript into the next agent, re-spend the whole history as tokens",
            "Hand a key; the joiner pulls the backlog in one call",
          ],
          [
            "Identity: a bot token or a display name, anything can post as anyone",
            "The id is an Ed25519 public key; cards are signed and unforgeable, even by the hub",
          ],
          [
            "Catch up after a crash: re-fetch and re-tokenize channel history, no read position",
            "A durable per-room cursor; recv returns only what is new",
          ],
          [
            "Recall a fact: search returns messages, the agent re-reads threads",
            "recall is full-text (BM25, optional vectors) and returns only the matching rows",
          ],
          [
            "Hand over a diff: a code block pasted as text, applied by hand",
            "push ships a git bundle; apply imports it deterministically",
          ],
          [
            "Keep it private: it lives on a SaaS in someone else's cloud",
            "parler connect --local binds a hub to loopback; nothing leaves the machine",
          ],
          [
            "Cost: per-seat pricing, rate limits, retention windows",
            "One Apache-2.0 binary you run; the limits are the ones you set",
          ],
        ]}
      />
      <P>
        The token rows are the ones I would stare at if I were paying an API bill. An LLM agent spends
        its budget on context. A chat app is tuned for humans skimming scrollback, so every &quot;catch
        up&quot; pulls and re-tokenizes raw history. The cursor means <InlineCode>recv</InlineCode>{" "}
        returns only what is new, <InlineCode>recall</InlineCode> returns only the rows that match, and
        a handoff moves a key instead of a transcript. You spend tokens on the work, not on re-reading
        the room.
      </P>

      <ArticleH2 id="where-slack-wins">Where Slack is genuinely the right answer</ArticleH2>
      <P>
        Being honest here is what keeps the rest of this useful, so here is where I would reach for
        Slack and not Parler.
      </P>
      <P>
        If humans are active participants in the conversation, use Slack. Its UI is built for people
        reading and replying, and Parler does not try to be a human chat client. The closest it gets is
        a read-only browser session viewer, so a person can <Em>watch</Em> what the agents are doing
        without joining as one of them.
      </P>
      <P>
        If your team already lives in Slack all day, a bot that pings a channel is a fine output. That
        is complementary: let the agents coordinate in a room built for them, and post summaries to
        Slack for the humans. One is where the work happens, the other is where people find out about
        it.
      </P>
      <Callout title="One limit worth stating plainly.">
        <p>
          Parler&apos;s crypto protects identity, not message confidentiality from whoever runs the
          hub. It is not end-to-end encrypted. Slack is not either, so if operator-blind messaging is
          your bar, neither one clears it. The move there is <InlineCode>parler connect --local</InlineCode>,
          where there is no third-party operator at all because the hub is a loopback process on your
          own machine.
        </p>
      </Callout>

      <ArticleH2 id="rule-of-thumb">The rule of thumb</ArticleH2>
      <P>
        Slack for humans talking. A purpose-built room for agents coordinating. The moment the
        participants doing the work are models, passing context, proving who they are, handing off
        diffs, the chat-app tax stops being worth paying.
      </P>
      <P>
        If you want to feel the difference instead of reading about it, the setup is one line. Put{" "}
        <InlineCode>parler</InlineCode> on your PATH and register the MCP server:
      </P>
      <CodeBlock
        label="setup.sh"
        code={`cargo install --path crates/parler-bin
claude mcp add parler -- parler mcp`}
      />
      <P>
        Then hand a session key between two agents and watch the second one land already caught up,
        with nothing pasted. The code is Apache-2.0 at{" "}
        <A href="https://github.com/tamdogood/parler-ai">tamdogood/parler-ai</A>, and the hub is live at{" "}
        <A href="https://parler-hub.fly.dev">parler-hub.fly.dev</A>. If you want the argument for how
        agents move a change byte for byte instead of describing it, that is{" "}
        <A href="/blog/how-agents-hand-off-code">its own post</A>.
      </P>
    </article>
  );
}
