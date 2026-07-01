import { ImageResponse } from "next/og";
import { POSTS, getPost } from "@/lib/blog";
import { SITE_NAME, SITE_TAGLINE } from "@/lib/seo";
import { OgMark } from "@/lib/og-mark";

export const alt = `${SITE_NAME} — Blog`;
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

// Prerender one card per post at build time (mirrors the page's params) instead of on demand,
// so a crawler always gets a cached image and never a cold render.
export function generateStaticParams() {
  return POSTS.map((p) => ({ slug: p.slug }));
}

// Per-post branded 1200×630 social card. The in-page cover illustrations vary in aspect ratio
// (1.14–2.36:1) and weigh up to ~400 KB, so they crop unpredictably as share images and can bury
// the headline; this renders the post title on-brand at the exact OG size instead. Matches the
// root card (next/og default font — no build-time font fetch).
export default async function Image({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;
  const post = getPost(slug);
  const title = post?.title ?? SITE_NAME;
  const dek = post?.dek ?? SITE_TAGLINE;

  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          display: "flex",
          flexDirection: "column",
          justifyContent: "space-between",
          padding: "96px",
          background: "#000000",
          backgroundImage:
            "radial-gradient(900px 500px at 50% -10%, rgba(59,158,255,0.18), transparent 70%), radial-gradient(700px 500px at 90% 0%, rgba(146,129,247,0.14), transparent 70%)",
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "16px",
            color: "#3b9eff",
            fontSize: 30,
            letterSpacing: "0.04em",
          }}
        >
          <OgMark size={44} />
          {SITE_NAME.toUpperCase()} · BLOG
        </div>
        <div style={{ display: "flex", flexDirection: "column" }}>
          <div
            style={{
              fontSize: 64,
              fontWeight: 700,
              lineHeight: 1.05,
              letterSpacing: "-0.03em",
              color: "#ffffff",
              maxWidth: 1000,
            }}
          >
            {title}
          </div>
          <div
            style={{
              marginTop: 28,
              fontSize: 28,
              lineHeight: 1.4,
              color: "#a1a4a5",
              maxWidth: 940,
              display: "-webkit-box",
              WebkitLineClamp: 3,
              WebkitBoxOrient: "vertical",
              overflow: "hidden",
            }}
          >
            {dek}
          </div>
        </div>
      </div>
    ),
    { ...size },
  );
}
