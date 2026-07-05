import { SITE_URL, SITE_NAME, RSS_URL } from "@/lib/seo";
import { postsByDate } from "@/lib/blog";

// The feed is built from static registry data, so render it once at build time.
export const dynamic = "force-static";

const CHANNEL_DESCRIPTION =
  "Engineering notes from the Parler Protocol project — architecture deep dives on coordinating AI agents over one Rust binary and an embedded SQLite log.";

/** Escape the five XML-significant characters so post titles/deks can't break the feed. */
function escapeXml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function GET() {
  const blogUrl = `${SITE_URL}/blog`;

  const items = postsByDate
    .map((post) => {
      const link = `${SITE_URL}/blog/${post.slug}`;
      const categories = post.tags
        .map((t) => `      <category>${escapeXml(t)}</category>`)
        .join("\n");
      return `    <item>
      <title>${escapeXml(post.title)}</title>
      <link>${link}</link>
      <guid isPermaLink="true">${link}</guid>
      <pubDate>${new Date(post.date).toUTCString()}</pubDate>
      <description>${escapeXml(post.dek)}</description>
${categories}
    </item>`;
    })
    .join("\n");

  const lastBuildDate = new Date(postsByDate[0]?.date ?? Date.now()).toUTCString();

  const xml = `<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">
  <channel>
    <title>${escapeXml(SITE_NAME)} — Blog</title>
    <link>${blogUrl}</link>
    <description>${escapeXml(CHANNEL_DESCRIPTION)}</description>
    <language>en-us</language>
    <lastBuildDate>${lastBuildDate}</lastBuildDate>
    <atom:link href="${RSS_URL}" rel="self" type="application/rss+xml" />
${items}
  </channel>
</rss>`;

  return new Response(xml, {
    headers: {
      "Content-Type": "application/rss+xml; charset=utf-8",
      "Cache-Control": "public, max-age=3600, s-maxage=3600",
    },
  });
}
