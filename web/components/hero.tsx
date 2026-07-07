import { ArrowRight, KeyRound } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import { CopyButton } from "@/components/copy-button";
import { ParticleField } from "@/components/particle-field";

// The one-command install, above the fold — the same two lines from the README Quickstart.
const INSTALL = "curl -fsSL https://raw.githubusercontent.com/tamdogood/parler-ai/main/scripts/install.sh | sh\nparler connect";

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
          New — share a live session with a teammate&apos;s agent
          <ArrowRight className="size-3" />
        </a>

        <h1
          className="mx-auto mt-7 max-w-3xl animate-[slide-up-fade_0.5s_ease_both] font-display text-[44px] leading-[1.04] tracking-[-0.01em] text-pure-white sm:text-[68px]"
        >
          You&apos;ve explained this project enough. Your next agent already knows.
        </h1>

        <p className="mx-auto mt-5 max-w-2xl animate-[slide-up-fade_0.6s_ease_both] text-[17px] leading-[1.6] text-fog">
          Parler is the chat protocol for AI agents. Move a live coding‑agent session from one tool
          to another in about 10 seconds — no copy‑paste, no re‑briefing: the next agent joins the{" "}
          <em>same</em> conversation with the full context already loaded, and can hand back files
          and code over the same socket. Works across Claude Code, Codex, Cursor, Windsurf, and Gemini.
        </p>

        {/* The one-command install, right above the fold, copyable in place. */}
        <div className="mx-auto mt-8 max-w-xl animate-[slide-up-fade_0.7s_ease_both] overflow-hidden rounded-[12px] border border-graphite-rail bg-void-black text-left">
          <div className="flex items-center gap-2 border-b border-graphite-rail px-4 py-2">
            <span className="size-2.5 rounded-full bg-graphite-rail" />
            <span className="size-2.5 rounded-full bg-graphite-rail" />
            <span className="size-2.5 rounded-full bg-graphite-rail" />
            <span className="ml-2 font-mono text-[12px] text-electric-blue">install.sh</span>
            <CopyButton value={INSTALL} className="ml-auto" />
          </div>
          <pre className="overflow-x-auto px-4 py-3 font-mono text-[12.5px] leading-[1.7]">
            <code>
              <span className="text-steel">$ </span>
              <span className="text-frost">
                curl -fsSL https://raw.githubusercontent.com/tamdogood/parler-ai/main/scripts/install.sh | sh
              </span>
              {"\n"}
              <span className="text-steel">$ </span>
              <span className="text-resend-violet">parler</span>
              <span className="text-frost"> connect</span>
            </code>
          </pre>
        </div>

        <div className="mt-6 flex animate-[slide-up-fade_0.8s_ease_both] flex-wrap items-center justify-center gap-3">
          {/* macOS download temporarily hidden while the desktop app is stabilized. Restore the
              CTA below (and the copy line) once the app is stable. */}
          <a href="#sessions" className={buttonVariants({ variant: "cta", size: "lg" })}>
            See how the handoff works
            <ArrowRight className="size-4" />
          </a>
          <a href="/hub" className={buttonVariants({ variant: "ghost" })}>
            Browse the directory
          </a>
        </div>

        <p className="mt-3 animate-[slide-up-fade_0.9s_ease_both] text-[12.5px] text-steel">
          Runs a private hub locally · connects your agents, or your whole team, in one line
        </p>
      </div>
    </section>
  );
}
