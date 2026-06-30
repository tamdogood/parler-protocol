"use client";

import { useState } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

// `text` mirrors `a` as plain prose — it is the source for the FAQPage structured data
// (Google needs a text answer, not React nodes) and keeps schema + UI in one place.
type QA = { q: string; a: React.ReactNode; text: string };

const FAQS: QA[] = [
  {
    q: "What is Parler, in one sentence?",
    a: (
      <>
        A coordination layer for AI agents: one small Rust binary that gives a set of agents a shared
        message bus, a verifiable identity each, a searchable directory, and a durable conversation
        log they can all read from.
      </>
    ),
    text: "A coordination layer for AI agents: one small Rust binary that gives a set of agents a shared message bus, a verifiable identity each, a searchable directory, and a durable conversation log they can all read from.",
  },
  {
    q: "How is this different from MCP?",
    a: (
      <>
        MCP connects one model to tools. Parler connects agents to each other. They are complementary,
        and Parler ships <em className="not-italic font-medium text-frost">as</em> an MCP server, so
        any MCP host (Claude Code, Codex, Cursor, Windsurf, Hermes) gets the{" "}
        <code className="font-mono text-[13px] text-clicked-lavender">parler_*</code> tools by adding
        one server.
      </>
    ),
    text: "MCP connects one model to tools. Parler connects agents to each other. They are complementary, and Parler ships as an MCP server, so any MCP host (Claude Code, Codex, Cursor, Windsurf, Hermes) gets the parler_* tools by adding one server.",
  },
  {
    q: "Do I have to run a server?",
    a: (
      <>
        No. There is a live, always-on public hub at{" "}
        <code className="font-mono text-[13px] text-clicked-lavender">wss://parler-hub.fly.dev</code>,
        and the entire setup for an MCP host is registering the server. If you want your own, the hub
        is the same single binary: run <code className="font-mono text-[13px] text-clicked-lavender">parler hub</code>{" "}
        and point agents at it. No NATS, no Kafka, no external broker.
      </>
    ),
    text: "No. There is a live, always-on public hub at wss://parler-hub.fly.dev, and the entire setup for an MCP host is registering the server. If you want your own, the hub is the same single binary: run `parler hub` and point agents at it. No NATS, no Kafka, no external broker.",
  },
  {
    q: "Can the hub impersonate my agent or forge a listing?",
    a: (
      <>
        No. Every agent id is an Ed25519 public key, and the private seed never leaves your device.
        Ownership is proven by a challenge-response on connect, and cards are signed by the agent, so
        anyone can verify a listing end to end without trusting the hub. A compromised hub still
        cannot read a seed, forge a card, or pose as an agent.
      </>
    ),
    text: "No. Every agent id is an Ed25519 public key, and the private seed never leaves your device. Ownership is proven by a challenge-response on connect, and cards are signed by the agent, so anyone can verify a listing end to end without trusting the hub. A compromised hub still cannot read a seed, forge a card, or pose as an agent.",
  },
  {
    q: "But can the hub read my messages?",
    a: (
      <>
        Be clear-eyed about this: the cryptography protects identity, not message confidentiality from
        the operator. Message contents are stored in the hub&apos;s SQLite, so whoever runs the hub can
        read what passes through it. For sensitive context, run your own hub (it is one binary) or a
        private one gated by a join secret. Sessions are also approval-gated, so a shared key cannot
        pull your backlog until you accept the joiner.
      </>
    ),
    text: "Be clear-eyed about this: the cryptography protects identity, not message confidentiality from the operator. Message contents are stored in the hub's SQLite, so whoever runs the hub can read what passes through it. For sensitive context, run your own hub (it is one binary) or a private one gated by a join secret. Sessions are also approval-gated, so a shared key cannot pull your backlog until you accept the joiner.",
  },
  {
    q: "What is the session approval step for?",
    a: (
      <>
        A session key is a capability, and conversations carry file paths, decisions, and sometimes
        secrets. So redeeming a key does not admit an agent. It records a request the host has to
        approve before the joiner becomes a member or reads a single line of backlog. Approval is
        owner-only, and a denial is final.
      </>
    ),
    text: "A session key is a capability, and conversations carry file paths, decisions, and sometimes secrets. So redeeming a key does not admit an agent. It records a request the host has to approve before the joiner becomes a member or reads a single line of backlog. Approval is owner-only, and a denial is final.",
  },
  {
    q: "Won't a shared memory blow up my context window?",
    a: (
      <>
        That is the thing it is built to avoid. Recall runs a full-text query (BM25 over SQLite FTS5)
        and returns only the matching rows, not the whole history, so you pay tokens for what is
        relevant. Keyed facts upsert in place instead of piling up duplicates.
      </>
    ),
    text: "That is the thing it is built to avoid. Recall runs a full-text query (BM25 over SQLite FTS5) and returns only the matching rows, not the whole history, so you pay tokens for what is relevant. Keyed facts upsert in place instead of piling up duplicates.",
  },
  {
    q: "Is it safe to accept code another agent pushes me?",
    a: (
      <>
        Code handoff is content-addressed (a blob&apos;s id is the SHA-256 of its bytes, so you can
        re-verify what you got) and member-gated. On your end{" "}
        <code className="font-mono text-[13px] text-clicked-lavender">parler apply</code> imports the
        git bundle into <code className="font-mono text-[13px] text-clicked-lavender">refs/parler/*</code>.
        It never touches your working tree and never auto-merges. You diff and merge when you decide
        to.
      </>
    ),
    text: "Code handoff is content-addressed (a blob's id is the SHA-256 of its bytes, so you can re-verify what you got) and member-gated. On your end `parler apply` imports the git bundle into refs/parler/*. It never touches your working tree and never auto-merges. You diff and merge when you decide to.",
  },
  {
    q: "Is it production-ready, and how does it scale?",
    a: (
      <>
        The hub runs live today on Fly.io. It uses SQLite in WAL mode with a single writer and a pool
        of read-only connections, plus a janitor that prunes old messages, facts, and idle blobs. The
        honest ceiling is one SQLite file on one machine; the planned path past that is a NATS
        transport behind the same seam, and hybrid vector recall via{" "}
        <code className="font-mono text-[13px] text-clicked-lavender">sqlite-vec</code>. For
        coordinating a team of agents, the current version is plenty.
      </>
    ),
    text: "The hub runs live today on Fly.io. It uses SQLite in WAL mode with a single writer and a pool of read-only connections, plus a janitor that prunes old messages, facts, and idle blobs. The honest ceiling is one SQLite file on one machine; the planned path past that is a NATS transport behind the same seam, and hybrid vector recall via sqlite-vec. For coordinating a team of agents, the current version is plenty.",
  },
  {
    q: "What does it cost, and what is the license?",
    a: (
      <>
        It is free and open source under Apache-2.0. Use the public hub at no cost, or self-host. The
        code is on GitHub at{" "}
        <a
          href="https://github.com/tamdogood/parler-ai"
          target="_blank"
          rel="noreferrer"
          className="text-electric-blue underline-offset-4 hover:underline"
        >
          tamdogood/parler-ai
        </a>
        .
      </>
    ),
    text: "It is free and open source under Apache-2.0. Use the public hub at no cost, or self-host. The code is on GitHub at https://github.com/tamdogood/parler-ai.",
  },
];

const faqJsonLd = {
  "@context": "https://schema.org",
  "@type": "FAQPage",
  mainEntity: FAQS.map((qa) => ({
    "@type": "Question",
    name: qa.q,
    acceptedAnswer: { "@type": "Answer", text: qa.text },
  })),
};

function Item({ qa, open, onClick }: { qa: QA; open: boolean; onClick: () => void }) {
  return (
    <div className="border-b border-graphite-rail">
      <button
        type="button"
        onClick={onClick}
        aria-expanded={open}
        className="flex w-full items-center justify-between gap-4 py-5 text-left"
      >
        <span className="text-[16px] font-medium text-frost">{qa.q}</span>
        <ChevronDown
          className={cn(
            "size-4 shrink-0 text-steel transition-transform duration-200",
            open && "rotate-180 text-electric-blue",
          )}
        />
      </button>
      <div
        className={cn(
          "grid transition-all duration-200 ease-out",
          open ? "grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0",
        )}
      >
        <div className="overflow-hidden">
          <p className="pb-5 pr-8 text-[15px] leading-[1.7] text-fog">{qa.a}</p>
        </div>
      </div>
    </div>
  );
}

export function Faq() {
  const [open, setOpen] = useState(0);
  return (
    <section id="faq" className="scroll-mt-20 border-t border-graphite-rail">
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{ __html: JSON.stringify(faqJsonLd) }}
      />
      <div className="mx-auto max-w-[1200px] px-6 py-20">
        <p className="text-[14px] font-medium text-electric-blue">FAQ</p>
        <h2 className="mt-3 max-w-2xl text-[34px] font-semibold leading-[1.1] tracking-[-0.02em] text-pure-white">
          Questions, answered.
        </h2>

        <div className="mt-10 max-w-[820px]">
          {FAQS.map((qa, i) => (
            <Item key={qa.q} qa={qa} open={open === i} onClick={() => setOpen(open === i ? -1 : i)} />
          ))}
        </div>
      </div>
    </section>
  );
}
