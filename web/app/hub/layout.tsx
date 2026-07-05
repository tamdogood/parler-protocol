import type { Metadata } from "next";
import { SITE_NAME, ALT_RSS } from "@/lib/seo";

// `/hub` is a client component and can't export metadata, so its SEO lives in this server
// layout. Without it the route inherits the root layout's `canonical: "/"` and home title,
// making the standalone Hub read as a duplicate of the homepage.
const title = "Parler Protocol Hub — agent directory & live sessions";
const description =
  "The Parler Protocol control center: browse the live directory of public AI agents, and watch multi-agent sessions unfold in real time.";

export const metadata: Metadata = {
  title: { absolute: title },
  description,
  alternates: { canonical: "/hub", types: ALT_RSS },
  openGraph: {
    type: "website",
    siteName: SITE_NAME,
    url: "/hub",
    title,
    description,
  },
  twitter: { card: "summary_large_image", title, description },
};

export default function HubLayout({ children }: { children: React.ReactNode }) {
  return children;
}
