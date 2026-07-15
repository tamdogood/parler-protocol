import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Eye,
  KeyRound,
  Lock,
  Package,
  Paperclip,
  ServerCrash,
  ShieldCheck,
  Users,
  Coins,
  MessageSquare,
  Clock,
  Gauge,
  Download,
} from "lucide-react";
import type {
  SessionAgent,
  SessionFile,
  SessionMessage,
  SessionPart,
  SessionStats,
  SessionView,
} from "@/lib/types";
import { fetchSession, fetchSessionBlob, HubError } from "@/lib/api";
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
  const [downloadError, setDownloadError] = useState<string | null>(null);
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
    setDownloadError(null);
    setToken(t);
  };

  // Fetch the bytes for an exchanged file (watch-token gated) and save them locally. Streaming to an
  // object URL + a synthetic <a download> triggers the app's download of the exact bytes.
  const downloadFile = useCallback(
    async (file: SessionFile, name: string) => {
      if (!token) return;
      setDownloadError(null);
      try {
        const blob = await fetchSessionBlob(base, token, file.blob, name);
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = name;
        document.body.appendChild(a);
        a.click();
        a.remove();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
      } catch (e) {
        setDownloadError(e instanceof Error ? e.message : "Couldn't download that file.");
      }
    },
    [base, token],
  );

  if (view && !unauthorized) {
    return (
      <ConnectedView
        view={view}
        messages={messages}
        error={error}
        downloadError={downloadError}
        onDownload={downloadFile}
        onDisconnect={reset}
      />
    );
  }

  return (
    <div className="rounded-[16px] border border-graphite-rail bg-void-black p-6">
      <label className="text-[13px] font-medium text-frost">Watch a conversation</label>
      <p className="mt-0.5 text-[13px] text-fog">Paste its viewer code to follow that exact conversation live.</p>
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
  downloadError,
  onDownload,
  onDisconnect,
}: {
  view: SessionView;
  messages: SessionMessage[];
  error: string | null;
  downloadError: string | null;
  onDownload: (file: SessionFile, name: string) => void;
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

      {/* Activity metrics — how many tokens the agents have spent + who's doing the talking. */}
      {view.stats && <SessionStatsStrip stats={view.stats} />}

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
        {downloadError && (
          <p className="mb-4 flex items-center gap-2 text-[12px] text-bounced-red">
            <ServerCrash className="size-3.5" />
            {downloadError}
          </p>
        )}
        {list.length === 0 ? (
          <p className="py-10 text-center text-[14px] text-steel">No messages in this conversation yet.</p>
        ) : (
          <div className="flex max-h-[52vh] flex-col gap-5 overflow-y-auto" data-selectable>
            {list.map((m) => (
              <MessageRow key={m.seq} message={m} onDownload={onDownload} />
            ))}
            <div ref={endRef} />
          </div>
        )}
      </div>

      <p className="flex items-center gap-2 border-t border-graphite-rail px-5 py-3 text-[12px] text-steel">
        <ShieldCheck className="size-3.5 text-delivered-green" />
        Read-only — this code can only read this exact conversation.
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

function MessageRow({
  message,
  onDownload,
}: {
  message: SessionMessage;
  onDownload: (file: SessionFile, name: string) => void;
}) {
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
            <PartView key={i} part={p} onDownload={onDownload} />
          ))}
        </div>
      </div>
    </div>
  );
}

function PartView({
  part,
  onDownload,
}: {
  part: SessionPart;
  onDownload: (file: SessionFile, name: string) => void;
}) {
  if (part.kind === "text") {
    return <span className="whitespace-pre-wrap break-words">{part.text}</span>;
  }
  if (part.kind === "com.parler.bundle" || part.kind === "com.parler.file") {
    return <FilePart part={part} onDownload={onDownload} />;
  }
  const label = part.kind === "data" ? "data" : part.kind;
  return (
    <span className="inline-flex items-center gap-1.5 rounded-[8px] border border-graphite-rail bg-black/30 px-2 py-1 text-[12px] text-fog">
      <Paperclip className="size-3.5 text-electric-blue" />
      {label}
    </span>
  );
}

/**
 * A file the session exchanged — a code bundle (`com.parler.bundle`) or a handed-off file
 * (`com.parler.file`) — rendered as a card: name, size + type, and a **Download** button that pulls
 * the exact bytes (watch-token gated). Metadata-only parts (an older hub, no `file`) still render the
 * name/type without a download.
 */
function FilePart({
  part,
  onDownload,
}: {
  part: SessionPart;
  onDownload: (file: SessionFile, name: string) => void;
}) {
  const file = part.file;
  const isBundle = part.kind === "com.parler.bundle";
  const Icon = isBundle ? Package : Paperclip;
  const name = fileDisplayName(part);
  const meta = [file ? fmtBytes(file.size) : null, isBundle ? `${file?.vcs ?? "code"} bundle` : file?.mediaType]
    .filter(Boolean)
    .join(" · ");
  return (
    <span
      className="inline-flex max-w-full items-center gap-2 rounded-[10px] border border-graphite-rail bg-black/30 px-2.5 py-1.5"
      title={file?.summary || name}
    >
      <Icon className="size-4 shrink-0 text-electric-blue" />
      <span className="min-w-0">
        <span className="block truncate text-[13px] text-frost">{name}</span>
        {meta && <span className="block truncate text-[11px] text-steel">{meta}</span>}
      </span>
      {file && (
        <button
          type="button"
          onClick={() => onDownload(file, name)}
          className="ml-1 inline-flex shrink-0 items-center gap-1 rounded-[8px] border border-electric-blue/40 bg-electric-blue/10 px-2 py-1 text-[11px] text-frost transition-colors hover:bg-electric-blue/20"
          title="Download this file"
        >
          <Download className="size-3.5" />
          Download
        </button>
      )}
    </span>
  );
}

/** A human filename for a file/bundle part: the handoff's basename, else a name derived from the
 * bundle kind + short content id (a code bundle carries no filename of its own). */
function fileDisplayName(part: SessionPart): string {
  const file = part.file;
  if (file?.name) return file.name;
  const short = file?.blob?.slice(0, 8) ?? "";
  if (part.kind === "com.parler.bundle") {
    const ext = file?.vcs === "patch" ? "patch" : file?.vcs === "tar" ? "tar" : "bundle";
    return short ? `handoff-${short}.${ext}` : "code bundle";
  }
  return short ? `file-${short}` : "file";
}

/** Compact byte size: 900 → "900 B", 20000 → "20 KB", 5_000_000 → "4.8 MB". */
function fmtBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "";
  if (n < 1024) return `${n} B`;
  const kb = n / 1024;
  if (kb < 1024) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`;
  const mb = kb / 1024;
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
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

/**
 * The activity panel: how many tokens the agents have spent talking (the headline ask), plus message
 * count, activity span, tokens/message, and a per-agent "who's talking" breakdown. Token figures are
 * estimates the hub computes from message text — it relays text, it doesn't run the model — so this is
 * directional insight, not a billed count (hence the `≈` and the footnote).
 */
function SessionStatsStrip({ stats }: { stats: SessionStats }) {
  const span =
    stats.firstMessageAt && stats.lastMessageAt
      ? Math.max(0, stats.lastMessageAt - stats.firstMessageAt)
      : 0;
  const avg = stats.messages > 0 ? Math.round(stats.estimatedTokens / stats.messages) : 0;
  const top = stats.perAgent.slice(0, 5);
  const maxTokens = top.reduce((m, a) => Math.max(m, a.estimatedTokens), 0) || 1;

  return (
    <div className="border-b border-graphite-rail bg-black/20 px-5 py-4">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <StatTile
          icon={<Coins className="size-3.5 text-electric-blue" />}
          label="Est. tokens"
          value={`≈ ${fmtNum(stats.estimatedTokens)}`}
          hint="spent communicating"
        />
        <StatTile
          icon={<MessageSquare className="size-3.5 text-electric-blue" />}
          label="Messages"
          value={fmtNum(stats.messages)}
        />
        <StatTile
          icon={<Gauge className="size-3.5 text-electric-blue" />}
          label="Tokens / msg"
          value={`≈ ${fmtNum(avg)}`}
        />
        <StatTile
          icon={<Clock className="size-3.5 text-electric-blue" />}
          label="Active for"
          value={fmtDuration(span)}
        />
      </div>

      {top.length > 0 && (
        <div className="mt-4">
          <p className="text-[11px] uppercase tracking-wide text-steel">Who&apos;s talking · estimated tokens</p>
          <div className="mt-2 flex flex-col gap-1.5">
            {top.map((a) => (
              <div key={`${a.name}-${a.role ?? ""}`} className="flex items-center gap-3">
                <span className="w-28 shrink-0 truncate text-[12px] text-frost">
                  {a.name}
                  {a.role && <span className="text-steel"> · {a.role}</span>}
                </span>
                <div className="relative h-2 flex-1 overflow-hidden rounded-full bg-graphite-rail/50">
                  <div
                    className="absolute inset-y-0 left-0 rounded-full bg-electric-blue/60"
                    style={{ width: `${Math.max(3, (a.estimatedTokens / maxTokens) * 100)}%` }}
                  />
                </div>
                <span className="w-28 shrink-0 text-right font-mono text-[11px] text-fog">
                  ≈ {fmtNum(a.estimatedTokens)}
                  <span className="text-steel"> · {fmtNum(a.messages)} msg</span>
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      <p className="mt-3 text-[11px] leading-relaxed text-steel">
        Token counts are estimated from message text (~4 chars/token). The hub relays text, so this is a
        directional cost signal, not a model&apos;s exact billing.
      </p>
    </div>
  );
}

function StatTile({
  icon,
  label,
  value,
  hint,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  hint?: string;
}) {
  return (
    <div className="rounded-[12px] border border-graphite-rail bg-void-black px-3 py-2.5">
      <div className="flex items-center gap-1.5 text-[11px] text-steel">
        {icon}
        {label}
      </div>
      <div className="mt-1 text-[18px] font-semibold leading-none text-pure-white">{value}</div>
      {hint && <div className="mt-1 text-[10px] text-steel">{hint}</div>}
    </div>
  );
}

/** Compact number: 1234 → "1.2k", 2_500_000 → "2.5M". Small values stay exact. */
function fmtNum(n: number): string {
  if (!Number.isFinite(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 10_000) return `${Math.round(n / 1_000)}k`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return `${n}`;
}

/** Human span: "45s", "12m", "1h 3m", "2d 4h". `—` for an empty/zero span. */
function fmtDuration(ms: number): string {
  if (ms <= 0) return "—";
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) {
    const rem = m % 60;
    return rem ? `${h}h ${rem}m` : `${h}h`;
  }
  const d = Math.floor(h / 24);
  return `${d}d ${h % 24}h`;
}
