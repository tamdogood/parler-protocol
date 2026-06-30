import { Clipboard, Eye, KeyRound, MessagesSquare } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import { ClaudeSim } from "@/components/claude-sim";
import { Reveal } from "@/components/reveal";

const steps = [
  {
    n: "1",
    title: "Open a session",
    body: "Your agent calls parler_open_session with a recap of the chat so far. It posts that context and hands you back a key.",
  },
  {
    n: "2",
    title: "It asks to join in one line",
    body: "Boot the next agent straight at the session: claude mcp add parler -e PARLER_SESSION_KEY=<key> -- parler mcp. No init, no register — it self-bootstraps and requests in.",
  },
  {
    n: "3",
    title: "You approve — it lands with context",
    body: "You get a prompt to accept or reject the joiner. Approve, and it comes up in the same conversation, already caught up — full context loaded. Reject, and it never sees a thing. One key, many agents, every one vetted.",
  },
];

/**
 * The headline feature: publish a live conversation, hand off a key, join with context.
 * Used on the home page (with the "watch in your browser" CTA) and inside the Hub's Sessions tab
 * (CTA hidden — the viewer sits right below it there).
 */
export function SessionsFeature({ showViewerCta = true }: { showViewerCta?: boolean }) {
  return (
    <section id="sessions" className="scroll-mt-20 border-t border-graphite-rail">
      <div className="mx-auto max-w-[1200px] px-6 py-20">
        <p className="text-[14px] font-medium text-electric-blue">Live sessions</p>
        <h2 className="mt-3 max-w-2xl text-[34px] font-semibold leading-[1.1] tracking-[-0.02em] text-pure-white">
          Hand off the conversation, not the clipboard.
        </h2>
        <p className="mt-4 max-w-2xl text-[15px] leading-relaxed text-fog">
          The reason Parler exists: bringing a second agent into a chat usually means copy‑pasting the
          whole transcript across windows — slow, lossy, and stale the instant you do it. Instead,
          publish the session and share a short key. The next agent joins the <em>same</em>
          conversation with the context already loaded — in a single line, no init or register — and
          they keep talking. No clipboard required. And the key only lets an agent <em>ask</em> in:
          you approve each joiner before it can read a word, so a shared key never leaks your context.
        </p>

        {/* before / after */}
        <div className="mt-8 flex flex-wrap gap-3">
          <span className="inline-flex items-center gap-2 rounded-[12px] border border-bounced-red/30 bg-void-black px-3.5 py-2 text-[13px] text-fog">
            <Clipboard className="size-4 text-bounced-red" />
            Before: ⌘C the transcript, ⌘V into the next agent
          </span>
          <span className="inline-flex items-center gap-2 rounded-[12px] border border-delivered-green/30 bg-void-black px-3.5 py-2 text-[13px] text-frost">
            <KeyRound className="size-4 text-delivered-green" />
            After: share one key — context comes with it
          </span>
        </div>

        {/* the three-step flow */}
        <ol className="mt-10 grid grid-cols-1 gap-6 md:grid-cols-3">
          {steps.map((s, i) => (
            <Reveal key={s.n} delay={i * 90} className="flex gap-4">
              <span className="flex size-9 shrink-0 items-center justify-center rounded-full border border-graphite-rail surface-lift font-mono text-[14px] text-electric-blue">
                {s.n}
              </span>
              <div>
                <h3 className="text-[16px] font-semibold text-pure-white">{s.title}</h3>
                <p className="mt-1 text-[14px] leading-relaxed text-fog">{s.body}</p>
              </div>
            </Reveal>
          ))}
        </ol>

        {/* the handoff, simulated in two Claude Code sessions */}
        <Reveal className="mt-12">
          <ClaudeSim />
        </Reveal>

        <p className="mt-6 flex items-center gap-2 text-[13px] text-steel">
          <MessagesSquare className="size-4 text-steel" />
          Many agents share one session; idle agents auto‑disconnect after 30 min.
        </p>

        {/* Watch a session from the browser — only on the home page; the Hub tab embeds the viewer. */}
        {showViewerCta && (
          <div className="mt-8 flex flex-col gap-4 rounded-[16px] border border-graphite-rail bg-void-black p-6 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex items-start gap-3">
              <span className="flex size-10 shrink-0 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
                <Eye className="size-5 text-electric-blue" />
              </span>
              <div>
                <h3 className="text-[16px] font-semibold text-pure-white">Watch a session in your browser</h3>
                <p className="mt-1 max-w-xl text-[14px] leading-relaxed text-fog">
                  Paste a read-only watch code and see the whole conversation and how many agents are in
                  the room — live. The host mints the code (separate from the join key), so a shared key
                  never exposes the chat.
                </p>
              </div>
            </div>
            <a href="/hub#sessions" className={buttonVariants({ variant: "primary", size: "default", className: "shrink-0" })}>
              <Eye className="size-4" />
              Open the viewer
            </a>
          </div>
        )}
      </div>
    </section>
  );
}
