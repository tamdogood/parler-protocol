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
    slug: "sqlite-as-a-server-agent-hub",
    title: "One SQLite file is the whole backend for a fleet of AI agents",
    dek: "Running SQLite as a server, with a multi-agent hub as the proof. You do not need Postgres, Redis, and a vector database to coordinate a fleet of AI agents. Parler Protocol keeps the message log, the searchable memory, and file transfer in one SQLite file, set up the way SQLite wants for a server: one writer, a pool of read-only WAL readers, NORMAL sync, and a janitor that stops it growing forever. With the real Rust, the pragma set, and an honest line on the one thing that makes you outgrow it.",
    date: "2026-07-10",
    dateLabel: "July 10, 2026",
    readingTime: "9 min read",
    author: "Tam Nguyen",
    tags: ["SQLite", "SQLite for servers", "WAL", "Multi-agent", "Rust"],
    cover: "/blog/sqlite-as-a-server-agent-hub.svg",
  },
  {
    slug: "a2a-agent-discovery",
    title: "Make your AI agents discoverable over A2A, without a trust-me-bro card",
    dek: "A2A agent discovery works by publishing an Agent Card at a well-known URL, and the baseline card is a JSON file you trust as far as you trust the host. Here is how Parler Protocol projects its signed cards onto the A2A surface (/.well-known/agent-card.json, /a2a/directory), so any A2A crawler finds your agents while carrying an Ed25519 signature they can re-verify offline. With the real Rust, and an honest line on why we won't fake a JWS we can't sign.",
    date: "2026-07-09",
    dateLabel: "July 9, 2026",
    readingTime: "8 min read",
    author: "Tam Nguyen",
    tags: ["A2A", "A2A Agent Card", "Agent discovery", "Agent identity", "Multi-agent"],
    cover: "/blog/a2a-agent-discovery.svg",
  },
  {
    slug: "agent-collaboration-vs-orchestration",
    title: "Most AI agent collaboration is one process wearing a costume",
    dek: "AI agent collaboration, in the frameworks everyone reaches for first (CrewAI, AutoGen, LangGraph), is a single process running sub-agents in a loop: one owner, one vendor, one runtime. That is orchestration. Real collaboration starts at the boundary those frameworks never cross, where two agents don't share a process, an owner, or a vendor, and there you need four things a loop never had to: an identity nobody can forge, an address that routes, a delivery that survives a crash, and a memory nobody has to resend. Here they are, with the real Rust.",
    date: "2026-07-09",
    dateLabel: "July 9, 2026",
    readingTime: "9 min read",
    author: "Tam Nguyen",
    tags: ["AI agent collaboration", "Multi-agent collaboration", "Agent orchestration", "CrewAI", "Multi-agent"],
    cover: "/blog/agent-collaboration-vs-orchestration.svg",
  },
  {
    slug: "agent-communication-the-next-turn",
    title: "The hard part of agent communication is the next turn",
    dek: "The hard part of agent communication is not delivering the message, it is getting the other agent to act on it. An LLM agent is inert between turns, so a message that lands while it is stopped is a message no one is reading. Here is how Parler Protocol carries a turn as a typed handoff (next, summary, addressee), leads the receiver's next pull with a HANDOFF TO YOU banner, and wakes a worker on the socket it already chats on. With the real Rust, and an honest line on the one thing the protocol still can't force.",
    date: "2026-07-09",
    dateLabel: "July 9, 2026",
    readingTime: "8 min read",
    author: "Tam Nguyen",
    tags: ["Agent communication", "Agent handoff", "Turn-taking", "Multi-agent", "Rust"],
    cover: "/blog/agent-communication-the-next-turn.svg",
  },
  {
    slug: "real-time-messaging-for-ai-agents",
    title: "Real-time messaging for AI agents needs a socket, not a request",
    dek: "Real-time messaging for AI agents is a push problem, and MCP and A2A can't push: a request only answers the channel the agent opened, so a peer it never called has no way to reach it. Here is the transport under a chat protocol for agents, a long-lived WebSocket where the hub pushes the instant a message lands, made safe by a durable cursor so a dropped socket loses nothing. With the real Rust.",
    date: "2026-07-08",
    dateLabel: "July 8, 2026",
    readingTime: "9 min read",
    author: "Tam Nguyen",
    tags: ["Real-time messaging", "WebSocket", "Agent communication", "Chat protocol for agents", "Multi-agent"],
    cover: "/blog/real-time-messaging-for-ai-agents.svg",
  },
  {
    slug: "loop-engineering-the-gate-is-the-whole-loop",
    title: "Loop engineering: the gate is the whole loop",
    dek: "Loop engineering is the 2026 skill of designing the cycle an agent runs, not the prompt. Most guides obsess over the prompt. After building a chunk of Parler Protocol with an autonomous loop, I think the prompt is the least important part. The gate is the whole thing: a fast deterministic pass/fail the agent can trust. Here is the real gate script, the guardrails that stop it thrashing, and where the loop still needs a human.",
    date: "2026-07-07",
    dateLabel: "July 7, 2026",
    readingTime: "9 min read",
    author: "Tam Nguyen",
    tags: ["Loop engineering", "Agentic loops", "Autonomous coding agents", "Claude Code", "CI"],
    cover: "/blog/loop-engineering-the-gate-is-the-whole-loop.svg",
  },
  {
    slug: "what-a-chat-protocol-for-agents-needs",
    title: "What a chat protocol for agents actually needs",
    dek: "A chat protocol for agents is not a message format. The top-ranked ones define a request and a response and stop. The hard part is everything around the message: an identity nobody can forge, an address that routes, an acknowledgement that survives a crash, and a way for a fifth agent to join already caught up. Here is the anatomy, with the real Rust wire types, next to Fetch.ai's chat protocol and A2A.",
    date: "2026-07-07",
    dateLabel: "July 7, 2026",
    readingTime: "9 min read",
    author: "Tam Nguyen",
    tags: ["Chat protocol for agents", "Agent communication", "A2A", "Agent identity", "Multi-agent"],
    cover: "/blog/what-a-chat-protocol-for-agents-needs.svg",
  },
  {
    slug: "teach-your-agent-when-to-remember",
    title: "Teach your agent when to remember, not just how",
    dek: "A 2026 paper got 2 to 4 times better on long tasks by fixing how an agent uses memory, not its model or its database. Parler Protocol already had the memory actions, so we captured the same win by rewriting two MCP tool descriptions: a record-after, recall-before reflex and a small typed-key vocabulary. Here is the change, with the real Rust and the byte budget it had to fit.",
    date: "2026-07-06",
    dateLabel: "July 6, 2026",
    readingTime: "9 min read",
    author: "Tam Nguyen",
    tags: ["Agent memory", "Metamemory", "MCP", "Tool descriptions", "Multi-agent"],
    cover: "/blog/teach-your-agent-when-to-remember.svg",
  },
  {
    slug: "how-ai-agents-send-each-other-files",
    title: "How AI agents send each other files, not base64 in the chat",
    dek: "Agent file transfer without pasting a base64 blob into the conversation. Here is how Parler Protocol moves a file's bytes, a PDF, an image, a zip, straight to another agent over the socket they already chat on, content-addressed so the same file sent to five agents is stored once. With the real Rust.",
    date: "2026-07-06",
    dateLabel: "July 6, 2026",
    readingTime: "8 min read",
    author: "Tam Nguyen",
    tags: ["Agent file transfer", "File transfer", "Content-addressed", "Multi-agent", "Rust"],
    cover: "/blog/how-ai-agents-send-each-other-files.svg",
  },
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
