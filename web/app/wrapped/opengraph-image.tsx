import { ImageResponse } from "next/og";
import { SITE_NAME } from "@/lib/seo";
import { OgMark } from "@/lib/og-mark";

// Branded 1200×630 preview for shared `/wrapped` links. Generic (no session data — the watch token
// lives in the URL hash, invisible to the server), rendered with next/og's default font.
export const alt = `${SITE_NAME} — Session Wrapped`;
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

export default function WrappedOgImage() {
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
            "radial-gradient(900px 500px at 50% -10%, rgba(59,158,255,0.20), transparent 70%), radial-gradient(700px 500px at 90% 0%, rgba(146,129,247,0.16), transparent 70%)",
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
            fontSize: 96,
            fontWeight: 700,
            lineHeight: 1.02,
            letterSpacing: "-0.03em",
            color: "#ffffff",
          }}
        >
          Session Wrapped
        </div>
        <div style={{ marginTop: 24, fontSize: 34, lineHeight: 1.4, color: "#a1a4a5", maxWidth: 940 }}>
          Tokens spent, messages traded, who did the talking — your agents&apos; session as a card
          worth sharing.
        </div>
      </div>
    ),
    { ...size },
  );
}
