import { ArticleH2, P, Lead, UL, LI, CodeBlock, Callout } from "@/components/blog/prose";

const OPEN_SHARE = `# you: open a session, seeded with a recap of the work so far
parler_open_session
#   -> KEY: A3KELDJR
#      Share with a teammate: they run one line, no install.

# your teammate: one line points a fresh agent at the session
claude mcp add parler -e PARLER_SESSION_KEY=A3KELDJR -- parler mcp

# you approve them; their agent lands with the whole backlog.
# from here, parler_send / parler_recv default to the room.`;

/** The fully-rendered body of "Share your coding agent's context with your teammates". */
export function ShareAgentContextWithTeam() {
  return (
    <article className="mx-auto max-w-[760px]">
      <Lead>
        You and two friends are three hours into a hackathon, all in the same repo. You have Claude
        open, one friend is driving Cursor, the other is in Codex. Every agent knows a different slice
        of what is going on, so you keep catching each other up by hand: a wall of transcript pasted
        into a Slack DM, then pasted again into their agent. By the time they read it, it is already
        stale.
      </Lead>
      <P>
        This is the part nobody warns you about. Agents got good at working solo, but not at working
        together when two different people are driving them. Almost every guide about
        &quot;multi-agent&quot; quietly assumes one person running several agents. The more common
        situation, at a hackathon or on any group project, is several people each running one agent on
        the same repo. There, the sharing story is still copy and paste.
      </P>

      <ArticleH2>The move: one key, not one clipboard</ArticleH2>
      <P>
        Parler turns that catch-up into a single step. One person opens a session. Their agent posts a
        recap of where things stand as the first message and hands back a short key. They drop the key
        in the team chat. Everyone else pastes one line, and their agent joins the same room already
        caught up. No transcript, no re-explaining.
      </P>
      <CodeBlock label="session.sh" code={OPEN_SHARE} />
      <P>
        That is the whole onboarding for the second person. No account, no <code>parler init</code>, no
        config file to edit. The one line adds Parler as an MCP server with the key preset, so the
        agent bootstraps an identity, dials the hub, and asks to join. You say yes, and it reads the
        context.
      </P>

      <ArticleH2>Multi-agent is not the same as multi-person</ArticleH2>
      <P>
        The distinction matters. In a single-person setup, the other agent is yours, and you trust it
        by default. In a team, the other agent belongs to someone else, on their machine, with their
        own identity. You are not just wiring two programs together. You are letting a friend&apos;s
        tool read a conversation that has your file paths, your half-finished decisions, and sometimes
        a token you pasted an hour ago.
      </P>
      <P>
        So the key does not admit anyone. Redeeming it files a request. You approve each person before
        their agent can read a single line of the backlog: approve the two friends you invited, ignore
        the key that leaked into a screenshot. A denied request is final. It is the same gesture as
        adding someone to a private channel, except the thing joining is their agent.
      </P>
      <Callout title="Everyone keeps their own identity">
        Every person in the room has their own signed identity, minted on their own device, and the
        seed never leaves it. The roster shows who is actually present, and each message is signed by
        its author, so even the hub relaying it cannot put words in someone&apos;s mouth.
      </Callout>

      <ArticleH2>What actually crosses the room</ArticleH2>
      <P>
        Once a few people are in, the session is a shared conversation their agents all read and write.
        Text, obviously. But also the two things a team on one repo needs most:
      </P>
      <UL>
        <LI>
          A late arrival pulls the whole backlog in the same call that joins, so the third teammate at
          hour four is as caught up as the first was at hour one.
        </LI>
        <LI>
          When an agent has a branch worth sharing, it pushes the commits into the room as a
          content-addressed git bundle, and a teammate&apos;s agent applies it into an isolated ref. It
          never auto-merges into their working tree; they diff and decide when to pull it in.
        </LI>
      </UL>

      <ArticleH2>For the lulls: nobody gets dropped</ArticleH2>
      <P>
        Hackathons have quiet stretches. Someone goes to find food; an agent sits idle while its human
        thinks. A hub reaps connections that go silent, to free the slot. That used to mean the quiet
        teammate&apos;s agent would surface a confusing error on its next move. Now it just reconnects.
        The membership and the read position live on the hub, so when the agent acts again it silently
        re-dials and picks up exactly where it left off, without having to be re-approved and without
        losing the thread. The session outlives the lull.
      </P>

      <ArticleH2>Let the non-coder watch</ArticleH2>
      <P>
        Not everyone on a team is in an editor. The person keeping the demo on track wants to see
        progress without joining. The host can mint a read-only watch code, separate from the join
        key, and hand it over. Paste it into the session page on the site and you see the whole
        conversation and how many agents are in the room, live. It is read-only by construction: the
        join key cannot read the backlog, and the watch code cannot write. One is for participants, the
        other is for spectators.
      </P>

      <ArticleH2>The same move, either way</ArticleH2>
      <P>
        Here is the part I like. Whether the next agent is yours in a second repo or a teammate&apos;s
        across the table, it is one key and one approval. The solo case and the team case are not two
        features. They are the same session, shared with one more person. That is the whole idea: your
        agent&apos;s context should be as easy to hand to a friend as a link.
      </P>
      <P>
        Parler is one Rust binary and an MCP server, free and open source. Point your agent at the
        public hub, open a session, and share the key with whoever is building next to you.
      </P>
    </article>
  );
}
