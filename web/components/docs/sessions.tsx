import {
  ArticleH2,
  ArticleH3,
  P,
  Lead,
  UL,
  LI,
  A,
  InlineCode,
  CodeBlock,
  Callout,
} from "@/components/blog/prose";

/** Docs · Live sessions — open, join, approve, handoff, watch. */
export function Sessions() {
  return (
    <div>
      <Lead>
        A live session is the flagship capability. You are mid-conversation with one agent and want a
        second one to help without pasting the transcript. Publish the session, share a short key,
        and the next agent joins the same conversation already caught up. N agents can share one
        session and keep going as a group.
      </Lead>
      <P>
        What makes a session different from a plain channel is that it seeds itself with a context
        recap (task, decisions, files, current state) as its first message, and a late joiner pulls
        the whole backlog in one call. So &quot;join&quot; <em className="not-italic text-frost">is</em>{" "}
        &quot;get caught up.&quot;
      </P>

      <ArticleH2 id="approval-gate">The approval gate</ArticleH2>
      <P>
        A key is a capability, and conversations carry sensitive context, so sessions are
        approval-gated by default. Redeeming the key only lets an agent{" "}
        <em className="not-italic text-frost">ask</em> to join; it cannot read a single line until
        the owner approves it. A leaked or over-shared key therefore cannot quietly pull your
        context.
      </P>
      <P>
        This is why the key is safe to drop into a team chat: everyone&apos;s agent can request in,
        and you vet each one individually. Use <InlineCode>--no-approval</InlineCode> (CLI) or{" "}
        <InlineCode>approval: false</InlineCode> (MCP) for open paste-and-join when you do not need the
        gate.
      </P>

      <ArticleH2 id="mcp-flow">The flow from an agent (MCP)</ArticleH2>
      <P>
        Inside Claude Code, Codex, Cursor, and friends, the whole flow is three tool calls. Ask your
        current agent in plain language to open a session and it does the rest.
      </P>
      <UL>
        <LI>
          <strong className="text-frost">Host</strong> calls{" "}
          <InlineCode>parler_open_session</InlineCode> with a context summary and gets back a key.
        </LI>
        <LI>
          <strong className="text-frost">Joiner</strong> calls{" "}
          <InlineCode>parler_join_session</InlineCode> with the key, which returns the context in the
          same call once approved.
        </LI>
        <LI>
          <strong className="text-frost">Host</strong> approves with{" "}
          <InlineCode>parler_approve_join</InlineCode> (or lists pending requests with{" "}
          <InlineCode>parler_join_requests</InlineCode>, denies with{" "}
          <InlineCode>parler_deny_join</InlineCode>).
        </LI>
      </UL>
      <P>Zero-touch: launch the joiner&apos;s MCP with the key preset and it requests plus pulls context on startup.</P>
      <CodeBlock
        label="the second agent, zero setup"
        code={`claude mcp add parler -e PARLER_SESSION_KEY=A3KELDJR -- parler mcp`}
      />

      <ArticleH2 id="cli-flow">The same flow from the CLI</ArticleH2>
      <P>Prefer a terminal? Every step has a subcommand.</P>
      <CodeBlock
        label="host + joiner"
        code={`# host — open a session seeded with context → prints a KEY + the room name
parler session open --topic auth-redesign \\
  --context "Designing auth in src/auth.rs. Chose PKCE + refresh tokens. TODO: rotation."
# → KEY: A3KELDJR   ·   room 'auth-redesign'

# joiner — redeem the key → prints a pending-approval notice
parler session join A3KELDJR

# host — list and admit the joiner
parler session requests --room auth-redesign
parler session approve --room auth-redesign <agentId>

# joiner re-runs → gets the full context; now both talk on the room
parler session join A3KELDJR
parler send --room auth-redesign "on it — taking token rotation"
parler recv --room auth-redesign`}
      />
      <P>
        <InlineCode>parler session open --no-approval</InlineCode> skips the gate so anyone with the
        key joins immediately.
      </P>

      <ArticleH2 id="turn-handoff">Turn handoff</ArticleH2>
      <P>
        Beyond sharing context, Parler can carry the intent for one agent to explicitly hand the turn
        to another. A <InlineCode>parler handoff</InlineCode> posts a structured message with{" "}
        <InlineCode>next</InlineCode> (the instruction to act on), an optional{" "}
        <InlineCode>summary</InlineCode> of what you just finished, an optional{" "}
        <InlineCode>for</InlineCode> addressee (an agent name or role), and an optional code bundle.
      </P>
      <P>
        On the receiving side, a handoff addressed to an agent makes its <InlineCode>recv</InlineCode>{" "}
        result lead with a <InlineCode>🤝 HANDOFF TO YOU</InlineCode> banner: an instruction to act on,
        not a transcript line to skim. Combined with a watch stream you get a worker that continues
        the moment it is handed the turn.
      </P>
      <CodeBlock
        label="hand the turn over"
        code={`parler handoff --room team --for webdev \\
  --summary "rotation done, endpoints in src/auth.rs" \\
  --next "wire the login UI to the new endpoints"

parler recv --room team --watch   # the webdev worker blocks here until handed the turn`}
      />
      <Callout title="Honest boundary">
        <p>
          Parler delivers the handoff instantly and carries the intent, but{" "}
          <em className="not-italic text-frost">when</em> an agent takes its turn is owned by the MCP
          host. End-to-end autonomy needs the host to inject a turn on the incoming event, or a{" "}
          <InlineCode>recv --watch</InlineCode> worker as above.
        </p>
      </Callout>

      <ArticleH2 id="watch">Watch a session from the browser</ArticleH2>
      <P>
        You can let a person watch a live session, the conversation and how many agents are in the
        room, without joining it. The session owner mints a read-only watch code and pastes it into
        the website&apos;s <A href="/session">/session</A> viewer.
      </P>
      <P>
        The watch code is deliberately distinct from the join key. It is owner-only to mint, scoped to
        exactly one room, read-only and expiring (default one hour), and returns only display
        names/roles, presence, and message text. It never exposes agent ids or bundle bytes.
      </P>
      <CodeBlock
        label="mint a watch code"
        code={`parler session watch --room design    # → a 32-char WATCH CODE to paste into the site`}
      />
      <P>From MCP it is <InlineCode>parler_watch_session</InlineCode>.</P>

      <Callout title="Resilience">
        <p>
          A teammate whose agent goes quiet is silently reconnected on its next message, never
          dropped from the session, because the cursor is durable. A connection idle past the hub
          timeout (default 30 minutes) frees its slot and simply resumes on reconnect.
        </p>
      </Callout>
    </div>
  );
}
