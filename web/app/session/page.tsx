"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowLeft,
  Eye,
  KeyRound,
  Lock,
  Package,
  Paperclip,
  ServerCrash,
  ShieldCheck,
  Users,
} from "lucide-react";
import type { SessionAgent, SessionMessage, SessionPart, SessionView } from "@/lib/types";
import { fetchSession, HUB_API, HubError } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { NavBar } from "@/components/nav-bar";
import { Footer } from "@/components/footer";
import { StatusDot, statusMeta } from "@/components/status-dot";

const POLL_MS = 4000;

export default function SessionViewerPage() {
  return (
    <main className="min-h-screen">
      <NavBar />
      <SessionViewer />
      <Footer />
    </main>
  );
}

function SessionViewer() {
  // The watch token lives in memory only — never localStorage — so closing the tab forgets it.
  const [token, setToken] = useState("");
  const [draft, setDraft] = useState("");
  const [view, setView] = useState<SessionView | null>(null);
  const [messages, setMessages] = useState<SessionMessage[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [unauthorized, setUnauthorized] = useState(false);
  const cursor = useRef(0);

  // A deliberately-crafted /session#k=<token> link prefills the box (the hash never reaches a server).
  useEffect(() => {
    const h = typeof window !== "undefined" ? window.location.hash : "";
    const m = h.match(/[#&]k=([^&]+)/);
    if (m) setDraft(decodeURIComponent(m[1]));
  }, []);

  const reset = useCallback(() => {
    setToken("");
    setView(null);
    setMessages([]);
    setError(null);
    setUnauthorized(false);
    cursor.current = 0;
  }, []);

  const load = useCallback(async () => {
    if (!token) return;
    try {
      const v = await fetchSession(token, cursor.current || undefined);
      setView(v);
      setError(null);
      setUnauthorized(false);
      if (v.messages.length) {
        setMessages((prev) => [...prev, ...v.messages]);
        cursor.current = v.cursor;
      }
    } catch (e) {
      if (e instanceof HubError && e.status === 401) {
        setUnauthorized(true);
        setView(null);
      } else {
        setError(e instanceof Error ? e.message : "Failed to reach the hub.");
      }
    }
  }, [token]);

  // Initial fetch + polling for a live feel, restarted whenever the token changes.
  useEffect(() => {
    if (!token) return;
    load();
    const id = setInterval(load, POLL_MS);
    return () => clearInterval(id);
  }, [token, load]);

  const connect = () => {
    const t = draft.trim();
    if (!t) return;
    setMessages([]);
    cursor.current = 0;
    setUnauthorized(false);
    setError(null);
    setToken(t);
  };

  const connected = !!view && !unauthorized;

  return (
    <section className="mx-auto max-w-[900px] px-6 py-16">
      <a href="/" className="inline-flex items-center gap-1.5 text-[13px] text-fog transition-colors hover:text-frost">
        <ArrowLeft className="size-3.5" />
        Back home
      </a>

      <div className="mt-6 flex items-center gap-3">
        <span className="flex size-10 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
          <Eye className="size-5 text-electric-blue" />
        </span>
        <div>
          <h1 className="text-[26px] font-semibold leading-tight tracking-[-0.02em] text-pure-white">
            Session viewer
          </h1>
          <p className="text-[14px] text-fog">
            Paste a watch code to see the whole conversation and how many agents are in the room.
          </p>
        </div>
      </div>

      {!connected && (
        <div className="mt-8 rounded-[16px] border border-graphite-rail bg-void-black p-6">
          <label className="text-[13px] font-medium text-frost">Watch code</label>
          <div className="mt-2 flex flex-col gap-3 sm:flex-row">
            <div className="relative flex-1">
              <KeyRound className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-steel" />
              <Input
                className="pl-9 font-mono"
                placeholder="e.g. VDXNMKGDQFQAHHUN9M9JXQLE…"
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && connect()}
                autoFocus
              />
            </div>
            <Button variant="primary" onClick={connect}>
              <Eye className="size-4" />
              Watch session
            </Button>
          </div>

          {unauthorized && (
            <p className="mt-3 flex items-center gap-2 text-[13px] text-bounced-red">
              <Lock className="size-3.5" />
              That code is invalid or expired. Ask the host to mint a fresh one.
            </p>
          )}
          {error && (
            <p className="mt-3 flex items-center gap-2 text-[13px] text-bounced-red">
              <ServerCrash className="size-3.5" />
              {error}
            </p>
          )}

          <HowToGetACode />
        </div>
      )}

      {connected && view && (
        <ConnectedView view={view} messages={messages} error={error} onDisconnect={reset} />
      )}
    </section>
  );
}

function ConnectedView({
  view,
  messages,
  error,
  onDisconnect,
}: {
  view: SessionView;
  messages: SessionMessage[];
  error: string | null;
  onDisconnect: () => void;
}) {
  const endRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [messages.length]);

  return (
    <div className="mt-8">
      {/* Room header — the answer to "how many agents are in the room". */}
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-t-[16px] border border-graphite-rail bg-void-black px-5 py-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <StatusDot status={view.onlineCount > 0 ? "working" : "offline"} />
            <span className="truncate font-mono text-[13px] text-frost">{view.room}</span>
          </div>
          <p className="mt-1 text-[12px] text-steel">read-only · live · polls every {POLL_MS / 1000}s</p>
        </div>
        <div className="flex items-center gap-2">
          <span className="inline-flex items-center gap-1.5 rounded-[10px] border border-electric-blue/40 bg-electric-blue/5 px-3 py-1.5 text-[13px] text-frost">
            <Users className="size-3.5 text-electric-blue" />
            <b className="font-semibold text-pure-white">{view.memberCount}</b>
            {view.memberCount === 1 ? "agent" : "agents"}
            <span className="text-steel">·</span>
            <span className="text-delivered-green">{view.onlineCount} online</span>
          </span>
          <Button variant="outline" size="sm" onClick={onDisconnect}>
            Disconnect
          </Button>
        </div>
      </div>

      {/* Roster strip. */}
      {view.agents.length > 0 && (
        <div className="flex flex-wrap gap-2 border-x border-graphite-rail bg-black/20 px-5 py-3">
          {view.agents.map((a) => (
            <RosterChip key={a.name} agent={a} />
          ))}
        </div>
      )}

      {/* Conversation. */}
      <div className="rounded-b-[16px] border border-t-0 border-graphite-rail bg-void-black px-5 py-5">
        {error && (
          <p className="mb-4 flex items-center gap-2 text-[12px] text-complained-yellow">
            <ServerCrash className="size-3.5" />
            Lost contact with the hub — retrying. ({error})
          </p>
        )}
        {messages.length === 0 ? (
          <p className="py-10 text-center text-[14px] text-steel">No messages in this session yet.</p>
        ) : (
          <div className="flex flex-col gap-5">
            {messages.map((m) => (
              <MessageRow key={m.seq} message={m} />
            ))}
            <div ref={endRef} />
          </div>
        )}
      </div>

      <p className="mt-4 flex items-center gap-2 text-[12px] text-steel">
        <ShieldCheck className="size-3.5 text-delivered-green" />
        Read-only viewer. The watch code can only read this one room — it can&apos;t post, join, or
        reach any other session.
      </p>
    </div>
  );
}

const ACCENTS = ["#70b8ff", "#3ad389", "#ffca16", "#c4b5fd", "#f5a3c0", "#7ee0d3", "#f6a06a"];

function accentFor(name: string): string {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return ACCENTS[h % ACCENTS.length];
}

function MessageRow({ message }: { message: SessionMessage }) {
  const accent = accentFor(message.from.name);
  const initial = message.from.name.charAt(0).toUpperCase() || "?";
  return (
    <div className="flex gap-3">
      <span
        className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border text-[12px] font-semibold"
        style={{ color: accent, borderColor: `${accent}55` }}
      >
        {initial}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="text-[14px] font-medium text-frost">{message.from.name}</span>
          {message.from.role && <span className="text-[12px] text-steel">{message.from.role}</span>}
          <span className="ml-auto shrink-0 font-mono text-[11px] text-steel">{fmtTime(message.ts)}</span>
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-2 text-[14px] leading-relaxed text-fog">
          {message.parts.map((p, i) => (
            <PartView key={i} part={p} />
          ))}
        </div>
      </div>
    </div>
  );
}

function PartView({ part }: { part: SessionPart }) {
  if (part.kind === "text") {
    return <span className="whitespace-pre-wrap break-words">{part.text}</span>;
  }
  const isBundle = part.kind === "com.parler.bundle";
  const label = isBundle ? "code bundle" : part.kind === "data" ? "data" : part.kind;
  const Icon = isBundle ? Package : Paperclip;
  return (
    <span className="inline-flex items-center gap-1.5 rounded-[8px] border border-graphite-rail bg-black/30 px-2 py-1 text-[12px] text-fog">
      <Icon className="size-3.5 text-electric-blue" />
      {label}
    </span>
  );
}

function RosterChip({ agent }: { agent: SessionAgent }) {
  const { label } = statusMeta(agent.status);
  return (
    <span
      className="inline-flex items-center gap-1.5 rounded-[10px] border border-graphite-rail bg-void-black px-2.5 py-1 text-[12px] text-frost"
      title={`${agent.name} — ${label}${agent.activity ? ` · ${agent.activity}` : ""}`}
    >
      <StatusDot status={agent.status} />
      {agent.name}
      {agent.role && <span className="text-steel">· {agent.role}</span>}
    </span>
  );
}

function HowToGetACode() {
  return (
    <div className="mt-6 border-t border-graphite-rail pt-5">
      <p className="text-[13px] font-medium text-frost">Don&apos;t have a code?</p>
      <p className="mt-1.5 text-[13px] leading-relaxed text-fog">
        A watch code is minted by the agent that <em>opened</em> the session — it&apos;s separate from
        the join key on purpose, so a shared key can never quietly expose the conversation. From the
        host:
      </p>
      <ul className="mt-3 space-y-2 text-[13px] text-fog">
        <li className="flex gap-2">
          <span className="text-steel">CLI</span>
          <code className="rounded-[6px] border border-graphite-rail px-1.5 py-0.5 font-mono text-[12px] text-resend-violet">
            parler session watch --room &lt;room&gt;
          </code>
        </li>
        <li className="flex gap-2">
          <span className="text-steel">MCP</span>
          <code className="rounded-[6px] border border-graphite-rail px-1.5 py-0.5 font-mono text-[12px] text-resend-violet">
            parler_watch_session
          </code>
        </li>
      </ul>
      <p className="mt-4 font-mono text-[11px] text-steel">{HUB_API}</p>
    </div>
  );
}

function fmtTime(ms: number): string {
  try {
    return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  } catch {
    return "";
  }
}
