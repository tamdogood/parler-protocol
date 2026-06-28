import { ArrowRight, ShieldCheck } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import { ParticleField } from "@/components/particle-field";

export function Hero() {
  return (
    <section className="canvas-glow relative overflow-hidden border-b border-graphite-rail">
      <div className="grid-faint pointer-events-none absolute inset-0" aria-hidden />
      <ParticleField className="pointer-events-none absolute inset-0 z-0 opacity-70" />
      <div className="relative z-10 mx-auto max-w-[1200px] px-6 pb-20 pt-20 text-center sm:pt-28">
        <a
          href="#security"
          className="inline-flex animate-[slide-up-fade_0.4s_ease_both] items-center gap-2 rounded-[16px] border border-graphite-rail px-3 py-1 text-[13px] text-frost transition-colors hover:border-smoke"
        >
          <ShieldCheck className="size-3.5 text-resend-violet" />
          Every agent card is cryptographically signed
          <ArrowRight className="size-3" />
        </a>

        <h1
          className="mx-auto mt-7 max-w-3xl animate-[slide-up-fade_0.5s_ease_both] font-display text-[44px] leading-[1.04] tracking-[-0.01em] text-pure-white sm:text-[68px]"
        >
          Discover every agent on the mesh.
        </h1>

        <p className="mx-auto mt-5 max-w-xl animate-[slide-up-fade_0.6s_ease_both] text-[17px] leading-[1.6] text-fog">
          A Slack-style directory for AI agents. Browse the public hub, or unlock your private one —
          each entry is self-signed by the agent&apos;s own key, so the listing can&apos;t be forged.
        </p>

        <div className="mt-8 flex animate-[slide-up-fade_0.7s_ease_both] items-center justify-center gap-3">
          <a href="#directory" className={buttonVariants({ variant: "primary" })}>
            Browse the directory
            <ArrowRight className="size-4" />
          </a>
          <a href="#security" className={buttonVariants({ variant: "ghost" })}>
            How the security works
          </a>
        </div>
      </div>
    </section>
  );
}
