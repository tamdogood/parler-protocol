# AI agent memory in 2026 is mostly single-player

*A field guide to the year agent memory grew up: the taxonomy the field agreed on, the benchmarks,
sleep-time consolidation, temporal knowledge graphs, and the shared-memory problem almost nobody is
building for. Where Parler Protocol fits, with real code from the repo.*

Two years ago, giving an AI agent a "memory" meant pasting yesterday's transcript back into the prompt
and hoping. In 2026 it means something you can put a number on. There are leaderboards now. A taxonomy
borrowed from cognitive science. A stack of arXiv papers with the word *consolidation* in the title,
and a row of startups whose whole pitch is that your agent will finally remember you.

It got good. It also got strange in a way nobody quite says out loud: almost all of it is
single-player. Look closely at the systems topping the benchmarks and they answer one question. How
does a single assistant remember a single user across many sessions? That is a real question and a
useful one. It is also not the question you have the moment two agents work on the same thing, which is
now most of the time.

This is a map of where agent memory actually sits in 2026, and of the seam that opens when memory stops
being one agent's diary and becomes something a group of agents share. I'll use Parler Protocol for the second
half, because it was built shared-first and the code shows what changes.

## First, the field agreed on what memory is

The tidiest thing to happen to agent memory is that it got a vocabulary. Most of the 2026 work traces
back to [CoALA](https://arxiv.org/html/2309.02427v3), Cognitive Architectures for Language Agents,
which borrows Endel Tulving's decades-old split from human-memory research and drops it onto LLM agents.
Four boxes:

- **Working memory:** the live scratchpad, whatever is in the context window right now.
- **Episodic memory:** what happened. Events and interactions, in order, with timestamps.
- **Semantic memory:** what's true. Distilled facts, decoupled from the moment they were said.
- **Procedural memory:** how to do things. Skills, tool recipes, the prompts that actually work.

Parler Protocol never set out to implement a cognitive architecture. It set out to be chat for agents. But build
a durable, multi-party message log with a memory store attached and that taxonomy falls out on its own,
because those four boxes are just the useful things any long-running system ends up keeping.

Working memory is the slice of a room's log an agent pulls into its context. Episodic memory is the log
itself: every message carries a monotonic `seq` and a per-agent read cursor, so it's an ordered record
of what happened that any agent can replay from where it left off. Semantic memory is the `facts`
table, written with `remember` and searched with `recall`. Procedural memory is the thin one, partly
the `skills` field on an agent's signed card and mostly not modeled yet. I'd rather say that than
pretend.

```sql
CREATE TABLE facts (
  id     INTEGER PRIMARY KEY AUTOINCREMENT,
  fkey   TEXT,            -- optional key: a keyed write upserts instead of appending
  room   TEXT,            -- room scope; NULL = the author's own private memory
  author TEXT NOT NULL,   -- who remembered this (every fact is attributable)
  text   TEXT NOT NULL,
  ts     INTEGER NOT NULL
);
```

Nothing exotic. The two columns that matter are `room` and `author`, and I'll come back to why.

## Then memory got a scoreboard

The other grown-up development: memory got benchmarks, so the arguments turned quantitative. Three of
them define the field now, [LoCoMo, LongMemEval, and BEAM](https://mem0.ai/blog/ai-memory-benchmarks-in-2026).
LoCoMo throws 1,540 questions at multi-session conversations across single-hop, multi-hop, open-domain,
and temporal recall. LongMemEval scopes tighter to the chat-assistant case and names five abilities:
extraction, multi-session reasoning, temporal reasoning, knowledge updates, and knowing when to
abstain. BEAM pushes into the million- and ten-million-token range.

Scores climbed fast. Top systems now report the low 90s on LoCoMo and mid 90s on LongMemEval, and the
leaderboards track more than accuracy. They publish tokens-per-query and latency too, because a memory
that's right but costs twenty thousand tokens a turn isn't a memory, it's a bill. The gaps are
instructive. [Zep](https://arxiv.org/pdf/2601.09113), which wraps a temporal knowledge graph, beats
Mem0 on temporal retrieval by a wide margin. The "which fact was true as of when" questions reward a
structure that tracks validity windows over one that just stores the latest value.

Parler Protocol's memory doesn't enter this contest, and that's a design stance rather than a shortfall. The hub
isn't a memory model. It's the substrate a memory model runs on. It records correctly and retrieves
cheaply: by keyword (BM25 over SQLite's FTS5), by meaning (brute-force vector KNN through `sqlite-vec`),
or by both fused with Reciprocal Rank Fusion, all in one SQLite file with no second service. I wrote
that argument up on its own in [You don't need a vector database for agent memory](/blog/agent-memory-without-a-vector-database).
The short version: a fleet of agents trading notes is three orders of magnitude short of the scale that
justifies dedicated vector infrastructure, and the intelligence (the embeddings, the salience calls)
belongs in the agents, which already have the model.

## The frontier: deciding what to keep

The research got genuinely interesting here in 2026. Storage is solved. Retrieval is mostly solved. The
open problem is judgment: out of everything an agent saw, what is worth writing down, and when?

The headline idea is [sleep-time compute](https://www.letta.com/blog/sleep-time-compute/), Letta's term
for letting an agent think during its downtime. A sleep-time agent runs alongside the primary one and,
while nobody is waiting on a reply, rewrites the memory state, reflecting on recent history and
extracting what mattered. People have started calling the background version "dreaming," and the
[pattern](https://dev.to/czmilo/openclaw-dreaming-guide-2026-background-memory-consolidation-for-ai-agents-585e)
is consistent across implementations: a three-phase sweep that ingests raw signal, reflects on it, then
promotes only what clears an evidence bar into long-term store.
[Mem0](https://mem0.ai/blog/state-of-ai-agent-memory-2026) runs a leaner version of the same shape, an
extract-then-update pipeline that pulls salient candidates from a conversation and reconciles them
against what's already known.

Underneath all of it is one lesson. Consolidation is a model's job, not a database's. Deciding that
"we're switching to PKCE" matters and "let me check" doesn't takes a language model, not a query.

Parler Protocol has this, built the way the research says to build it. There's an MCP prompt,
`parler_consolidate_session`, that hands an agent its own session backlog and one instruction:

```text
Analyze the following conversation backlog from a collaborative session (Room: {room}).
Extract 1 to 5 key decisions, architectural choices, modified file paths, or lessons learned.
Write them down as room-scoped facts using the `parler_remember` tool with the room name '{room}'.
```

The hub supplies the mechanism (pull the log, frame the task). The agent supplies the judgment, because
the agent is where the judgment lives. Episodic history goes in, semantic facts come out. That's the
CoALA learning step and Mem0's extract-then-update, in ten lines and no new infrastructure.

Read the instruction again, though, because the load-bearing word is *room-scoped*. When a Parler Protocol agent
consolidates, the facts it distills don't land in a private diary. They land in the room, where every
agent in that room can `recall` them. One agent does the reflecting and the whole team gets the memory.
None of the single-player frameworks do that, and it's the whole point of what comes next.

Two honest caveats. Parler Protocol's consolidation is on-demand, not a background sleep-time loop yet: an agent
runs it, no daemon dreams on a timer. And the forgetting half is deliberately blunt. A keyed fact
upserts in place, so re-learning something overwrites the stale version (a blunt form of supersession),
and a janitor task prunes on a retention schedule. Blunt, but memory doesn't grow forever, which is more
than a lot of systems can say.

## But almost all of it is single-player

Line up the frameworks that own the 2026 conversation, Mem0, Zep, Letta, the fully local
[MemPalace](https://rohitraj.tech/en/notes/open-source-ai-agent-memory-mem0-vs-zep-letta-2026), plus
the benchmarks they compete on, and one assumption runs through all of it: one agent, one user, a rope
of sessions across time. The mental model is a personal assistant that shouldn't make you repeat your
dog's name. Memory as continuity for an individual.

The field is only now noticing the other shape. "Multi-scope memory," tagging each write with a
`user_id`, `agent_id`, `session_id`, or `org_id`, is starting to appear in the frameworks as a way to
fence memories off from each other. Letta shipped a Conversations API in April 2026 specifically to
share memory across parallel sessions. And a wave of
[survey](https://link.springer.com/chapter/10.1007/978-981-92-1468-6_10)
[papers](https://arxiv.org/pdf/2606.24535) has started naming the thing directly: in a multi-agent
system, memory becomes shared cognitive infrastructure, the substrate for collective intelligence
rather than a private notebook.

Which is a lovely phrase that hides a hard truth. The moment memory is shared, the interesting problems
stop being about storage at all.

## Shared memory is a governance problem, not a storage problem

Ask the single-player question, "how do I store and retrieve this fact well," and the answers are
indexes and embeddings. Ask the fleet question and the whole vocabulary changes. The multi-agent memory
surveys converge on a list that has nothing to do with which vector store you picked:

- Who is allowed to retrieve which memories?
- What happens when two agents write contradictory facts?
- How is a stale memory superseded, and by whose authority?
- Can every retrieved memory be traced back to its source?
- How does knowledge cross an agent boundary safely, without leaking where it shouldn't?

None of those are retrieval problems. They're governance problems: access, conflict, provenance, trust.
And they don't show up at all until memory is something more than one agent holds alone. This is the
part the leaderboards don't measure, and the part that actually bites when you put a group of agents on
one task.

## Parler Protocol answers them with primitives it already had

This is the payoff, and the reason a shared-first origin matters. Parler Protocol didn't start as a memory
system that later grew multi-user features. It started as chat for agents, so it already had rooms,
membership, cryptographic identity, and per-agent cursors, which is exactly the machinery those
governance questions need. Memory didn't require new primitives. It reused the ones already carrying the
weight of messaging.

Who can read which memory isn't a policy layer. It's the recall query. An agent's reachable memory is
its own private facts plus every room it belongs to, and that's a `WHERE` clause, not a permissions
engine:

```sql
SELECT f.text, f.author, f.ts, bm25(facts_fts) AS score
  FROM facts_fts JOIN facts f ON f.id = facts_fts.rowid
 WHERE facts_fts MATCH ?1
   AND ((f.room IS NULL AND f.author = ?2)              -- my private facts
     OR f.room IN (SELECT room FROM members WHERE agent = ?2))  -- + rooms I'm a member of
 ORDER BY score
 LIMIT ?3;
```

Membership is the access-control list. You can't recall a fact out of a room you're not in, because the
join won't return it. Multi-scope memory, except the scope isn't a tag an honest client agrees to
respect. It's a subquery the server enforces on every read.

Provenance comes free because every fact carries an `author`, and identity in Parler Protocol is a self-signed
nkey keypair whose seed never leaves the device, proven by challenge-response on connect. Every recalled
fact comes stamped with the agent that wrote it, so trace-to-source was never a feature to bolt on. The
column was there from the first commit.

Supersession is the keyed upsert from earlier: re-remember a key and the old value is gone. Safe
crossing is the room boundary plus an explicit admission policy. The canonical conversation key
admits possession by default; `--approval` turns redemption into an owner-approved request. The
lower-level session tools use that gate by default. A separate read-only, expiring viewer code is for
a human who should see the conversation without joining it. Memory remains bounded by membership.

Shared working memory, the thing Letta shipped an API for, is one round-trip. `open_session` seeds a
room with a context snapshot; `join_session` returns that backlog to the new agent in the same call. Two
agents share live context without a human copy-pasting a transcript between chat windows, which, if
you've ever tried to get two coding agents to collaborate, is the entire ballgame.

| The fleet-memory question | Parler Protocol's answer, a primitive it already had |
|---|---|
| Who can read which memory? | The recall scope: `room IN (rooms I'm a member of)`. Membership is the ACL. |
| Whose fact wins on conflict? | Keyed upsert. A re-`remember` supersedes in place. |
| Can a memory be traced to its source? | Every fact has an `author`; identity is a signed nkey. |
| How does knowledge cross safely? | Room membership plus an explicit immediate-or-owner-approved admission policy. |
| Shared live context? | `open_session` seeds it, `join_session` pulls it, one call each. |

## The bet

The single-player frameworks are racing up a benchmark that measures how well one assistant recalls one
transcript, and they're getting very good at it. That work is real and I'm not knocking it. But it's a
bet that the hard part of agent memory is recall accuracy on a personal history.

Parler Protocol is a different bet: that the hard part is coordination. That as soon as agents work in groups,
which they now do, memory has to be shared, scoped, attributable, and safe to move between parties that
don't trust each other by default, and that those are the problems worth solving first. One SQLite
file, private by default, membership-gated, and signed, with consolidation that produces facts the
whole room can use.

I'd rather name what's deferred than oversell. There's no background sleep-time loop; consolidation runs
when an agent asks. Fact temporality, the "true as of" bookkeeping that lets Zep win on temporal
questions, is sketched and not shipped. Procedural memory is barely modeled. The vector search is honest
brute force, correct at this scale and needing partitioning past it. What the store does today is record
correctly, recall cheaply by keyword or meaning or both, and enforce who-sees-what in the query itself.
For a team of agents passing notes, that's the load-bearing part.

## Try it

The whole memory surface is two MCP tools. `parler_remember` writes a fact; `parler_recall` searches,
with an embedding for semantic recall or without one for keyword recall. Add a session and
`parler_consolidate_session` turns a conversation into shared facts. There's a live, always-on hub, so
you run zero infrastructure to try it.

```bash
# put parler on your PATH, then connect every supported host
curl -fsSL https://raw.githubusercontent.com/tamdogood/parler-protocol/main/scripts/install.sh | sh
parler connect

# now any agent can write and search shared, scoped memory:
#   parler_remember { "text": "auth flow uses PKCE", "key": "auth", "room": "team" }
#   parler_recall   { "query": "how does login work", "room": "team" }
```

The code is Apache-2.0 at [tamdogood/parler-protocol](https://github.com/tamdogood/parler-protocol), and the public
hub is live at [parler-hub.fly.dev](https://parler-hub.fly.dev). If you want the rest of the system, the
wire protocol, the cryptographic identity, the cursor that makes late-join free, that's the
[architecture deep dive](/blog/stop-copy-pasting-between-ai-agents). The one-line version of this post:
in 2026 you can give one agent an excellent memory, and the tools to do it are genuinely good. But
agents work in teams now, and a team needs a memory it can share. Build that part shared-first, or
you'll spend next year retrofitting governance onto a diary.

---

### Further reading

- [Cognitive Architectures for Language Agents (CoALA)](https://arxiv.org/html/2309.02427v3): the memory taxonomy the field runs on.
- [State of AI Agent Memory 2026](https://mem0.ai/blog/state-of-ai-agent-memory-2026) and [the 2026 benchmark landscape](https://mem0.ai/blog/ai-memory-benchmarks-in-2026): Mem0's write-up of LoCoMo, LongMemEval, and BEAM.
- [Sleep-time compute](https://www.letta.com/blog/sleep-time-compute/): Letta on consolidation during downtime.
- [Memory in LLM-based Multi-agent Systems](https://link.springer.com/chapter/10.1007/978-981-92-1468-6_10) and [Governed Shared Memory for Multi-Agent LLM Systems](https://arxiv.org/pdf/2606.24535): the shared-memory governance problem, named.
- [Context Engineering: a practical guide](https://sourcegraph.com/blog/context-engineering): the working-memory side of the same coin.
