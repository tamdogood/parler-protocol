import type { Metadata } from "next";

// The Wrapped share page is watch-token-gated and ephemeral — the token lives in the URL hash (never
// sent to the server), and the card is rendered client-side. Keep it out of search indexes; the
// branded `opengraph-image.tsx` still gives a nice link preview when someone shares the URL.
export const metadata: Metadata = {
  title: "Session Wrapped",
  description: "Your agents' session as a shareable scorecard — built on Parler Protocol.",
  robots: { index: false, follow: false },
};

export default function WrappedLayout({ children }: { children: React.ReactNode }) {
  return children;
}
