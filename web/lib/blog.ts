/**
 * Blog post registry. Metadata lives here; each post's fully-rendered body is a
 * component in `components/blog/<slug>.tsx`, wired up in `app/blog/[slug]/page.tsx`.
 */

export type BlogMeta = {
  slug: string;
  title: string;
  /** One-line standfirst shown under the title and on the index card. */
  dek: string;
  /** ISO date (for <time> + sorting). */
  date: string;
  /** Human label, e.g. "June 28, 2026". */
  dateLabel: string;
  readingTime: string;
  author: string;
  tags: string[];
  /** Cover image served from /public. */
  cover: string;
};

export const POSTS: BlogMeta[] = [
  {
    slug: "mcp-a2a-and-where-agents-live",
    title: "MCP and A2A standardized how agents talk. Not where they live.",
    dek: "2026 gave AI agents two great protocols: MCP for calling tools, A2A for delegating tasks. Neither gives a fleet of agents a persistent place to meet, prove who they are, and remember. Here is how Parler builds that room in one Rust binary, and why it rides the standards instead of fighting them.",
    date: "2026-07-01",
    dateLabel: "July 1, 2026",
    readingTime: "13 min read",
    author: "Tam Nguyen",
    tags: ["MCP", "A2A", "Agent interoperability", "Multi-agent"],
    cover: "/blog/architecture.png",
  },
  {
    slug: "agent-memory-without-a-vector-database",
    title: "You don't need a vector database for agent memory",
    dek: "How Parler gives a fleet of AI agents shared, searchable memory in one SQLite file: BM25 full-text search by default, semantic vector recall when you want it, and no second service to run.",
    date: "2026-06-29",
    dateLabel: "June 29, 2026",
    readingTime: "10 min read",
    author: "Tam Nguyen",
    tags: ["Agent memory", "SQLite", "Vector search", "RAG"],
    cover: "/blog/agent-memory.png",
  },
  {
    slug: "stop-copy-pasting-between-ai-agents",
    title: "Stop copy-pasting between your AI agents",
    dek: "A heavy-technical tour of Parler: the chat protocol for AI agents, in one Rust binary and an embedded SQLite log.",
    date: "2026-06-28",
    dateLabel: "June 28, 2026",
    readingTime: "12 min read",
    author: "Tam Nguyen",
    tags: ["Architecture", "Rust", "Multi-agent", "Deep dive"],
    cover: "/blog/hero.png",
  },
];

/** Newest first. */
export const postsByDate = [...POSTS].sort((a, b) => b.date.localeCompare(a.date));

export function getPost(slug: string): BlogMeta | undefined {
  return POSTS.find((p) => p.slug === slug);
}
