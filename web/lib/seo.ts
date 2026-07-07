/**
 * Single source of truth for SEO/metadata. `SITE_URL` matches the `metadataBase`
 * in `app/layout.tsx`; everything else (sitemap, robots, OG image, JSON-LD) reads
 * from here so the canonical host is declared in exactly one place.
 */

// Canonical host of the marketing site. This is NOT the hub server (parler-hub.fly.dev) that agents
// dial or that lib/api.ts reads the directory from — that lives in `PUBLIC_HUB`. Everything SEO
// (canonical, sitemap, robots, OG, JSON-LD) reads this constant, so the whole site advertises one
// canonical host and search engines don't split ranking signals.
//
// NOTE: the apex currently 308-redirects to www at the hosting layer. For canonical/sitemap URLs to
// resolve without a redirect, make the apex the primary domain (flip the host redirect to www → apex).
export const SITE_URL = "https://parlerprotocol.com";
export const SITE_NAME = "Parler Protocol";
export const SITE_TAGLINE = "the chat protocol for AI agents";
export const SITE_DESCRIPTION =
  "The chat protocol for AI agents. Hand a live session to another agent with one key — no copy-pasting transcripts — and transfer files and code between agents over the same socket. Built for hackathons and group projects; one small Rust binary, private by default, every identity cryptographically signed.";
export const AUTHOR = "Tam Nguyen";
export const GITHUB_URL = "https://github.com/tamdogood/parler-ai";
/** Where the macOS desktop app (DMG) is published — the "Download for macOS" CTA. */
export const MAC_DOWNLOAD_URL = "https://github.com/tamdogood/parler-ai/releases/latest";

/** The blog's RSS feed. Absolute so it's valid wherever it's referenced. */
export const RSS_URL = `${SITE_URL}/blog/rss.xml`;
/**
 * Feed autodiscovery. Spread into a page's `alternates.types` so Next emits
 * `<link rel="alternate" type="application/rss+xml" href=…>` in the head. Because
 * `alternates` is replaced (not deep-merged) across route segments, every page that
 * sets its own `canonical` must also re-declare this to keep the feed discoverable.
 */
export const ALT_RSS = { "application/rss+xml": RSS_URL };

export const KEYWORDS = [
  "chat protocol for agents",
  "chat protocol for AI agents",
  "agent file transfer",
  "agent file transfers",
  "send files between agents",
  "agent-to-agent file transfer",
  "A2A file transfer",
  "AI agents",
  "agent communication protocol",
  "multi-agent",
  "agent coordination",
  "share agent context",
  "collaborative AI agents",
  "multiplayer AI agents",
  "AI pair programming",
  "hackathon AI tools",
  "group project AI agents",
  "team of agents",
  "share Claude context",
  "MCP",
  "Model Context Protocol",
  "MCP server",
  "A2A",
  "Agent2Agent",
  "agent-to-agent protocol",
  "agent interoperability",
  "agent protocols",
  "agent discovery",
  "agent directory",
  "Claude Code",
  "Rust",
  "agent mesh",
  "shared agent memory",
  "agent memory",
  "vector database",
  "SQLite vector search",
  "sqlite-vec",
  "BM25 hybrid search",
  "RAG",
];

/** Structured data describing the site as a whole — injected once in the root layout. */
export const websiteJsonLd = {
  "@context": "https://schema.org",
  "@type": "WebSite",
  name: SITE_NAME,
  url: SITE_URL,
  description: SITE_DESCRIPTION,
};

/** Structured data describing Parler Protocol the product — eligible for a SoftwareApplication rich result. */
export const softwareJsonLd = {
  "@context": "https://schema.org",
  "@type": "SoftwareApplication",
  name: SITE_NAME,
  applicationCategory: "DeveloperApplication",
  operatingSystem: "Linux, macOS, Windows",
  description: SITE_DESCRIPTION,
  url: SITE_URL,
  author: { "@type": "Person", name: AUTHOR },
  license: "https://www.apache.org/licenses/LICENSE-2.0",
  sameAs: [GITHUB_URL],
  offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
};
