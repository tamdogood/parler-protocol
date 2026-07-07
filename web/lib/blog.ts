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
    slug: "fetch-agent-memory-by-key",
    title: "Stop searching agent memory for a fact you can name",
    dek: "Full-text and vector search are the wrong tools when an agent already knows the exact name of the fact it wants. Here is how Parler Protocol adds a deterministic keyed fetch to agent memory, so the one fact you filed under a key comes back by key, newest first, and never gets buried under a better-ranked match. With the real Rust and SQL.",
    date: "2026-07-06",
    dateLabel: "July 6, 2026",
    readingTime: "9 min read",
    author: "Tam Nguyen",
    tags: ["Agent memory", "Key-value", "Deterministic recall", "SQLite", "Multi-agent"],
    cover: "/blog/fetch-agent-memory-by-key.svg",
  },
  {
    slug: "how-ai-agents-prove-who-they-are",
    title: "How AI agents prove who they are, without a login server",
    dek: "Cryptographic agent identity, end to end: an agent's id is a keypair it generates locally, the seed never leaves the device, and every card and message is signed. Here is how Parler Protocol lets a hub route and store agent traffic without ever being able to impersonate anyone, with the real Rust code.",
    date: "2026-07-05",
    dateLabel: "July 5, 2026",
    readingTime: "9 min read",
    author: "Tam Nguyen",
    tags: ["Agent identity", "Authentication", "ed25519", "Security", "Multi-agent"],
    cover: "/blog/agent-identity.svg",
  },
  {
    slug: "why-not-put-your-ai-agents-in-slack",
    title: "Why not just put your AI agents in a Slack channel?",
    dek: "It's the first thing everyone suggests: you already have a message bus with channels and a bot API, so make an #agents channel and let them talk. It works for a day. Then you notice the tax you pay every turn, in tokens, in trust, and in the human copy-pasting transcripts. Here's exactly where the chat-app line falls for a mesh of agents, and how Parler Protocol moves it.",
    date: "2026-07-04",
    dateLabel: "July 4, 2026",
    readingTime: "9 min read",
    author: "Tam Nguyen",
    tags: ["Slack", "Multi-agent", "Agent coordination", "MCP", "Sessions"],
    cover: "/blog/agents-in-slack.svg",
  },
  {
    slug: "how-to-connect-your-ai-agents",
    title: "How to connect your AI agents in two lines",
    dek: "A hands-on guide to Parler Protocol: install once, run one command to wire every AI agent on your machine to a shared hub, then hand a live conversation to another agent with a single key instead of a pasted transcript. Every command is here, copy-paste ready.",
    date: "2026-07-03",
    dateLabel: "July 3, 2026",
    readingTime: "8 min read",
    author: "Tam Nguyen",
    tags: ["Tutorial", "Getting started", "MCP", "Multi-agent", "Sessions"],
    cover: "/blog/connect-agents.svg",
  },
  {
    slug: "how-agents-hand-off-code",
    title: "How AI agents hand each other code, not just words",
    dek: "Two agents can talk about a change all day. Handing over the change itself, byte for byte, is a different problem. Here is how Parler Protocol moves a git bundle between agents as a content-addressed blob over the socket they already chat on, so the receiver ends up with the exact commits and nothing gets reconstructed from a description.",
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
    dek: "A WebSocket that passed every localhost test and died the moment it spoke TLS. A private hub that was not private. An invite that walked past its own approval gate. A crash loop that heated up a MacBook. Five debugging stories from shipping Parler Protocol, the chat protocol for AI agents, in one Rust binary.",
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
    dek: "A field guide to the year agent memory grew up: the taxonomy, the benchmarks, sleep-time consolidation, temporal knowledge graphs. Almost all of it assumes one agent and one user. Here is the shared-memory problem Parler Protocol was built for, with real code.",
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
    dek: "2026 gave AI agents two great protocols: MCP for calling tools, A2A for delegating tasks. Neither gives a fleet of agents a persistent place to meet, prove who they are, and remember. Here is how Parler Protocol builds that room in one Rust binary, and why it rides the standards instead of fighting them.",
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
    dek: "How Parler Protocol gives a fleet of AI agents shared, searchable memory in one SQLite file: BM25 full-text search by default, semantic vector recall when you want it, and no second service to run.",
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
    dek: "A heavy-technical tour of Parler Protocol: the chat protocol for AI agents, in one Rust binary and an embedded SQLite log.",
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
