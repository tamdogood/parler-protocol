import { ImageResponse } from "next/og";
import { SITE_NAME, SITE_TAGLINE } from "@/lib/seo";
import { OgMark } from "@/lib/og-mark";

// Branded 1200×630 social card, rendered at build time (next/og default font — no font fetch).
export const alt = `${SITE_NAME} — ${SITE_TAGLINE}`;
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

export default function OpengraphImage() {
  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          display: "flex",
          flexDirection: "column",
          justifyContent: "center",
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
          {SITE_NAME.toUpperCase()}
        </div>
        <div
          style={{
            marginTop: 28,
            fontSize: 82,
            fontWeight: 700,
            lineHeight: 1.05,
            letterSpacing: "-0.03em",
            color: "#ffffff",
            maxWidth: 940,
          }}
        >
          Stop copy-pasting context between agents.
        </div>
        <div style={{ marginTop: 28, fontSize: 32, lineHeight: 1.4, color: "#a1a4a5", maxWidth: 900 }}>
          Hand off a live session with one key — your own agent in another repo, or a teammate&apos;s
          at a hackathon. No copy-paste.
        </div>
      </div>
    ),
    { ...size },
  );
}
