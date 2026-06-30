import { ArrowRight, KeyRound } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import { ParticleField } from "@/components/particle-field";

export function Hero() {
  return (
    <section className="canvas-glow relative overflow-hidden border-b border-graphite-rail">
      <div className="grid-faint pointer-events-none absolute inset-0" aria-hidden />
      <ParticleField className="pointer-events-none absolute inset-0 z-0 opacity-70" />
      <div className="relative z-10 mx-auto max-w-[1200px] px-6 pb-20 pt-20 text-center sm:pt-28">
        <a
          href="#sessions"
          className="inline-flex animate-[slide-up-fade_0.4s_ease_both] items-center gap-2 rounded-[16px] border border-graphite-rail px-3 py-1 text-[13px] text-frost transition-colors hover:border-smoke"
        >
          <KeyRound className="size-3.5 text-resend-violet" />
          New — hand off a live conversation with a key
          <ArrowRight className="size-3" />
        </a>

        <h1
          className="mx-auto mt-7 max-w-3xl animate-[slide-up-fade_0.5s_ease_both] font-display text-[44px] leading-[1.04] tracking-[-0.01em] text-pure-white sm:text-[68px]"
        >
          Stop copy‑pasting between your agents.
        </h1>

        <p className="mx-auto mt-5 max-w-2xl animate-[slide-up-fade_0.6s_ease_both] text-[17px] leading-[1.6] text-fog">
          You&apos;re mid‑conversation with one AI agent and need another to jump in. Skip the
          ⌘C / ⌘V: publish your session, share a key, and the next agent — Claude, Codex, Hermes —
          joins the <em>same</em> conversation with the full context already loaded — in one line.
        </p>

        <div className="mt-8 flex animate-[slide-up-fade_0.7s_ease_both] flex-wrap items-center justify-center gap-3">
          <a href="#sessions" className={buttonVariants({ variant: "primary" })}>
            See how the handoff works
            <ArrowRight className="size-4" />
          </a>
          <a href="/hub" className={buttonVariants({ variant: "ghost" })}>
            Browse the directory
          </a>
        </div>
      </div>
    </section>
  );
}
