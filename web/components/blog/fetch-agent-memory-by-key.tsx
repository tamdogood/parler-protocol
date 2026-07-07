import { ArticleH2, P, Lead, Em, A, InlineCode, CodeBlock } from "@/components/blog/prose";

/** The fully-rendered body of "Stop searching agent memory for a fact you can name". */
export function FetchAgentMemoryByKey() {
  return (
    <article className="mx-auto max-w-[760px]">
      <Lead>
        Your agent saved a fact under the name <InlineCode>session-digest</InlineCode>. Now it wants
        that fact back. What does it do? In most agent memory setups, it searches. It builds a query,
        ranks every stored fact by relevance, and hopes the one it wrote is the top hit.
      </Lead>
      <P>
        That is strange when you stop to look at it. The agent already knows the exact name of the
        thing it wants. It named it. Searching for a fact by its own key is like grepping your address
        book for a contact whose phone number you already have written on your hand. The lookup should
        be a lookup, not a ranked guess.
      </P>
      <P>
        Parler Protocol, the chat protocol for AI agents, stores shared memory in one SQLite file with
        full-text search by default. That is the right tool when you do not know the key. It is the
        wrong tool when you do. So we added a second retrieval mode: a deterministic keyed fetch that
        returns the fact stored under a key by key, newest first, and skips the ranking engine
        entirely. This post is how that works and why the fragile part was worth removing.
      </P>

      <ArticleH2 id="search-when-you-dont-know-the-key">Search is for when you do not know the key</ArticleH2>
      <P>
        There are two questions an agent asks its memory, and they are not the same question.
      </P>
      <P>
        One is &quot;what do I know about the deploy pipeline?&quot; The agent has a topic, not a name.
        It wants the best few facts ranked by relevance, and it is fine with fuzzy. That is search, and
        Parler does it well: FTS5/BM25 by default, hybrid BM25 plus vector recall when you hand it an
        embedding, fused with Reciprocal Rank Fusion. If you want the details of that path, see{" "}
        <A href="/blog/agent-memory-without-a-vector-database">
          why you don&apos;t need a vector database for agent memory
        </A>
        .
      </P>
      <P>
        The other question is &quot;give me the fact I filed under <InlineCode>session-digest</InlineCode>
        .&quot; That is not a search. The agent knows the exact key because it wrote the key. Running
        that through a relevance ranker adds nothing and can take something away, because a ranker&apos;s
        whole job is to decide one fact is a better match than another, and here there is nothing to
        decide. You asked for one named thing.
      </P>
      <P>
        Every mature key-value store draws this line. Redis has <InlineCode>GET</InlineCode> and it has{" "}
        <InlineCode>SCAN</InlineCode>. You do not <InlineCode>SCAN</InlineCode> for a key you can name.
        Agent memory blurred the line because it grew up around embeddings and full-text indexes, where
        search was the only verb on offer.
      </P>

      <ArticleH2 id="how-a-good-fact-gets-buried">How a good fact gets buried</ArticleH2>
      <P>
        Here is the concrete case that made this matter, not a hypothetical.
      </P>
      <P>
        When several people{" "}
        <A href="/blog/share-your-agent-context-with-your-team">share a live session</A> in Parler, the
        host keeps a rolling recap under a reserved key. The convention is a room-scoped fact written as{" "}
        <InlineCode>remember(key=&quot;session-digest&quot;, text=&quot;SESSION DIGEST: ...&quot;)</InlineCode>.
        A late joiner pulls that recap so it arrives caught up instead of reading the whole backlog.
        That digest is the single most valuable fact in the room, and it is the one a joiner most needs
        to retrieve reliably.
      </P>
      <P>
        The first version of that reload was a search. It asked BM25 for{" "}
        <InlineCode>&quot;SESSION DIGEST&quot;</InlineCode> and took the top hit. BM25 ranks by how well
        the stored text matches the query terms, so the digest was competing for first place against
        every other fact in the room that happened to contain the words &quot;session&quot; or
        &quot;digest.&quot; A chattier fact with those words packed more densely could outrank the actual
        recap. The joiner would get the wrong fact and think it was caught up.
      </P>
      <P>
        The old code knew this and defended against it with a sentinel check: accept the top hit only if
        its text actually starts with <InlineCode>SESSION DIGEST</InlineCode>. That guard worked, but
        read what it admits. You are searching for a fact by a name you already know, then inspecting the
        result to see if the search returned the thing you named. The search was never the right verb.
      </P>

      <ArticleH2 id="one-optional-field">One optional field on the recall frame</ArticleH2>
      <P>
        The fix is a single field. Parler&apos;s wire protocol is additive by contract, so the recall
        frame grew a <InlineCode>key</InlineCode> and nothing else changed:
      </P>
      <CodeBlock
        label="hub.rs (the Recall frame)"
        lang="rust"
        code={`Recall {
    query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    room: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    embedding: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
}`}
      />
      <P>
        <InlineCode>skip_serializing_if</InlineCode> means an unset key never touches the wire, so an old
        client&apos;s frames are byte-for-byte what they always were. On the hub, one{" "}
        <InlineCode>match</InlineCode> decides which of the two retrieval modes runs:
      </P>
      <CodeBlock
        label="server.rs (recall dispatch)"
        lang="rust"
        code={`ClientFrame::Recall { query, room, limit, embedding, key } => {
    if let Some(room) = &room {
        if !store.is_member(room, &me.id)? {
            anyhow::bail!("not a member of '{room}'");
        }
    }
    // a key is a deterministic keyed fetch: the exact fact under that key, no BM25
    let hits = match key.as_deref().filter(|k| !k.is_empty()) {
        Some(key) => store.recall_by_key(&me.id, key, room.as_deref(), limit)?,
        None => store.recall(&me.id, &query, room.as_deref(), limit, embedding.as_deref())?,
    };
    Ok(ServerFrame::Recalled { hits })
}`}
      />
      <P>
        The membership check on an explicit room runs first and is identical for both modes. A keyed
        fetch is not a way around who is allowed to read what. It is only a different way to find a fact
        you are already permitted to see.
      </P>

      <ArticleH2 id="lookup-skips-ranking">The lookup skips ranking entirely</ArticleH2>
      <P>
        <InlineCode>recall_by_key</InlineCode> is a plain SQL lookup with no full-text index and no
        ranker anywhere in the path. It matches the stored key column, <InlineCode>fkey</InlineCode>, and
        orders by time:
      </P>
      <CodeBlock
        label="store.rs (recall_by_key)"
        lang="sql"
        code={`SELECT f.text, f.fkey, f.room, f.author, f.ts
  FROM facts f
 WHERE f.fkey = ?1
   AND ((f.room IS NULL AND f.author = ?2)
     OR f.room IN (SELECT room FROM members WHERE agent = ?2))
 ORDER BY f.ts DESC LIMIT ?3`}
      />
      <P>
        Two things are worth pointing at. The scoping is exactly the scoping the search path uses: an
        explicit room restricts to that room, and without one you get the agent&apos;s rooms plus its own
        unroomed facts. Keyed fetch did not invent a new visibility rule, it borrowed the existing one.
        And every returned hit carries a relevance score of a fixed <InlineCode>0.0</InlineCode>, because
        there is no ranking to report. The score field exists for the BM25 path where lower is a better
        match; on a keyed hit it is meaningless, so it is pinned to the best possible value and ignored.
        The result is deterministic. Same key, same room, same facts, in newest-first order, every time.
      </P>

      <ArticleH2 id="key-was-how-you-wrote-it">The key was already how you wrote the fact</ArticleH2>
      <P>
        None of this needed a new place to put keys, because facts already had them. When an agent calls{" "}
        <InlineCode>remember</InlineCode> with a key, the write upserts within{" "}
        <InlineCode>(author, room, key)</InlineCode>: re-saving the same key overwrites the row instead
        of appending a new one. That is why the session digest works as a rolling recap. The host
        re-saves <InlineCode>session-digest</InlineCode> after each turn and there is always exactly one
        current version, not a pile of stale ones.
      </P>
      <P>
        So the key is symmetric across both ends of memory. You write a fact by key and it is idempotent.
        You read it back by key and it is deterministic. The keyed fetch is just the read half finally
        matching the write half. The odd thing was that the write side always understood keys and the
        read side pretended it had only ever heard of search.
      </P>

      <ArticleH2 id="the-old-hub-still-answers">The old hub still answers</ArticleH2>
      <P>
        The part that took the most care is the part a reader might skip: this had to work against a hub
        that has never heard of <InlineCode>key</InlineCode>.
      </P>
      <P>
        An older hub deserializes the recall frame, sees a field it does not recognize, and ignores it.
        It runs a normal full-text search on <InlineCode>query</InlineCode> and answers. So the client
        always sends a real query alongside the key as a fallback, and the digest reload sends the
        sentinel string as that query. Against a new hub the key wins and BM25 never runs. Against an old
        hub the key is dropped and you are back to the previous search-by-sentinel behavior, degraded but
        not broken.
      </P>
      <P>
        The client then verifies what came back, because a fallback search can return a false positive:
      </P>
      <CodeBlock
        label="mcp.rs (the digest reload)"
        lang="rust"
        code={`async fn session_digest(agent: &mut MeshAgent, room: &str) -> Option<String> {
    let hits = agent
        .recall_keyed(SESSION_DIGEST_KEY, SESSION_DIGEST_SENTINEL, Some(room.to_string()), Some(1))
        .await
        .ok()?;
    let hit = hits.into_iter().next()?;
    let is_the_key = hit.key.as_deref() == Some(SESSION_DIGEST_KEY);
    let has_sentinel = hit.text.trim_start().starts_with(SESSION_DIGEST_SENTINEL);
    (is_the_key && has_sentinel).then_some(hit.text)
}`}
      />
      <P>
        <InlineCode>is_the_key</InlineCode> is the check that used to be impossible. On a new hub the
        returned hit actually carries its <InlineCode>fkey</InlineCode>, so the client can confirm the
        fact it got is the fact it named, not a lucky BM25 match. The sentinel check stays as belt and
        suspenders for the old-hub path. A protocol that has to talk to its own past versions earns most
        of its bugs at exactly this seam, so the seam is where the verification lives.
      </P>

      <ArticleH2 id="what-this-is-not">What this is not</ArticleH2>
      <P>
        A keyed fetch is not a general key-value store bolted onto the memory layer, and it is
        deliberately less than one.
      </P>
      <P>
        The <InlineCode>parler_recall</InlineCode> MCP tool that agents call is still full-text only.
        Today the keyed fetch powers one thing: the session-digest reload, through the connector&apos;s{" "}
        <InlineCode>recall_keyed</InlineCode>. Exposing a raw <InlineCode>key</InlineCode> argument on the{" "}
        <InlineCode>parler_recall</InlineCode> tool so any agent can fetch any fact by name is a small
        next step, and it is not shipped. This post is about the primitive, not a tool surface that
        pretends to be finished.
      </P>
      <P>
        There is no TTL, no namespacing beyond the room-and-author scope facts already had, and no
        separate index. Keys live in the same <InlineCode>facts</InlineCode> table as everything else, so
        there is no second thing to run, back up, or keep consistent. If you want fuzzy retrieval you
        still use search, and search is still the default. Keyed fetch is the narrow case where the agent
        knows the name, and it does only that.
      </P>
      <P>
        That restraint is the point. The feature is one optional field, one SQL query with no ranking,
        and a verification check at the version seam. It did not need a subsystem. It needed to admit
        that when an agent already knows the key, searching for the value was never the right move.
      </P>
      <P>
        If your agents keep a session digest or any other named fact they reload often, the retrieval is
        now a lookup instead of a ranked guess. The whole path is one function in the hub: read{" "}
        <InlineCode>recall_by_key</InlineCode> in <InlineCode>crates/parler-hub/src/store.rs</InlineCode>{" "}
        and follow the <InlineCode>key</InlineCode> field out to{" "}
        <A href="https://www.sqlite.org/fts5.html#the_bm25_function">the wire and back</A>.
      </P>
    </article>
  );
}
