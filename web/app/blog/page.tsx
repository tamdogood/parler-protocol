import type { Metadata } from "next";
import { ArrowRight } from "lucide-react";
import { NavBar } from "@/components/nav-bar";
import { Footer } from "@/components/footer";
import { Reveal } from "@/components/reveal";
import { postsByDate } from "@/lib/blog";

export const metadata: Metadata = {
  title: "Blog — Parler",
  description:
    "Engineering notes from the Parler project: architecture deep dives on coordinating AI agents over one Rust binary and an embedded SQLite log.",
};

export default function BlogIndex() {
  return (
    <main className="min-h-screen">
      <NavBar />

      <section className="border-b border-graphite-rail">
        <div className="mx-auto max-w-[1200px] px-6 py-20">
          <p className="text-[14px] font-medium text-electric-blue">Blog</p>
          <h1 className="mt-3 max-w-2xl font-display text-[44px] leading-[1.05] tracking-[-0.01em] text-pure-white sm:text-[56px]">
            Engineering notes from the mesh.
          </h1>
          <p className="mt-4 max-w-2xl text-[16px] leading-relaxed text-fog">
            How Parler coordinates AI agents: the wire protocol, the cryptographic identity, the
            cursor that makes late-join free, and the rest of the architecture, with real code from
            the repo.
          </p>
        </div>
      </section>

      <section>
        <div className="mx-auto max-w-[1200px] px-6 py-16">
          <div className="grid grid-cols-1 gap-6 md:grid-cols-2 lg:grid-cols-3">
            {postsByDate.map((post, i) => (
              <Reveal key={post.slug} delay={i * 80}>
                <a
                  href={`/blog/${post.slug}`}
                  className="group flex h-full flex-col overflow-hidden rounded-[16px] border border-graphite-rail bg-void-black transition-colors hover:border-smoke"
                >
                  <div className="aspect-[16/9] overflow-hidden border-b border-graphite-rail">
                    {/* eslint-disable-next-line @next/next/no-img-element */}
                    <img
                      src={post.cover}
                      alt=""
                      className="h-full w-full object-cover opacity-90 transition-transform duration-500 group-hover:scale-[1.03]"
                    />
                  </div>
                  <div className="flex flex-1 flex-col p-6">
                    <div className="flex flex-wrap gap-2">
                      {post.tags.slice(0, 2).map((t) => (
                        <span
                          key={t}
                          className="rounded-[6px] border border-graphite-rail px-2 py-0.5 font-mono text-[11px] text-fog"
                        >
                          {t}
                        </span>
                      ))}
                    </div>
                    <h2 className="mt-4 text-[20px] font-semibold leading-snug text-pure-white">
                      {post.title}
                    </h2>
                    <p className="mt-2 text-[14px] leading-relaxed text-fog">{post.dek}</p>
                    <div className="mt-5 flex items-center gap-3 text-[12px] text-steel">
                      <time dateTime={post.date}>{post.dateLabel}</time>
                      <span className="size-1 rounded-full bg-steel" />
                      <span>{post.readingTime}</span>
                    </div>
                    <span className="mt-5 inline-flex items-center gap-1.5 text-[14px] font-medium text-electric-blue">
                      Read post
                      <ArrowRight className="size-3.5 transition-transform group-hover:translate-x-0.5" />
                    </span>
                  </div>
                </a>
              </Reveal>
            ))}
          </div>
        </div>
      </section>

      <Footer />
    </main>
  );
}
