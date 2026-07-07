import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { ArrowLeft, ArrowRight } from "lucide-react";
import { DOCS, getDoc, docNeighbors } from "@/lib/docs";
import { SITE_URL, SITE_NAME, ALT_RSS } from "@/lib/seo";
import { Introduction } from "@/components/docs/introduction";
import { Quickstart } from "@/components/docs/quickstart";
import { CoreConcepts } from "@/components/docs/core-concepts";
import { Sessions } from "@/components/docs/sessions";
import { Messaging } from "@/components/docs/messaging";
import { Memory } from "@/components/docs/memory";
import { FileAndCodeHandoff } from "@/components/docs/file-and-code-handoff";
import { Reference } from "@/components/docs/reference";
import { SelfHosting } from "@/components/docs/self-hosting";
import { Security } from "@/components/docs/security";
import { Troubleshooting } from "@/components/docs/troubleshooting";

/** slug → fully-rendered page body. Add a line here when you add a doc page. */
const BODIES: Record<string, React.ReactNode> = {
  introduction: <Introduction />,
  quickstart: <Quickstart />,
  "core-concepts": <CoreConcepts />,
  sessions: <Sessions />,
  messaging: <Messaging />,
  memory: <Memory />,
  "file-and-code-handoff": <FileAndCodeHandoff />,
  reference: <Reference />,
  "self-hosting": <SelfHosting />,
  security: <Security />,
  troubleshooting: <Troubleshooting />,
};

export function generateStaticParams() {
  return DOCS.map((d) => ({ slug: d.slug }));
}

export async function generateMetadata({
  params,
}: {
  params: Promise<{ slug: string }>;
}): Promise<Metadata> {
  const { slug } = await params;
  const doc = getDoc(slug);
  if (!doc) return { title: "Not found" };
  const url = `/docs/${doc.slug}`;
  return {
    // Root layout's title template appends " — Parler Protocol".
    title: `${doc.title} · Docs`,
    description: doc.description,
    alternates: { canonical: url, types: ALT_RSS },
    openGraph: {
      type: "article",
      url,
      title: `${doc.title} — Parler Protocol Docs`,
      description: doc.description,
    },
    twitter: {
      card: "summary_large_image",
      title: `${doc.title} — Parler Protocol Docs`,
      description: doc.description,
    },
  };
}

export default async function DocPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const doc = getDoc(slug);
  const body = BODIES[slug];
  if (!doc || !body) notFound();

  const { prev, next } = docNeighbors(slug);

  const articleJsonLd = {
    "@context": "https://schema.org",
    "@type": "TechArticle",
    headline: doc.title,
    description: doc.description,
    author: { "@type": "Organization", name: SITE_NAME, url: SITE_URL },
    publisher: { "@type": "Organization", name: SITE_NAME, url: SITE_URL },
    mainEntityOfPage: `${SITE_URL}/docs/${doc.slug}`,
  };

  const breadcrumbJsonLd = {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    itemListElement: [
      { "@type": "ListItem", position: 1, name: "Home", item: SITE_URL },
      { "@type": "ListItem", position: 2, name: "Documentation", item: `${SITE_URL}/docs` },
      {
        "@type": "ListItem",
        position: 3,
        name: doc.title,
        item: `${SITE_URL}/docs/${doc.slug}`,
      },
    ],
  };

  return (
    <>
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify([articleJsonLd, breadcrumbJsonLd]) }}
      />

      <div className="mx-auto max-w-[760px]">
        <a
          href="/docs"
          className="inline-flex items-center gap-1.5 text-[13px] text-fog transition-colors hover:text-frost"
        >
          <ArrowLeft className="size-3.5" />
          All docs
        </a>

        <p className="mt-6 font-mono text-[12px] uppercase tracking-[0.08em] text-steel">
          {doc.group}
        </p>
        <h1 className="mt-2 font-display text-[38px] leading-[1.08] tracking-[-0.01em] text-pure-white sm:text-[46px]">
          {doc.title}
        </h1>
        <p className="mt-4 text-[17px] leading-[1.6] text-fog">{doc.description}</p>

        <div className="mt-4 border-t border-graphite-rail" />

        {body}

        {/* Prev / next */}
        <nav className="mt-16 grid grid-cols-1 gap-4 border-t border-graphite-rail pt-8 sm:grid-cols-2">
          {prev ? (
            <a
              href={`/docs/${prev.slug}`}
              className="group flex flex-col rounded-[12px] border border-graphite-rail p-4 transition-colors hover:border-smoke"
            >
              <span className="inline-flex items-center gap-1.5 text-[12px] text-steel">
                <ArrowLeft className="size-3" />
                Previous
              </span>
              <span className="mt-1 text-[15px] font-medium text-frost">{prev.title}</span>
            </a>
          ) : (
            <span />
          )}
          {next && (
            <a
              href={`/docs/${next.slug}`}
              className="group flex flex-col rounded-[12px] border border-graphite-rail p-4 text-right transition-colors hover:border-smoke sm:col-start-2"
            >
              <span className="inline-flex items-center justify-end gap-1.5 text-[12px] text-steel">
                Next
                <ArrowRight className="size-3" />
              </span>
              <span className="mt-1 text-[15px] font-medium text-frost">{next.title}</span>
            </a>
          )}
        </nav>
      </div>
    </>
  );
}
