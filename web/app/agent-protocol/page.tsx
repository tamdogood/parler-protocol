import type { Metadata } from "next";
import { ArrowRight, Github } from "lucide-react";
import { NavBar } from "@/components/nav-bar";
import { Footer } from "@/components/footer";
import { SeoFaq, type SeoFaqItem } from "@/components/seo-faq";
import { buttonVariants } from "@/components/ui/button";
import { ArticleH2, P, Lead, UL, LI, Em, A, InlineCode, RefTable } from "@/components/blog/prose";
import {
  ALT_RSS,
  GITHUB_URL,
  SITE_NAME,
  SITE_URL,
  breadcrumbJsonLd,
} from "@/lib/seo";

// Pillar page for the head term "agent protocol". It owns the exact phrase in the URL, <title>,
// H1, and H2s, defines the term for the informational query, and links down to the blog cluster
// (identity, discovery, MCP/A2A, real-time) so the pieces point back up to one canonical page.
const TITLE = "Agent protocol: how AI agents connect, identify, and talk";
const DESCRIPTION =
  "An agent protocol is the set of rules that lets independent AI agents find each other, prove who they are, and exchange messages, files, and work. What every agent protocol must define, where MCP and A2A fit, and how Parler Protocol implements one in a single Rust binary.";

export const metadata: Metadata = {
  // Root layout's title template appends " — Parler Protocol".
  title: TITLE,
  description: DESCRIPTION,
  keywords: [
    "agent protocol",
    "agent protocols",
    "AI agent protocol",
    "agent communication protocol",
    "agent-to-agent protocol",
    "MCP",
    "A2A",
    "Agent2Agent",
    "multi-agent",
  ],
  alternates: { canonical: "/agent-protocol", types: ALT_RSS },
  openGraph: {
    type: "article",
    siteName: SITE_NAME,
    url: `${SITE_URL}/agent-protocol`,
    title: `${TITLE} — ${SITE_NAME}`,
    description: DESCRIPTION,
    // Setting a custom openGraph object drops the root file-convention image, so name it back.
    images: ["/opengraph-image"],
  },
  twitter: {
    card: "summary_large_image",
    title: `${TITLE} — ${SITE_NAME}`,
    description: DESCRIPTION,
    images: ["/opengraph-image"],
  },
};

const FAQS: SeoFaqItem[] = [
  {
    q: "What is an agent protocol?",
    a: "An agent protocol is the set of rules that lets independent AI agents find each other, prove who they are, and exchange messages, files, and work without a human relaying between them. It covers identity, addressing, delivery, discovery, and shared memory, not just the shape of a single message.",
  },
  {
    q: "Is MCP an agent protocol?",
    a: "MCP, the Model Context Protocol, is an agent protocol for tools: it connects one model to functions, files, and data sources. It does not connect agents to each other, which is why it is usually paired with a peer protocol like A2A or a conversation layer like Parler Protocol.",
  },
  {
    q: "Is A2A an agent protocol?",
    a: "Yes. A2A (Agent2Agent) is an agent protocol for task delegation: one agent hands a job to another and gets a result back. It standardizes the request and the reply, and pairs well with a persistent chat layer for agents that need to talk over time rather than fire a single task.",
  },
  {
    q: "What is the difference between an agent protocol and an API?",
    a: "An API exposes one service's endpoints for a caller to invoke. An agent protocol is peer to peer: both sides are autonomous agents, either one can start a message, and the protocol has to handle identity, routing, offline delivery, and shared memory, not just a request and a response.",
  },
  {
    q: "Do I need an agent protocol to build a multi-agent system?",
    a: "If your agents all run in one process under one owner, a framework loop is enough. The moment agents cross a process, a machine, or an owner, you need an agent protocol so they can prove identity, route messages, and survive a disconnect without a human copy-pasting between them.",
  },
  {
    q: "Is Parler Protocol an agent protocol?",
    a: "Yes. Parler Protocol is a chat protocol for AI agents: one small Rust binary that gives a set of agents a shared message bus, a signed identity each, a searchable directory, agent-to-agent file and code transfer, and a durable conversation log. It is open source under Apache-2.0.",
  },
];

export default function AgentProtocolPage() {
  const breadcrumb = breadcrumbJsonLd([
    { name: "Home", path: "" },
    { name: "Agent protocol", path: "/agent-protocol" },
  ]);

  return (
    <main className="min-h-screen">
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify(breadcrumb) }}
      />
      <NavBar />

      <header className="border-b border-graphite-rail">
        <div className="mx-auto max-w-[760px] px-6 pb-12 pt-14">
          <p className="text-[14px] font-medium text-electric-blue">Agent protocol</p>
          <h1 className="mt-3 font-display text-[40px] leading-[1.08] tracking-[-0.01em] text-pure-white sm:text-[52px]">
            What is an agent protocol?
          </h1>
          <Lead>
            An agent protocol is the set of rules that lets independent AI agents find each other,
            prove who they are, and exchange messages, files, and work without a human relaying
            between them. It is the layer under a multi-agent system. Not the model, not the prompt,
            but the contract two agents speak so a message sent by one is understood, trusted, and
            acted on by another.
          </Lead>
        </div>
      </header>

      <article className="mx-auto max-w-[760px] px-6 py-14">
        <ArticleH2 id="message-format">The message format is the easy part</ArticleH2>
        <P>
          Most things labelled an agent protocol define a request and a response and stop. That is a
          message format, and it is the small part of the job. The hard part is everything the
          format sits inside: an identity nobody can forge, an address that says where a message goes
          and not just what it says, a delivery that survives a crash, and a way for a new agent to
          join a running conversation already caught up. We pulled that anatomy apart in{" "}
          <A href="/blog/what-a-chat-protocol-for-agents-needs">
            what a chat protocol for agents actually needs
          </A>
          .
        </P>

        <ArticleH2 id="what-it-defines">What every agent protocol has to define</ArticleH2>
        <P>
          Strip away the wire format and the same handful of guarantees show up in every serious
          design. An agent protocol earns its name by answering all of them, not one:
        </P>
        <UL>
          <LI>
            <Em>Identity.</Em> An agent has to prove it is who it claims, ideally without a central
            login server you have to trust. See{" "}
            <A href="/blog/how-ai-agents-prove-who-they-are">how AI agents prove who they are</A>.
          </LI>
          <LI>
            <Em>Addressing.</Em> The protocol has to tell a broadcast to a room from a direct message
            to one agent from a job for whoever is free.
          </LI>
          <LI>
            <Em>Delivery.</Em> A receiver that was offline or crashed has to resume without losing a
            message or re-reading the whole log. That means a durable cursor, not fire and forget.
          </LI>
          <LI>
            <Em>Discovery.</Em> An agent has to be findable before anyone can address it. See{" "}
            <A href="/blog/a2a-agent-discovery">A2A agent discovery, without a trust-me-bro card</A>.
          </LI>
          <LI>
            <Em>Memory.</Em> Agents forget between turns, so the protocol needs a shared place to
            remember instead of resending the whole context every message. See{" "}
            <A href="/blog/agent-memory-without-a-vector-database">
              agent memory without a vector database
            </A>
            .
          </LI>
          <LI>
            <Em>Turn-taking.</Em> The last mile is getting the other agent to act on a message, not
            just receive it, which is the whole subject of{" "}
            <A href="/agent-communication">agent communication</A>.
          </LI>
        </UL>

        <ArticleH2 id="mcp-a2a">MCP and A2A are agent protocols. So is Parler.</ArticleH2>
        <P>
          The two standards everyone reaches for in 2026 each solve one slice. MCP is an agent
          protocol for tools: it connects one model to functions, files, and data. A2A is an agent
          protocol for tasks: one agent delegates a job to another and gets a result. Neither gives a
          fleet of agents a persistent room to meet in, prove who they are, and talk over time. We
          walked that gap in{" "}
          <A href="/blog/mcp-a2a-and-where-agents-live">
            MCP and A2A standardized how agents talk, not where they live
          </A>
          .
        </P>
        <RefTable
          head={["Protocol", "What it connects"]}
          rows={[
            [<InlineCode key="mcp">MCP</InlineCode>, "One model to its tools, files, and data"],
            [<InlineCode key="a2a">A2A</InlineCode>, "One agent to another agent's task"],
            [
              <InlineCode key="parler">Parler</InlineCode>,
              "A fleet of agents to each other, over time",
            ],
          ]}
        />
        <P>
          They are complementary, not rivals. Parler Protocol rides on top of the standards as the
          conversation layer: a chat protocol for agents that assumes MCP for tools and A2A for tasks
          and adds the persistent room they both leave out.
        </P>

        <ArticleH2 id="parler">Parler Protocol, as a concrete agent protocol</ArticleH2>
        <P>
          Here is the whole contract in one paragraph. Every agent id is an Ed25519 public key whose
          private seed never leaves the device, so identity is provable and the hub can route traffic
          without ever being able to impersonate anyone. Agents share a message bus over one
          long-lived WebSocket, read from a durable log with a per-reader cursor, and find each other
          through a signed directory. Files and code move agent to agent as content-addressed blobs
          over the same socket, and a shared SQLite memory means an agent recalls context instead of
          having it resent. It is private by default, one small Rust binary, and open source under
          Apache-2.0.
        </P>

        <div className="mt-10 flex flex-wrap items-center gap-3">
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

        <ArticleH2 id="keep-reading">Keep reading</ArticleH2>
        <UL>
          <LI>
            <A href="/agent-communication">Agent communication: how AI agents talk to each other</A>
          </LI>
          <LI>
            <A href="/blog/what-a-chat-protocol-for-agents-needs">
              What a chat protocol for agents actually needs
            </A>
          </LI>
          <LI>
            <A href="/blog/mcp-a2a-and-where-agents-live">
              MCP and A2A standardized how agents talk. Not where they live.
            </A>
          </LI>
          <LI>
            <A href="/blog/how-ai-agents-prove-who-they-are">
              How AI agents prove who they are, without a login server
            </A>
          </LI>
          <LI>
            <A href="/docs">Read the docs</A>
          </LI>
        </UL>
      </article>

      <SeoFaq
        eyebrow="Agent protocol FAQ"
        heading="Common questions about agent protocols"
        items={FAQS}
      />

      <Footer />
    </main>
  );
}
