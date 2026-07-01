import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Eye, KeyRound, Lock, Package, Paperclip, ServerCrash, ShieldCheck, Users } from "lucide-react";
import type { SessionAgent, SessionMessage, SessionPart, SessionView } from "@/lib/types";
import { fetchSession, HubError } from "@/lib/api";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { StatusDot, statusMeta } from "@/components/status-dot";

const POLL_MS = 4000;
// This is a live view, not an archive — keep the tail bounded so a long-running watch can't grow the
// message buffer without limit.
const MAX_MESSAGES = 1000;

/**
 * Read-only session viewer, gated by a watch token. Deliberately minimal: connect with a code, then
 * a clean live chat + roster. (The old timeline-replay/scrubber mode was removed to declutter.)
 */
export function SessionViewer({ base, initialToken }: { base: string; initialToken?: string }) {
  const [token, setToken] = useState("");
  const [draft, setDraft] = useState("");
  const [view, setView] = useState<SessionView | null>(null);
  const [messages, setMessages] = useState<SessionMessage[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [unauthorized, setUnauthorized] = useState(false);
  const cursor = useRef(0);

  useEffect(() => {
    if (initialToken) {
      setDraft(initialToken);
      setMessages([]);
      cursor.current = 0;
      setUnauthorized(false);
      setError(null);
      setToken(initialToken);
    }
  }, [initialToken]);

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
      const v = await fetchSession(base, token, cursor.current || undefined);
      setView(v);
      setError(null);
      setUnauthorized(false);
      if (v.messages.length) {
        setMessages((prev) => [...prev, ...v.messages].slice(-MAX_MESSAGES));
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
  }, [token, base]);

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

  if (view && !unauthorized) {
    return <ConnectedView view={view} messages={messages} error={error} onDisconnect={reset} />;
  }

  return (
    <div className="rounded-[16px] border border-graphite-rail bg-void-black p-6">
      <label className="text-[13px] font-medium text-frost">Watch a session</label>
      <p className="mt-0.5 text-[13px] text-fog">Paste a watch code to follow the conversation live.</p>
      <div className="mt-3 flex flex-col gap-3 sm:flex-row">
        <div className="relative flex-1">
          <KeyRound className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-steel" />
          <Input
            className="pl-9 font-mono"
            placeholder="e.g. VDXNMKGDQFQAHHUN9M9JXQLE…"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && connect()}
          />
        </div>
        <Button variant="primary" onClick={connect}>
          <Eye className="size-4" />
          Watch
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
    </div>
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

  // Collapse repeated session-boundary lines.
  const list = useMemo(() => {
    const result: SessionMessage[] = [];
    let lastText = "";
    for (const m of messages) {
      const text = m.parts.map((p) => p.text || "").join(" ");
      const isBoundary = text.startsWith("🚀 Session started") || text === "👋 Session ended.";
      if (isBoundary && text === lastText) continue;
      result.push(m);
      lastText = isBoundary ? text : "";
    }
    return result;
  }, [messages]);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [list.length]);

  return (
    <div className="overflow-hidden rounded-[16px] border border-graphite-rail bg-void-black">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-graphite-rail px-5 py-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <StatusDot status={view.onlineCount > 0 ? "working" : "offline"} />
            <span className="truncate font-mono text-[13px] text-frost" data-selectable>
              {view.room}
            </span>
          </div>
          <p className="mt-1 text-[12px] text-steel">read-only · live</p>
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

      {view.agents.length > 0 && (
        <div className="flex flex-wrap gap-2 border-b border-graphite-rail bg-black/20 px-5 py-3">
          {view.agents.map((a) => (
            <RosterChip key={a.name} agent={a} />
          ))}
        </div>
      )}

      <div className="px-5 py-5">
        {error && (
          <p className="mb-4 flex items-center gap-2 text-[12px] text-complained-yellow">
            <ServerCrash className="size-3.5" />
            Lost contact with the hub — retrying.
          </p>
        )}
        {list.length === 0 ? (
          <p className="py-10 text-center text-[14px] text-steel">No messages in this session yet.</p>
        ) : (
          <div className="flex max-h-[52vh] flex-col gap-5 overflow-y-auto" data-selectable>
            {list.map((m) => (
              <MessageRow key={m.seq} message={m} />
            ))}
            <div ref={endRef} />
          </div>
        )}
      </div>

      <p className="flex items-center gap-2 border-t border-graphite-rail px-5 py-3 text-[12px] text-steel">
        <ShieldCheck className="size-3.5 text-delivered-green" />
        Read-only — this code can only read this one room.
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

function fmtTime(ms: number): string {
  try {
    return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  } catch {
    return "";
  }
}
