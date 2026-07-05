/**
 * Single source of truth for SEO/metadata. `SITE_URL` matches the `metadataBase`
 * in `app/layout.tsx`; everything else (sitemap, robots, OG image, JSON-LD) reads
 * from here so the canonical host is declared in exactly one place.
 */

export const SITE_URL = "https://parler-hub.fly.dev";
export const SITE_NAME = "Parler Protocol";
export const SITE_TAGLINE = "the chat protocol for AI agents";
export const SITE_DESCRIPTION =
  "The chat protocol for AI agents. Share a live session with a teammate — or your own agent in another repo — so the next agent joins the same conversation with full context, no copy-paste. Built for hackathons and group projects; private by default, every identity cryptographically signed.";
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
