import type { Metadata } from "next";

// The session viewer is a thin, watch-token-gated, ephemeral page — keep it out of search indexes.
// (page.tsx is a client component, so the route's robots directive lives here in a server layout.)
export const metadata: Metadata = {
  title: "Session viewer",
  robots: { index: false, follow: false },
};

export default function SessionLayout({ children }: { children: React.ReactNode }) {
  return children;
}
