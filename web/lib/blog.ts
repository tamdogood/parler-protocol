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
    slug: "how-agents-hand-off-code",
    title: "How AI agents hand each other code, not just words",
    dek: "Two agents can talk about a change all day. Handing over the change itself, byte for byte, is a different problem. Here is how Parler moves a git bundle between agents as a content-addressed blob over the socket they already chat on, so the receiver ends up with the exact commits and nothing gets reconstructed from a description.",
    date: "2026-07-03",
    dateLabel: "July 3, 2026",
    readingTime: "10 min read",
    author: "Tam Nguyen",
    tags: ["Code handoff", "Git bundles", "Multi-agent", "Rust", "Content-addressed"],
    cover: "/blog/code-handoff.svg",
  },
  {
    slug: "share-your-agent-context-with-your-team",
    title: "Share your coding agent's context with your teammates",
    dek: "Multi-agent guides assume one person running several agents. At a hackathon or on a group project it's the opposite: several people, each with their own agent, on one repo. Here's how to share a live session with one key — no pasted transcripts — so a teammate's agent joins the same conversation already caught up.",
    date: "2026-07-02",
    dateLabel: "July 2, 2026",
    readingTime: "9 min read",
    author: "Tam Nguyen",
    tags: ["Multi-agent", "Collaboration", "Hackathon", "Sessions", "MCP"],
    cover: "/blog/share-session.svg",
  },
  {
    slug: "bugs-that-hid-until-production",
    title: "The bugs that hid until production: building a multi-agent hub in Rust",
    dek: "A WebSocket that passed every localhost test and died the moment it spoke TLS. A private hub that was not private. An invite that walked past its own approval gate. A crash loop that heated up a MacBook. Five debugging stories from shipping Parler, the chat protocol for AI agents, in one Rust binary.",
    date: "2026-07-02",
    dateLabel: "July 2, 2026",
    readingTime: "12 min read",
    author: "Tam Nguyen",
    tags: ["Rust", "Debugging", "TLS", "SQLite", "Multi-agent"],
    cover: "/blog/war-stories.svg",
  },
  {
    slug: "ai-agent-memory-in-2026",
    title: "AI agent memory in 2026 is mostly single-player",
    dek: "A field guide to the year agent memory grew up: the taxonomy, the benchmarks, sleep-time consolidation, temporal knowledge graphs. Almost all of it assumes one agent and one user. Here is the shared-memory problem Parler was built for, with real code.",
    date: "2026-07-01",
    dateLabel: "July 1, 2026",
    readingTime: "13 min read",
    author: "Tam Nguyen",
    tags: ["Agent memory", "Multi-agent", "Episodic memory", "Knowledge graphs", "2026"],
    cover: "/blog/agent-memory-2026.svg",
  },
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
