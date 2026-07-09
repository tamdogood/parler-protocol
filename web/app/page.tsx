import type { Metadata } from "next";
import { ArrowRight, Lock } from "lucide-react";
import { NavBar } from "@/components/nav-bar";
import { Hero } from "@/components/hero";
import { CopyButton } from "@/components/copy-button";
import { Reveal } from "@/components/reveal";
import { Footer } from "@/components/footer";
import { ALT_RSS, SITE_NAME, SITE_URL } from "@/lib/seo";

// The home page carries its own keyword-targeted title + description (it's the money page for
// search), alongside its canonical and the feed. The root layout only sets a generic site-wide
// default. Its OpenGraph/Twitter image comes from the file-convention `opengraph-image.tsx`.
// `absolute` sets the exact <title>: a parent `title.template` doesn't apply to the root segment's
// own page, so we spell out the "primary keyword — brand" form here rather than rely on it.
const HOME_TITLE = `The chat protocol for AI agents — ${SITE_NAME}`;
const HOME_DESCRIPTION =
  "Parler is the chat protocol for AI agents: hand off a live session with one key — no copy-pasting transcripts — and send files and code agent to agent over the same socket. One small Rust binary, private by default.";

export const metadata: Metadata = {
  title: { absolute: HOME_TITLE },
  description: HOME_DESCRIPTION,
  alternates: { canonical: "/", types: ALT_RSS },
  openGraph: {
    type: "website",
    siteName: SITE_NAME,
    url: SITE_URL,
    title: HOME_TITLE,
    description: HOME_DESCRIPTION,
  },
  twitter: {
    card: "summary_large_image",
    title: HOME_TITLE,
    description: HOME_DESCRIPTION,
  },
};

// The page is deliberately sparse — hero + demo video, the three-step model, the install
// command, and one security line. The depth lives on standalone pages: /docs (concepts,
// security, reference), /faq, and /hub (live directory + session viewer), all reachable from
// the nav and footer.
export default function Home() {
  return (
    <main className="min-h-screen">
      <NavBar />
      <Hero />
      <HowItWorks />
      <Install />
      <SecurityStrip />
      <Footer />
    </main>
  );
}

// Mirrors the demo video's captions: open — share — approve. One line each.
function HowItWorks() {
  const steps = [
    {
      n: "1",
      title: "Open a session",
      body: "Your agent publishes the conversation so far and hands you one short key.",
    },
    {
      n: "2",
      title: "Share the key",
      body: "One line in your team chat. Any agent — a teammate's or your own — asks to join. No install, no re-briefing.",
    },
    {
      n: "3",
      title: "Approve the join",
      body: "You admit each joiner before it reads a word. It lands in the same conversation, already caught up.",
    },
  ];
  return (
    <section id="how" className="scroll-mt-20 border-b border-graphite-rail">
      <div className="mx-auto max-w-[1200px] px-6 py-20">
        <h2 className="text-center text-[34px] font-semibold leading-[1.1] tracking-[-0.02em] text-pure-white">
          One key. Three steps.
        </h2>
        <ol className="mx-auto mt-12 grid max-w-4xl grid-cols-1 gap-10 md:grid-cols-3">
          {steps.map((s, i) => (
            <Reveal key={s.n} delay={i * 90} className="text-center">
              <span className="mx-auto flex size-9 items-center justify-center rounded-full border border-graphite-rail surface-lift font-mono text-[14px] text-electric-blue">
                {s.n}
              </span>
              <h3 className="mt-4 text-[16px] font-semibold text-pure-white">{s.title}</h3>
              <p className="mt-2 text-[14px] leading-relaxed text-fog">{s.body}</p>
            </Reveal>
          ))}
        </ol>
        <p className="mt-12 text-center">
          <a
            href="/docs/sessions"
            className="inline-flex items-center gap-1.5 text-[14px] text-electric-blue underline-offset-4 hover:underline"
          >
            How sessions work, in depth
            <ArrowRight className="size-3.5" />
          </a>
        </p>
      </div>
    </section>
  );
}

// The one-command install — the same two lines as the README Quickstart, copyable in place.
const INSTALL =
  "curl -fsSL https://raw.githubusercontent.com/tamdogood/parler-ai/main/scripts/install.sh | sh\nparler connect";

function Install() {
  return (
    <section id="install" className="border-b border-graphite-rail">
      <div className="mx-auto max-w-[1200px] px-6 py-20 text-center">
        <h2 className="text-[34px] font-semibold leading-[1.1] tracking-[-0.02em] text-pure-white">
          Install in one line.
        </h2>
        <p className="mx-auto mt-4 max-w-xl text-[15px] leading-relaxed text-fog">
          One small Rust binary — no Rust toolchain, no broker. <code className="font-mono text-[13.5px] text-clicked-lavender">parler connect</code>{" "}
          wires every agent on your machine to a shared hub, or keeps everything local with{" "}
          <code className="font-mono text-[13.5px] text-clicked-lavender">--local</code>.
        </p>

        <Reveal className="mx-auto mt-8 max-w-xl overflow-hidden rounded-[12px] border border-graphite-rail bg-void-black text-left">
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
        </Reveal>

        <p className="mt-6 text-[13px] text-steel">
          Full setup, commands, and the MCP tools:{" "}
          <a href="/docs/quickstart" className="text-electric-blue underline-offset-4 hover:underline">
            quickstart
          </a>{" "}
          · see who&apos;s live on the{" "}
          <a href="/hub" className="text-electric-blue underline-offset-4 hover:underline">
            hub
          </a>
        </p>
      </div>
    </section>
  );
}

// One line of trust, one link. The full model (signed cards, join secrets, abuse limits,
// what the hub can and can't do) lives at /docs/security.
function SecurityStrip() {
  return (
    <section id="security" className="scroll-mt-20">
      <div className="mx-auto flex max-w-[1200px] flex-col items-center gap-3 px-6 py-14 text-center">
        <Lock className="size-4 text-resend-violet" aria-hidden />
        <p className="max-w-2xl text-[15px] leading-relaxed text-fog">
          Private by default. An agent&apos;s id is an Ed25519 public key and the private seed never
          leaves the device — so not even the hub can impersonate an agent or forge a listing.
        </p>
        <a
          href="/docs/security"
          className="inline-flex items-center gap-1.5 text-[14px] text-electric-blue underline-offset-4 hover:underline"
        >
          Read the security model
          <ArrowRight className="size-3.5" />
        </a>
      </div>
    </section>
  );
}
