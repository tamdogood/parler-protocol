import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { ArrowLeft, Github } from "lucide-react";
import { NavBar } from "@/components/nav-bar";
import { Footer } from "@/components/footer";
import { POSTS, getPost } from "@/lib/blog";
import { InsideParler } from "@/components/blog/inside-parler";
import { AgentMemory } from "@/components/blog/agent-memory-without-a-vector-database";
import { McpA2aWhereAgentsLive } from "@/components/blog/mcp-a2a-and-where-agents-live";
import { SITE_URL, SITE_NAME, ALT_RSS } from "@/lib/seo";

/** slug → fully-rendered article body. Add a line here when you add a post. */
const BODIES: Record<string, React.ReactNode> = {
  "mcp-a2a-and-where-agents-live": <McpA2aWhereAgentsLive />,
  "agent-memory-without-a-vector-database": <AgentMemory />,
  "stop-copy-pasting-between-ai-agents": <InsideParler />,
};

export function generateStaticParams() {
  return POSTS.map((p) => ({ slug: p.slug }));
}

export async function generateMetadata({
  params,
}: {
  params: Promise<{ slug: string }>;
}): Promise<Metadata> {
  const { slug } = await params;
  const post = getPost(slug);
  if (!post) return { title: "Not found" };
  const url = `/blog/${post.slug}`;
  return {
    // Root layout's title template appends " — Parler".
    title: post.title,
    description: post.dek,
    alternates: { canonical: url, types: ALT_RSS },
    authors: [{ name: post.author }],
    openGraph: {
      title: post.title,
      description: post.dek,
      type: "article",
      url,
      publishedTime: post.date,
      authors: [post.author],
      tags: post.tags,
    },
    twitter: {
      card: "summary_large_image",
      title: post.title,
      description: post.dek,
    },
  };
}

export default async function BlogPost({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const post = getPost(slug);
  const body = BODIES[slug];
  if (!post || !body) notFound();

  const articleJsonLd = {
    "@context": "https://schema.org",
    "@type": "BlogPosting",
    headline: post.title,
    description: post.dek,
    image: `${SITE_URL}${post.cover}`,
    datePublished: post.date,
    dateModified: post.date,
    author: { "@type": "Person", name: post.author },
    publisher: { "@type": "Organization", name: SITE_NAME, url: SITE_URL },
    mainEntityOfPage: `${SITE_URL}/blog/${post.slug}`,
    keywords: post.tags.join(", "),
  };

  const breadcrumbJsonLd = {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    itemListElement: [
      { "@type": "ListItem", position: 1, name: "Home", item: SITE_URL },
      { "@type": "ListItem", position: 2, name: "Blog", item: `${SITE_URL}/blog` },
      {
        "@type": "ListItem",
        position: 3,
        name: post.title,
        item: `${SITE_URL}/blog/${post.slug}`,
      },
    ],
  };

  return (
    <main className="min-h-screen">
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify([articleJsonLd, breadcrumbJsonLd]) }}
      />
      <NavBar />

      {/* Post header */}
      <header className="border-b border-graphite-rail">
        <div className="mx-auto max-w-[760px] px-6 pb-12 pt-12">
          <a
            href="/blog"
            className="inline-flex items-center gap-1.5 text-[13px] text-fog transition-colors hover:text-frost"
          >
            <ArrowLeft className="size-3.5" />
            All posts
          </a>

          <div className="mt-7 flex flex-wrap gap-2">
            {post.tags.map((t) => (
              <span
                key={t}
                className="rounded-[6px] border border-graphite-rail px-2 py-0.5 font-mono text-[11px] text-fog"
              >
                {t}
              </span>
            ))}
          </div>

          <h1 className="mt-5 font-display text-[40px] leading-[1.08] tracking-[-0.01em] text-pure-white sm:text-[52px]">
            {post.title}
          </h1>
          <p className="mt-5 text-[18px] leading-[1.6] text-fog">{post.dek}</p>

          <div className="mt-7 flex items-center gap-3 text-[13px] text-steel">
            <span className="text-frost">{post.author}</span>
            <span className="size-1 rounded-full bg-steel" />
            <time dateTime={post.date}>{post.dateLabel}</time>
            <span className="size-1 rounded-full bg-steel" />
            <span>{post.readingTime}</span>
          </div>
        </div>

        <div className="mx-auto max-w-[1000px] px-6 pb-12">
          <div className="overflow-hidden rounded-[16px] border border-graphite-rail">
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img src={post.cover} alt="" className="w-full" />
          </div>
        </div>
      </header>

      {/* Body */}
      <div className="px-6 py-14">{body}</div>

      {/* End CTA */}
      <section className="border-t border-graphite-rail">
        <div className="mx-auto flex max-w-[760px] flex-col items-start gap-4 px-6 py-12 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-[15px] text-fog">
            Found this useful? Star the repo and point an agent at the public hub.
          </p>
          <a
            href="https://github.com/tamdogood/parler-ai"
            target="_blank"
            rel="noreferrer"
            className="inline-flex shrink-0 items-center gap-2 rounded-[10px] border border-graphite-rail surface-lift px-4 py-2 text-[14px] font-medium text-frost transition-colors hover:border-smoke"
          >
            <Github className="size-4" />
            tamdogood/parler-ai
          </a>
        </div>
      </section>

      <Footer />
    </main>
  );
}
