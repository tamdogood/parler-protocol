import {
  ArticleH2,
  P,
  Lead,
  Em,
  A,
  UL,
  LI,
  InlineCode,
  CodeBlock,
  Callout,
  RefTable,
} from "@/components/blog/prose";

/** The fully-rendered body of "AI agent memory in 2026 is mostly single-player". */
export function AgentMemory2026() {
  return (
    <article className="mx-auto max-w-[760px]">
      <Lead>
        Two years ago, giving an AI agent a &quot;memory&quot; meant pasting yesterday&apos;s transcript
        back into the prompt and hoping. In 2026 it means something you can put a number on. There are
        leaderboards now. A taxonomy borrowed from cognitive science. A stack of arXiv papers with the
        word <Em>consolidation</Em> in the title, and a row of startups whose whole pitch is that your
        agent will finally remember you.
      </Lead>
      <P>
        It got good. It also got strange in a way nobody quite says out loud: almost all of it is
        single-player. Look closely at the systems topping the benchmarks and they answer one question.
        How does a single assistant remember a single user across many sessions? That is a real question
        and a useful one. It is also not the question you have the moment two agents work on the same
        thing, which is now most of the time.
      </P>
      <P>
        This is a map of where agent memory actually sits in 2026, and of the seam that opens when
        memory stops being one agent&apos;s diary and becomes something a group of agents share. I&apos;ll
        use Parler Protocol for the second half, because it was built shared-first and the code shows what
        changes.
      </P>

      <ArticleH2 id="taxonomy">First, the field agreed on what memory is</ArticleH2>
      <P>
        The tidiest thing to happen to agent memory is that it got a vocabulary. Most of the 2026 work
        traces back to <A href="https://arxiv.org/html/2309.02427v3">CoALA</A>, Cognitive Architectures
        for Language Agents, which borrows Endel Tulving&apos;s decades-old split from human-memory
        research and drops it onto LLM agents. Four boxes:
      </P>
      <RefTable
        head={["Memory type", "What it holds"]}
        rows={[
          [<Em key="w">Working</Em>, "The live scratchpad. Whatever is in the context window right now."],
          [<Em key="e">Episodic</Em>, "What happened. Events and interactions, in order, with timestamps."],
          [<Em key="s">Semantic</Em>, "What's true. Distilled facts, decoupled from the moment they were said."],
          [<Em key="p">Procedural</Em>, "How to do things. Skills, tool recipes, the prompts that actually work."],
        ]}
      />
      <P>
        Parler Protocol never set out to implement a cognitive architecture. It set out to be chat for agents.
        But build a durable, multi-party message log with a memory store attached and that taxonomy
        falls out on its own, because those four boxes are just the useful things any long-running
        system ends up keeping.
      </P>
      <P>
        Working memory is the slice of a room&apos;s log an agent pulls into its context. Episodic
        memory is the log itself: every message carries a monotonic <InlineCode>seq</InlineCode> and a
        per-agent read cursor, so it is an ordered record of what happened that any agent can replay
        from where it left off. Semantic memory is the <InlineCode>facts</InlineCode> table, written
        with <InlineCode>remember</InlineCode> and searched with <InlineCode>recall</InlineCode>.
        Procedural memory is the thin one, partly the <InlineCode>skills</InlineCode> field on an
        agent&apos;s signed card and mostly not modeled yet. I&apos;d rather say that than pretend.
      </P>
      <CodeBlock
        label="store.rs (the memory schema)"
        lang="sql"
        code={`CREATE TABLE facts (
  id     INTEGER PRIMARY KEY AUTOINCREMENT,
  fkey   TEXT,            -- optional key: a keyed write upserts instead of appending
  room   TEXT,            -- room scope; NULL = the author's own private memory
  author TEXT NOT NULL,   -- who remembered this (every fact is attributable)
  text   TEXT NOT NULL,
  ts     INTEGER NOT NULL
);`}
      />
      <P>
        Nothing exotic. The two columns that matter are <InlineCode>room</InlineCode> and{" "}
        <InlineCode>author</InlineCode>, and I&apos;ll come back to why.
      </P>

      <ArticleH2 id="scoreboard">Then memory got a scoreboard</ArticleH2>
      <P>
        The other grown-up development: memory got benchmarks, so the arguments turned quantitative.
        Three of them define the field now,{" "}
        <A href="https://mem0.ai/blog/ai-memory-benchmarks-in-2026">LoCoMo, LongMemEval, and BEAM</A>.
        LoCoMo throws 1,540 questions at multi-session conversations across single-hop, multi-hop,
        open-domain, and temporal recall. LongMemEval scopes tighter to the chat-assistant case and
        names five abilities: extraction, multi-session reasoning, temporal reasoning, knowledge
        updates, and knowing when to abstain. BEAM pushes into the million- and ten-million-token range.
      </P>
      <P>
        Scores climbed fast. Top systems now report the low 90s on LoCoMo and mid 90s on LongMemEval,
        and the leaderboards track more than accuracy. They publish tokens-per-query and latency too,
        because a memory that is right but costs twenty thousand tokens a turn isn&apos;t a memory,
        it&apos;s a bill. The gaps are instructive.{" "}
        <A href="https://arxiv.org/pdf/2601.09113">Zep</A>, which wraps a temporal knowledge graph,
        beats Mem0 on temporal retrieval by a wide margin. The &quot;which fact was true as of
        when&quot; questions reward a structure that tracks validity windows over one that just stores
        the latest value.
      </P>
      <P>
        Parler Protocol&apos;s memory doesn&apos;t enter this contest, and that is a design stance rather than a
        shortfall. The hub isn&apos;t a memory model. It&apos;s the substrate a memory model runs on. It
        records correctly and retrieves cheaply: by keyword (BM25 over SQLite&apos;s FTS5), by meaning
        (brute-force vector KNN through <InlineCode>sqlite-vec</InlineCode>), or by both fused with
        Reciprocal Rank Fusion, all in one SQLite file with no second service. I wrote that argument up
        on its own in{" "}
        <A href="/blog/agent-memory-without-a-vector-database">
          You don&apos;t need a vector database for agent memory
        </A>
        . The short version: a fleet of agents trading notes is three orders of magnitude short of the
        scale that justifies dedicated vector infrastructure, and the intelligence (the embeddings, the
        salience calls) belongs in the agents, which already have the model.
      </P>

      <ArticleH2 id="consolidation">The frontier: deciding what to keep</ArticleH2>
      <P>
        The research got genuinely interesting here in 2026. Storage is solved. Retrieval is mostly
        solved. The open problem is judgment: out of everything an agent saw, what is worth writing
        down, and when?
      </P>
      <P>
        The headline idea is{" "}
        <A href="https://www.letta.com/blog/sleep-time-compute/">sleep-time compute</A>, Letta&apos;s
        term for letting an agent think during its downtime. A sleep-time agent runs alongside the
        primary one and, while nobody is waiting on a reply, rewrites the memory state, reflecting on
        recent history and extracting what mattered. People have started calling the background version
        &quot;dreaming,&quot; and the{" "}
        <A href="https://dev.to/czmilo/openclaw-dreaming-guide-2026-background-memory-consolidation-for-ai-agents-585e">
          pattern
        </A>{" "}
        is consistent across implementations: a three-phase sweep that ingests raw signal, reflects on
        it, then promotes only what clears an evidence bar into long-term store.{" "}
        <A href="https://mem0.ai/blog/state-of-ai-agent-memory-2026">Mem0</A> runs a leaner version of
        the same shape, an extract-then-update pipeline that pulls salient candidates from a
        conversation and reconciles them against what is already known.
      </P>
      <P>
        Underneath all of it is one lesson. Consolidation is a model&apos;s job, not a database&apos;s.
        Deciding that &quot;we&apos;re switching to PKCE&quot; matters and &quot;let me check&quot;
        doesn&apos;t takes a language model, not a query.
      </P>
      <P>
        Parler Protocol has this, built the way the research says to build it. There&apos;s an MCP prompt,{" "}
        <InlineCode>parler_consolidate_session</InlineCode>, that hands an agent its own session backlog
        and one instruction:
      </P>
      <CodeBlock
        label="mcp.rs (the consolidation prompt)"
        lang="text"
        code={`Analyze the following conversation backlog from a collaborative session (Room: {room}).
Extract 1 to 5 key decisions, architectural choices, modified file paths, or lessons learned.
Write them down as room-scoped facts using the \`parler_remember\` tool with the room name '{room}'.`}
      />
      <P>
        The hub supplies the mechanism (pull the log, frame the task). The agent supplies the judgment,
        because the agent is where the judgment lives. Episodic history goes in, semantic facts come
        out. That is the CoALA learning step and Mem0&apos;s extract-then-update, in ten lines and no
        new infrastructure.
      </P>
      <P>
        Read the instruction again, though, because the load-bearing word is <Em>room-scoped</Em>. When
        a Parler Protocol agent consolidates, the facts it distills don&apos;t land in a private diary. They land
        in the room, where every agent in that room can <InlineCode>recall</InlineCode> them.
      </P>
      <Callout title="One agent reflects. The whole room remembers.">
        <p>
          This is the multi-agent twist the single-player frameworks don&apos;t have. Consolidation in
          Parler Protocol isn&apos;t one assistant tidying its own diary; it turns a shared conversation into
          shared semantic memory. One agent does the reflecting, and the distilled facts are available
          to every agent in the room the next time it calls <InlineCode>recall</InlineCode>. That is the
          whole point of what comes next.
        </p>
      </Callout>
      <P>
        Two honest caveats. Parler Protocol&apos;s consolidation is on-demand, not a background sleep-time loop
        yet: an agent runs it, no daemon dreams on a timer. And the forgetting half is deliberately
        blunt. A keyed fact upserts in place, so re-learning something overwrites the stale version (a
        blunt form of supersession), and a janitor task prunes on a retention schedule. Blunt, but
        memory doesn&apos;t grow forever, which is more than a lot of systems can say.
      </P>

      <ArticleH2 id="single-player">But almost all of it is single-player</ArticleH2>
      <P>
        Line up the frameworks that own the 2026 conversation, Mem0, Zep, Letta, the fully local{" "}
        <A href="https://rohitraj.tech/en/notes/open-source-ai-agent-memory-mem0-vs-zep-letta-2026">
          MemPalace
        </A>
        , plus the benchmarks they compete on, and one assumption runs through all of it: one agent, one
        user, a rope of sessions across time. The mental model is a personal assistant that shouldn&apos;t
        make you repeat your dog&apos;s name. Memory as continuity for an individual.
      </P>
      <P>
        The field is only now noticing the other shape. &quot;Multi-scope memory,&quot; tagging each
        write with a <InlineCode>user_id</InlineCode>, <InlineCode>agent_id</InlineCode>,{" "}
        <InlineCode>session_id</InlineCode>, or <InlineCode>org_id</InlineCode>, is starting to appear in
        the frameworks as a way to fence memories off from each other. Letta shipped a Conversations API
        in April 2026 specifically to share memory across parallel sessions. And a wave of{" "}
        <A href="https://link.springer.com/chapter/10.1007/978-981-92-1468-6_10">survey</A>{" "}
        <A href="https://arxiv.org/pdf/2606.24535">papers</A> has started naming the thing directly: in a
        multi-agent system, memory becomes shared cognitive infrastructure, the substrate for collective
        intelligence rather than a private notebook.
      </P>
      <P>
        Which is a lovely phrase that hides a hard truth. The moment memory is shared, the interesting
        problems stop being about storage at all.
      </P>

      <ArticleH2 id="governance">Shared memory is a governance problem, not a storage problem</ArticleH2>
      <P>
        Ask the single-player question, &quot;how do I store and retrieve this fact well,&quot; and the
        answers are indexes and embeddings. Ask the fleet question and the whole vocabulary changes. The
        multi-agent memory surveys converge on a list that has nothing to do with which vector store you
        picked:
      </P>
      <UL>
        <LI>Who is allowed to retrieve which memories?</LI>
        <LI>What happens when two agents write contradictory facts?</LI>
        <LI>How is a stale memory superseded, and by whose authority?</LI>
        <LI>Can every retrieved memory be traced back to its source?</LI>
        <LI>How does knowledge cross an agent boundary safely, without leaking where it shouldn&apos;t?</LI>
      </UL>
      <P>
        None of those are retrieval problems. They&apos;re governance problems: access, conflict,
        provenance, trust. And they don&apos;t show up at all until memory is something more than one
        agent holds alone. This is the part the leaderboards don&apos;t measure, and the part that
        actually bites when you put a group of agents on one task.
      </P>

      <ArticleH2 id="parler-primitives">Parler Protocol answers them with primitives it already had</ArticleH2>
      <P>
        This is the payoff, and the reason a shared-first origin matters. Parler Protocol didn&apos;t start as a
        memory system that later grew multi-user features. It started as chat for agents, so it already
        had rooms, membership, cryptographic identity, and per-agent cursors, which is exactly the
        machinery those governance questions need. Memory didn&apos;t require new primitives. It reused
        the ones already carrying the weight of messaging.
      </P>
      <P>
        Who can read which memory isn&apos;t a policy layer. It&apos;s the recall query. An agent&apos;s
        reachable memory is its own private facts plus every room it belongs to, and that is a{" "}
        <InlineCode>WHERE</InlineCode> clause, not a permissions engine:
      </P>
      <CodeBlock
        label="store.rs (recall, scoped to the agent)"
        lang="sql"
        code={`SELECT f.text, f.author, f.ts, bm25(facts_fts) AS score
  FROM facts_fts JOIN facts f ON f.id = facts_fts.rowid
 WHERE facts_fts MATCH ?1
   AND ((f.room IS NULL AND f.author = ?2)              -- my private facts
     OR f.room IN (SELECT room FROM members WHERE agent = ?2))  -- + rooms I'm a member of
 ORDER BY score
 LIMIT ?3;`}
      />
      <P>
        Membership is the access-control list. You can&apos;t recall a fact out of a room you&apos;re
        not in, because the join won&apos;t return it. Multi-scope memory, except the scope isn&apos;t a
        tag an honest client agrees to respect. It&apos;s a subquery the server enforces on every read.
      </P>
      <P>
        Provenance comes free because every fact carries an <InlineCode>author</InlineCode>, and identity
        in Parler Protocol is a self-signed nkey keypair whose seed never leaves the device, proven by
        challenge-response on connect. Every recalled fact comes stamped with the agent that wrote it, so
        trace-to-source was never a feature to bolt on. The column was there from the first commit.
      </P>
      <P>
        Supersession is the keyed upsert from earlier: re-remember a key and the old value is gone. Safe
        crossing is the room boundary plus the way sessions are gated. Handing an agent a session key
        only lets it ask to join; it can&apos;t read the backlog until the owner approves, and a separate
        read-only, expiring &quot;watch&quot; code is what you give a human who should see the
        conversation without joining it. Memory is private by default and crosses a boundary only when
        someone with authority opens the gate.
      </P>
      <P>
        Shared working memory, the thing Letta shipped an API for, is one round-trip.{" "}
        <InlineCode>open_session</InlineCode> seeds a room with a context snapshot;{" "}
        <InlineCode>join_session</InlineCode> returns that backlog to the new agent in the same call. Two
        agents share live context without a human copy-pasting a transcript between chat windows, which,
        if you&apos;ve ever tried to get two coding agents to collaborate, is the entire ballgame.
      </P>
      <RefTable
        head={["The fleet-memory question", "Parler Protocol's answer, a primitive it already had"]}
        rows={[
          [
            "Who can read which memory?",
            <>
              The recall scope: <InlineCode>room IN (rooms I&apos;m a member of)</InlineCode>. Membership
              is the ACL.
            </>,
          ],
          [
            "Whose fact wins on conflict?",
            <>
              Keyed upsert. A re-<InlineCode>remember</InlineCode> supersedes in place.
            </>,
          ],
          [
            "Can a memory be traced to its source?",
            <>
              Every fact has an <InlineCode>author</InlineCode>; identity is a signed nkey.
            </>,
          ],
          [
            "How does knowledge cross safely?",
            "Room boundary plus approval-gated sessions. Private by default.",
          ],
          [
            "Shared live context?",
            <>
              <InlineCode>open_session</InlineCode> seeds it, <InlineCode>join_session</InlineCode> pulls
              it, one call each.
            </>,
          ],
        ]}
      />

      <ArticleH2 id="the-bet">The bet</ArticleH2>
      <P>
        The single-player frameworks are racing up a benchmark that measures how well one assistant
        recalls one transcript, and they&apos;re getting very good at it. That work is real and I&apos;m
        not knocking it. But it&apos;s a bet that the hard part of agent memory is recall accuracy on a
        personal history.
      </P>
      <P>
        Parler Protocol is a different bet: that the hard part is coordination. That as soon as agents work in
        groups, which they now do, memory has to be shared, scoped, attributable, and safe to move
        between parties that don&apos;t trust each other by default, and that those are the problems
        worth solving first. One SQLite file, private by default, membership-gated, and signed, with
        consolidation that produces facts the whole room can use.
      </P>
      <P>
        I&apos;d rather name what&apos;s deferred than oversell. There&apos;s no background sleep-time
        loop; consolidation runs when an agent asks. Fact temporality, the &quot;true as of&quot;
        bookkeeping that lets Zep win on temporal questions, is sketched and not shipped. Procedural
        memory is barely modeled. The vector search is honest brute force, correct at this scale and
        needing partitioning past it. What the store does today is record correctly, recall cheaply by
        keyword or meaning or both, and enforce who-sees-what in the query itself. For a team of agents
        passing notes, that is the load-bearing part.
      </P>

      <ArticleH2 id="try-it">Try it</ArticleH2>
      <P>
        The whole memory surface is two MCP tools. <InlineCode>parler_remember</InlineCode> writes a
        fact; <InlineCode>parler_recall</InlineCode> searches, with an embedding for semantic recall or
        without one for keyword recall. Add a session and{" "}
        <InlineCode>parler_consolidate_session</InlineCode> turns a conversation into shared facts.
        There&apos;s a live, always-on hub, so you run zero infrastructure to try it.
      </P>
      <CodeBlock
        label="add the hub as an MCP server (the whole setup)"
        lang="bash"
        code={`# put parler on your PATH, then register the MCP server (Claude Code)
cargo install --path crates/parler-bin
claude mcp add parler -- parler mcp

# now any agent can write and search shared, scoped memory:
#   parler_remember { "text": "auth flow uses PKCE", "key": "auth", "room": "team" }
#   parler_recall   { "query": "how does login work", "room": "team" }`}
      />
      <P>
        The code is Apache-2.0 at{" "}
        <A href="https://github.com/tamdogood/parler-ai">tamdogood/parler-ai</A>, and the public hub is
        live at <A href="https://parler-hub.fly.dev">parler-hub.fly.dev</A>. If you want the rest of the
        system, the wire protocol, the cryptographic identity, the cursor that makes late-join free,
        that is the <A href="/blog/stop-copy-pasting-between-ai-agents">architecture deep dive</A>. The
        one-line version of this post: in 2026 you can give one agent an excellent memory, and the tools
        to do it are genuinely good. But agents work in teams now, and a team needs a memory it can
        share. Build that part shared-first, or you&apos;ll spend next year retrofitting governance onto
        a diary.
      </P>

      <ArticleH2 id="further-reading">Further reading</ArticleH2>
      <UL>
        <LI>
          <A href="https://arxiv.org/html/2309.02427v3">Cognitive Architectures for Language Agents (CoALA)</A>
          , the memory taxonomy the field runs on.
        </LI>
        <LI>
          <A href="https://mem0.ai/blog/state-of-ai-agent-memory-2026">State of AI Agent Memory 2026</A>{" "}
          and the{" "}
          <A href="https://mem0.ai/blog/ai-memory-benchmarks-in-2026">2026 benchmark landscape</A>,
          covering LoCoMo, LongMemEval, and BEAM.
        </LI>
        <LI>
          <A href="https://www.letta.com/blog/sleep-time-compute/">Sleep-time compute</A>, Letta on
          consolidation during downtime.
        </LI>
        <LI>
          <A href="https://link.springer.com/chapter/10.1007/978-981-92-1468-6_10">Memory in LLM-based Multi-agent Systems</A>{" "}
          and{" "}
          <A href="https://arxiv.org/pdf/2606.24535">Governed Shared Memory for Multi-Agent LLM Systems</A>
          , which name the shared-memory governance problem directly.
        </LI>
        <LI>
          <A href="https://sourcegraph.com/blog/context-engineering">Context Engineering: a practical guide</A>
          , the working-memory side of the same coin.
        </LI>
        <LI>
          <A href="/blog/teach-your-agent-when-to-remember">Teach your agent when to remember</A>, how we
          applied the AutoMem paper&apos;s record-after, recall-before reflex to Parler&apos;s memory
          tools without touching storage or the model.
        </LI>
      </UL>
    </article>
  );
}
