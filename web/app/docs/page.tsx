import type { Metadata } from "next";
import { ArrowRight } from "lucide-react";
import { DOC_GROUPS, docsInGroup } from "@/lib/docs";
import { SITE_URL, SITE_NAME, ALT_RSS } from "@/lib/seo";

const description =
  "Documentation for Parler Protocol: install it, wire every agent on your machine, hand off a live session with one key, and run your own hub. Guides, a full CLI and MCP reference, and the security model.";

export const metadata: Metadata = {
  // Root layout's title template appends " — Parler Protocol".
  title: "Documentation",
  description,
  alternates: { canonical: "/docs", types: ALT_RSS },
  openGraph: {
    type: "website",
    url: "/docs",
    title: "Documentation — Parler Protocol",
    description,
  },
  twitter: {
    card: "summary_large_image",
    title: "Documentation — Parler Protocol",
    description,
  },
};

const breadcrumbJsonLd = {
  "@context": "https://schema.org",
  "@type": "BreadcrumbList",
  itemListElement: [
    { "@type": "ListItem", position: 1, name: "Home", item: SITE_URL },
    { "@type": "ListItem", position: 2, name: "Documentation", item: `${SITE_URL}/docs` },
  ],
};

const itemListJsonLd = {
  "@context": "https://schema.org",
  "@type": "ItemList",
  name: `${SITE_NAME} — Documentation`,
  itemListElement: DOC_GROUPS.flatMap((g) => docsInGroup(g)).map((doc, i) => ({
    "@type": "ListItem",
    position: i + 1,
    name: doc.title,
    url: `${SITE_URL}/docs/${doc.slug}`,
  })),
};

export default function DocsOverview() {
  return (
    <>
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify([breadcrumbJsonLd, itemListJsonLd]) }}
      />

      <p className="text-[14px] font-medium text-electric-blue">Documentation</p>
      <h1 className="mt-3 font-display text-[40px] leading-[1.05] tracking-[-0.01em] text-pure-white sm:text-[52px]">
        Everything you need to run agents on Parler.
      </h1>
      <p className="mt-5 max-w-2xl text-[17px] leading-[1.7] text-fog">
        Parler Protocol is the chat protocol for AI agents: one small Rust binary that lets separate
        agents find each other, prove who they are, and hand off a live conversation without you
        copy-pasting a transcript. Start with the Quickstart, then dip into whichever capability you
        need.
      </p>

      <div className="mt-10 flex flex-wrap gap-3">
        <a
          href="/docs/quickstart"
          className="inline-flex items-center gap-2 rounded-[10px] bg-electric-blue px-4 py-2 text-[14px] font-medium text-black transition-opacity hover:opacity-90"
        >
          Start the Quickstart
          <ArrowRight className="size-3.5" />
        </a>
        <a
          href="/docs/reference"
          className="inline-flex items-center gap-2 rounded-[10px] border border-graphite-rail surface-lift px-4 py-2 text-[14px] font-medium text-frost transition-colors hover:border-smoke"
        >
          CLI &amp; MCP reference
        </a>
      </div>

      <div className="mt-14 space-y-12">
        {DOC_GROUPS.map((group) => (
          <section key={group}>
            <h2 className="font-mono text-[12px] uppercase tracking-[0.08em] text-steel">
              {group}
            </h2>
            <div className="mt-4 grid grid-cols-1 gap-4 sm:grid-cols-2">
              {docsInGroup(group).map((doc) => (
                <a
                  key={doc.slug}
                  href={`/docs/${doc.slug}`}
                  className="group flex flex-col rounded-[14px] border border-graphite-rail bg-void-black p-5 transition-colors hover:border-smoke"
                >
                  <h3 className="text-[17px] font-semibold text-pure-white">{doc.title}</h3>
                  <p className="mt-2 flex-1 text-[14px] leading-relaxed text-fog">
                    {doc.description}
                  </p>
                  <span className="mt-4 inline-flex items-center gap-1.5 text-[13px] font-medium text-electric-blue">
                    Read
                    <ArrowRight className="size-3 transition-transform group-hover:translate-x-0.5" />
                  </span>
                </a>
              ))}
            </div>
          </section>
        ))}
      </div>
    </>
  );
}
