# Teach your agent when to remember, not just how

A paper came out this year with a result that should annoy anyone who has spent a month building memory infrastructure. The authors took a 32B open model, left its weights alone, left its task behavior alone, and only changed how it managed a scratch directory of memory files. On long-horizon tasks the model went from losing to competing with frontier systems. The reported jumps were 2 to 4 times. The thing they improved was not the model and not the database. It was the habit of using memory.

The paper is AutoMem, "Automated Learning of Memory as a Cognitive Skill" (Wu et al., 2026). We build Parler Protocol, the chat protocol for AI agents, and we already had the memory storage the paper takes for granted: a `remember` verb, a `recall` verb, one SQLite file. What we did not have was the habit. So we shipped the habit, and the entire change was two tool descriptions. No new storage, no protocol change, no retraining. This is what that looked like and where it stops.

## Two verbs, and nobody told the agent when to use them

Parler gives every agent two memory tools over MCP. `parler_remember` writes a fact. `parler_recall` reads facts back with full-text search, or hybrid vector search when you hand it an embedding. The storage under them is one SQLite file, which we wrote about in [why you don't need a vector database for agent memory](/blog/agent-memory-without-a-vector-database). That part works.

Here is the gap. An agent has these two tools in its list, with descriptions that told it what each button does. Nothing told it *when* to press either one. A capable model would sometimes save a decision and sometimes not. It would re-read a hundred messages of history to reconstruct a state it had already written down once, because nothing prompted it to check memory first. The tools were fine. The discipline around them was left to chance, which for an LLM means left to whatever the base model happened to do that turn.

You can throw more storage at this and it does not help. The agent that forgets to save is not short on disk. The agent that re-reads history instead of recalling is not short on an index. Both are short on a routine.

## AutoMem's real result is a routine, not an architecture

Read the AutoMem method and the interesting part is almost boring. The agent runs two small routines around every step. After acting, it runs a LOG routine that asks "what is worth recording about what just happened." Before acting, it runs a PLAN routine that asks "what do I need to recall to act now." It keeps its memory in a handful of typed files with fixed jobs: a `status` file for current state, a `strategy` file for goals, a `progress` file, a `knowledge` file for durable rules. The memory operations are first-class actions the model chooses, the same way it chooses a move in the game.

That is the whole trick. Record after a decision, recall before one, and keep the records in named buckets instead of one pile. The reported numbers for a Qwen2.5-32B agent: Crafter went from 25% to 51%, MiniHack from 7.5% to 30%, NetHack from a rounding error to almost five times that. The authors are explicit that task behavior was untouched. Only the memory scaffold changed.

The reason this travels well is that it is model-agnostic. It is not a weight update and not a datastore. It is a convention plus a reflex, which is exactly the kind of thing you can write into a prompt and have any model follow.

## We already had the actions

The part that made this cheap for us: Parler's `remember` already did everything AutoMem's file operations do. It just did them under one verb.

Call `remember` with a key and it upserts. Re-saving the same key overwrites the row filed under `(author, room, key)` instead of appending a new one. Call it without a key and it appends a fresh row every time. That is AutoMem's keyed state file and its append-only log, both, decided by whether you pass a key. Here is the branch in the hub, lightly trimmed:

```rust
let fact_id: i64 = match &fact.key {
    // With a key: upsert. Overwrite the row filed under (author, room, key).
    Some(k) => {
        let updated = conn.execute(
            "UPDATE facts SET text = ?1, ts = ?2, embedding_model = ?6
               WHERE author = ?3 AND IFNULL(room, '') = IFNULL(?4, '') AND fkey = ?5",
            params![fact.text, ts, author, fact.room, k, embedding_model],
        )?;
        if updated == 0 { /* no such key yet: INSERT a new row */ }
        else { /* fetch the id of the row we just overwrote */ }
    }
    // Without a key: append. Every call is a new row.
    None => {
        conn.execute(
            "INSERT INTO facts (fkey, room, author, text, ts, embedding_model)
             VALUES (NULL, ?1, ?2, ?3, ?4, ?5)",
            params![fact.room, author, fact.text, ts, embedding_model],
        )?;
        conn.last_insert_rowid()
    }
};
```

So the mapping to the paper was already sitting in the store. Unkeyed `remember` is append. Keyed `remember` is upsert. `recall` is search. We had the verbs the whole time. What we were missing was any signal to the agent about which one to reach for and when.

## The change was the tool copy

An MCP tool description is not documentation the model reads later. It is context, injected into the model's window on every single call, and it is the closest thing you have to standing instructions for how to use the tool. If the reflex lives anywhere, it lives there.

So the old `parler_remember` description, which just said save a fact and re-saving with a key overwrites, became this:

```
Save a fact. LOG reflex: after a decision, record what matters. Same key
overwrites in place (idempotent); omit key to append a note. Reuse a small
key vocabulary: status, strategy, progress, knowledge, session-digest.
Optionally scope to a room or embedding.
```

And `parler_recall` grew the other half of the reflex:

```
Recall saved facts. PLAN reflex: pull what you need before acting, instead
of re-reading history. BM25, or hybrid BM25 + vector KNN with an embedding.
Query a key term or free text.
```

Two things are doing work here. The reflex, LOG on write and PLAN on read, lifted straight from the paper. And a shared key vocabulary: `status`, `strategy`, `progress`, `knowledge`, plus the `session-digest` key we already used for [live sessions](/blog/share-your-agent-context-with-your-team). That vocabulary is what turns a flat pile of facts into AutoMem's typed files. It is a convention, not a schema. The `key` was always a free string, so introducing it changed no code and no wire format. It only gave the agent a small, stable set of names to file things under, which is the difference between memory you can target and memory you have to search.

If your agent files current state under `status` every turn and reads it back under `status` before it acts, you get a deterministic lookup instead of a ranked guess. We wrote about why that lookup should not be a search in [stop searching agent memory for a fact you can name](/blog/fetch-agent-memory-by-key). The key vocabulary is what makes that pattern usable by convention instead of by accident.

## A tool description is context you pay for on every call

There is a tax on this approach, and it is worth showing because it shaped the final copy. Every byte of a tool description ships to the model on every request that includes the tool. Descriptions are not free prose, they are a recurring token cost. So the repo has a test that caps the total size of the tool specs:

```rust
assert!(
    bytes <= TOOL_SPECS_BUDGET,   // 12,400 bytes
    "tool specs {bytes} B exceed budget {TOOL_SPECS_BUDGET} B, trim descriptions"
);
```

My first draft of the new descriptions was generous. It spelled out the key vocabulary three times, once in each description and once on the `key` field. The test failed immediately: 12,773 bytes against a 12,400 budget. That is the guardrail doing its job. A verbose tool description is not a better tool description, it is a slower and more expensive one, and the reflex is worth nothing if it costs more than it saves.

So I cut the redundancy. The vocabulary appears once, in the place an agent reads before it writes. The final specs came in at 12,357 bytes, back under budget with room to spare. The lesson matches the paper's: the win is discipline and structure, and structure means saying the useful thing once, not saying more things.

## What we did not do

AutoMem has a second half, and we did not ship it. After establishing the reflex and the file structure, the paper trains a small "memory specialist" model with LoRA on the agent's own good memory decisions, distilling the skill into the weights. That is a real result and it is not available to us. Parler is an MCP client. It talks to whatever model the user is running, Claude or a local Qwen or anything else, and it neither owns nor trains those weights. We can teach the reflex through context. We cannot bake it into a model we do not control. That half of the paper is out of scope by construction, and pretending otherwise would be dishonest.

There is also a loop the paper runs that we have only sketched. AutoMem uses a strong model to review whole trajectories and rewrite the memory scaffold itself, the prompts and file schemas, gated on whether the rewrite actually improves a metric. The analog for us is an offline pass where a model reads real mesh transcripts and proposes better memory conventions or tool copy, measured against a retrieval metric. We wrote it into the design notes and left it there. It is the right next step and it is not built, so it does not get to be in this post as if it were.

And the reflex itself is a nudge, not an enforcement. The tool descriptions strongly suggest when to log and when to plan. A model can still ignore them. We are not intercepting turns to force a save, and we are not going to, because a memory layer that overrides the agent's judgment is a worse memory layer. The bet is that a clear, cheap convention in the context beats silence, which is the same bet the paper made and won.

## The change is four sentences of tool copy

If you run agents on Parler, the reflex is already in front of them: the `parler_remember` and `parler_recall` descriptions now tell the model to record after a decision and recall before one, and to file state under a small set of stable keys. If you build your own agent memory, the cheaper experiment than a new datastore is to open your tool descriptions and add two sentences: when to write, and to read before you re-read history. Then measure whether your agent stops reconstructing state it already saved.

The whole change on our side is visible in one file. Read the `parler_remember` and `parler_recall` specs in `crates/parler-cli/src/mcp.rs`, and the AutoMem write-up in `docs/storage-and-memory.md` where we placed it next to Letta, Mem0, and Zep. It is a small diff for a paper that got 2 to 4 times, which is the point. The expensive part was never the storage.
