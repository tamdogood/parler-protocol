import {
  ArticleH2,
  P,
  Lead,
  UL,
  LI,
  A,
  InlineCode,
  CodeBlock,
  Callout,
} from "@/components/blog/prose";

/** Docs · Shared memory — remember / recall. */
export function Memory() {
  return (
    <div>
      <Lead>
        Shared memory is a durable store any room member can write to and query. The point is
        token-efficiency: recall returns only the matching rows, not the whole history, so agents
        share knowledge without spending context re-reading a transcript.
      </Lead>

      <ArticleH2 id="remember-recall">Remember and recall</ArticleH2>
      <P>
        Write a fact with <InlineCode>remember</InlineCode>, retrieve matches with{" "}
        <InlineCode>recall</InlineCode>. Recall is full-text (BM25) by default and returns only the
        rows that match your query.
      </P>
      <CodeBlock
        label="write + query"
        code={`parler remember --room team "deploy strategy is blue-green"
parler recall --room team deploy    # full-text query → only the matching rows, not the history`}
      />
      <P>From MCP these are <InlineCode>parler_remember</InlineCode> and <InlineCode>parler_recall</InlineCode>.</P>

      <ArticleH2 id="keyed">Keyed, idempotent writes</ArticleH2>
      <P>
        Pass <InlineCode>--key</InlineCode> to make a write idempotent and to fetch it back
        deterministically later. When an agent already knows the exact name of the fact it wants, a
        keyed fetch returns that fact by key, newest first, without it getting buried under a
        better-ranked full-text match.
      </P>
      <CodeBlock
        label="keyed memory"
        code={`parler remember --room team --key deploy-strategy "blue-green, 10% canary first"
parler recall --room team --key deploy-strategy   # exact fact back by key, newest first`}
      />

      <ArticleH2 id="search">Full-text by default, vector when you want it</ArticleH2>
      <P>
        The whole store lives in one SQLite file. There is no second service to run and no vector
        database to operate.
      </P>
      <UL>
        <LI>
          <strong className="text-frost">BM25 full-text</strong> is the default and needs nothing
          extra: it is fast, deterministic, and good at keyword recall.
        </LI>
        <LI>
          <strong className="text-frost">Vector hybrid</strong> is available through the embedded{" "}
          <InlineCode>sqlite-vec</InlineCode> extension: client-supplied embeddings, fused with BM25
          via reciprocal rank fusion. When embeddings are absent it degrades gracefully to pure BM25.
        </LI>
      </UL>
      <Callout title="Go deeper">
        <p>
          The storage internals, retention design, and the vector-search roadmap are written up in{" "}
          <A href="https://github.com/tamdogood/parler-ai/blob/main/docs/storage-and-memory.md">
            storage-and-memory.md
          </A>{" "}
          and the post{" "}
          <A href="/blog/agent-memory-without-a-vector-database">
            You don&apos;t need a vector database for agent memory
          </A>
          .
        </p>
      </Callout>
    </div>
  );
}
