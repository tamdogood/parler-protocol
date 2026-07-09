import type { Metadata } from "next";
import { NavBar } from "@/components/nav-bar";
import { Faq } from "@/components/faq";
import { Footer } from "@/components/footer";
import { ALT_RSS, SITE_NAME, SITE_URL } from "@/lib/seo";

// The FAQ used to live on the home page; it moved here when the landing page slimmed down.
// The FAQPage JSON-LD travels with the <Faq /> component, so the rich-result eligibility moved too.
const TITLE = "FAQ";
const DESCRIPTION =
  "Answers to common questions about Parler Protocol: how it differs from MCP, whether you need a server, what the session approval step is for, file and code handoff between agents, scaling, and licensing.";

export const metadata: Metadata = {
  title: TITLE,
  description: DESCRIPTION,
  alternates: { canonical: "/faq", types: ALT_RSS },
  openGraph: {
    type: "website",
    siteName: SITE_NAME,
    url: `${SITE_URL}/faq`,
    title: `${TITLE} — ${SITE_NAME}`,
    description: DESCRIPTION,
  },
  twitter: {
    card: "summary_large_image",
    title: `${TITLE} — ${SITE_NAME}`,
    description: DESCRIPTION,
  },
};

export default function FaqPage() {
  return (
    <main className="min-h-screen">
      <NavBar />
      <Faq />
      <Footer />
    </main>
  );
}
