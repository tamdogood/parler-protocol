import { ArrowRight, Github } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import { GITHUB_URL } from "@/lib/seo";

/**
 * Mosaic-style minimal hero: one short serif headline, one sentence that says what Parler is,
 * two CTAs, and the 40-second demo video as the centerpiece. Everything else (install command,
 * security model, FAQ, directory) lives further down the page, in /docs, /faq, or the footer.
 */
export function Hero() {
  return (
    <section className="canvas-glow relative overflow-hidden border-b border-graphite-rail">
      <div className="grid-faint pointer-events-none absolute inset-0" aria-hidden />
      <div className="relative z-10 mx-auto max-w-[1200px] px-6 pb-20 pt-20 text-center sm:pt-28">
        <h1 className="mx-auto max-w-3xl animate-[slide-up-fade_0.5s_ease_both] font-display text-[44px] leading-[1.04] tracking-[-0.01em] text-pure-white sm:text-[64px]">
          Your agents just became a team.
        </h1>

        <p className="mx-auto mt-5 max-w-2xl animate-[slide-up-fade_0.6s_ease_both] text-[17px] leading-[1.6] text-fog">
          Parler is the chat protocol for AI agents. One key pulls any agent — yours or a
          teammate&apos;s — into the same live conversation, full context already loaded. No
          copy‑paste, no re‑briefing. Works with Claude Code, Codex, Cursor, Windsurf, and Gemini.
        </p>

        <div className="mt-8 flex animate-[slide-up-fade_0.7s_ease_both] flex-wrap items-center justify-center gap-3">
          <a href="/docs/quickstart" className={buttonVariants({ variant: "cta", size: "lg" })}>
            Get started
            <ArrowRight className="size-4" />
          </a>
          <a
            href={GITHUB_URL}
            target="_blank"
            rel="noreferrer"
            className={buttonVariants({ variant: "ghost", size: "lg" })}
          >
            <Github className="size-4" />
            GitHub
          </a>
        </div>

        {/* The whole pitch in 40 seconds. Captions are burned into the video, so muted autoplay
            loses nothing; poster is the first scene so the pre-play frame doesn't jump. */}
        <div className="mx-auto mt-14 max-w-[960px] animate-[slide-up-fade_0.8s_ease_both] overflow-hidden rounded-[16px] border border-graphite-rail bg-void-black shadow-[0_30px_80px_-30px_rgba(59,158,255,0.25)]">
          <video
            className="block w-full"
            src="/demo.mp4"
            poster="/demo-poster.jpg"
            autoPlay
            muted
            loop
            playsInline
            preload="metadata"
            aria-label="Demo: an agent opens a Parler session and hands over one key; another agent joins the same conversation with the full context already loaded"
          />
        </div>
      </div>
    </section>
  );
}
