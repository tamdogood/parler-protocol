import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Eye,
  KeyRound,
  Lock,
  Package,
  Paperclip,
  ServerCrash,
  ShieldCheck,
  Users,
  Play,
  Pause,
  RotateCcw,
  ChevronRight,
  ChevronLeft,
  AlertCircle,
  Terminal,
} from "lucide-react";
import type { SessionAgent, SessionMessage, SessionPart, SessionView } from "@/lib/types";
import { fetchSession, HubError } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { StatusDot, statusMeta } from "@/components/status-dot";

const POLL_MS = 4000;

/**
 * Read-only session viewer, gated by a watch token. Ported from the website; the only differences are
 * the dynamic hub `base` (the desktop app can point at the local or public hub) and an `initialToken`
 * so opening a session in-app can hand its watch code straight to the viewer. The token lives in
 * memory only.
 */
export function SessionViewer({ base, initialToken }: { base: string; initialToken?: string }) {
  const [token, setToken] = useState("");
  const [draft, setDraft] = useState("");
  const [view, setView] = useState<SessionView | null>(null);
  const [messages, setMessages] = useState<SessionMessage[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [unauthorized, setUnauthorized] = useState(false);
  const cursor = useRef(0);

  // An in-app "open session" flow can seed the viewer with the freshly minted watch code.
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

  const connected = !!view && !unauthorized;

  return (
    <div>
      <div className="flex items-center gap-3">
        <span className="flex size-10 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
          <Eye className="size-5 text-electric-blue" />
        </span>
        <div>
          <h2 className="text-[20px] font-semibold leading-tight tracking-[-0.02em] text-pure-white">
            Watch a live session
          </h2>
          <p className="text-[13px] text-fog">
            Paste a watch code to see the whole conversation and how many agents are in the room.
          </p>
        </div>
      </div>

      {!connected && (
        <div className="mt-6 rounded-[16px] border border-graphite-rail bg-void-black p-6">
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

          <HowToGetACode base={base} />
        </div>
      )}

      {connected && view && (
        <ConnectedView view={view} messages={messages} error={error} onDisconnect={reset} />
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
  const listContainerRef = useRef<HTMLDivElement>(null);

  const [viewMode, setViewMode] = useState<"chat" | "timeline">("chat");
  const [replayIndex, setReplayIndex] = useState(0);
  const [isPlaying, setIsPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);

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
    if (viewMode === "chat") endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [list.length, viewMode]);

  useEffect(() => {
    if (!isPlaying || viewMode !== "timeline" || list.length === 0) return;
    const interval = setInterval(() => {
      setReplayIndex((prev) => {
        if (prev >= list.length - 1) {
          setIsPlaying(false);
          return prev;
        }
        return prev + 1;
      });
    }, 1500 / speed);
    return () => clearInterval(interval);
  }, [isPlaying, speed, viewMode, list.length]);

  useEffect(() => {
    if (viewMode !== "timeline") return;
    const container = listContainerRef.current;
    if (!container) return;
    const activeItem = container.querySelector(`[data-index="${replayIndex}"]`);
    activeItem?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }, [replayIndex, viewMode]);

  const activeMessage = list[replayIndex] || null;

  return (
    <div className="mt-6">
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-t-[16px] border border-graphite-rail bg-void-black px-5 py-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <StatusDot status={view.onlineCount > 0 ? "working" : "offline"} />
            <span className="truncate font-mono text-[13px] text-frost" data-selectable>
              {view.room}
            </span>
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

      {view.agents.length > 0 && (
        <div className="flex flex-wrap gap-2 border-x border-graphite-rail bg-black/20 px-5 py-3">
          {view.agents.map((a) => (
            <RosterChip key={a.name} agent={a} />
          ))}
        </div>
      )}

      <div className="flex border-x border-b border-graphite-rail bg-black/10 px-5 py-2">
        <button
          onClick={() => setViewMode("chat")}
          className={cn(
            "no-drag -mb-[9px] border-b-2 px-4 py-1.5 text-[13px] font-medium transition-colors focus:outline-none",
            viewMode === "chat"
              ? "border-electric-blue text-electric-blue"
              : "border-transparent text-steel hover:text-frost",
          )}
        >
          💬 Chat View
        </button>
        <button
          onClick={() => {
            setViewMode("timeline");
            if (list.length > 0) setReplayIndex(list.length - 1);
          }}
          className={cn(
            "no-drag -mb-[9px] ml-2 border-b-2 px-4 py-1.5 text-[13px] font-medium transition-colors focus:outline-none",
            viewMode === "timeline"
              ? "border-electric-blue text-electric-blue"
              : "border-transparent text-steel hover:text-frost",
          )}
        >
          ⏱️ Timeline Replay
        </button>
      </div>

      <div className="rounded-b-[16px] border border-t-0 border-graphite-rail bg-void-black px-5 py-5">
        {error && (
          <p className="mb-4 flex items-center gap-2 text-[12px] text-complained-yellow">
            <ServerCrash className="size-3.5" />
            Lost contact with the hub — retrying. ({error})
          </p>
        )}

        {viewMode === "chat" ? (
          list.length === 0 ? (
            <p className="py-10 text-center text-[14px] text-steel">No messages in this session yet.</p>
          ) : (
            <div className="flex flex-col gap-5" data-selectable>
              {list.map((m) => (
                <MessageRow key={m.seq} message={m} />
              ))}
              <div ref={endRef} />
            </div>
          )
        ) : list.length === 0 ? (
          <p className="py-10 text-center text-[14px] text-steel">No messages to replay.</p>
        ) : (
          <div className="flex flex-col gap-4">
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-[12px] border border-graphite-rail/60 bg-black/30 p-3">
              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  className="h-8 w-8 p-0"
                  disabled={replayIndex === 0}
                  onClick={() => {
                    setIsPlaying(false);
                    setReplayIndex(0);
                  }}
                >
                  <RotateCcw className="size-3.5" />
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-8 w-8 p-0"
                  disabled={replayIndex === 0}
                  onClick={() => {
                    setIsPlaying(false);
                    setReplayIndex((prev) => Math.max(0, prev - 1));
                  }}
                >
                  <ChevronLeft className="size-4" />
                </Button>
                <Button variant="primary" size="sm" className="h-8 px-3" onClick={() => setIsPlaying(!isPlaying)}>
                  {isPlaying ? <Pause className="mr-1 size-3.5" /> : <Play className="mr-1 size-3.5" />}
                  {isPlaying ? "Pause" : "Play"}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-8 w-8 p-0"
                  disabled={replayIndex >= list.length - 1}
                  onClick={() => {
                    setIsPlaying(false);
                    setReplayIndex((prev) => Math.min(list.length - 1, prev + 1));
                  }}
                >
                  <ChevronRight className="size-4" />
                </Button>
              </div>

              <div className="flex min-w-[150px] flex-1 items-center gap-3 px-2">
                <span className="font-mono text-[11px] text-steel">
                  Step {replayIndex + 1}/{list.length}
                </span>
                <input
                  type="range"
                  min="0"
                  max={list.length - 1}
                  value={replayIndex}
                  onChange={(e) => {
                    setIsPlaying(false);
                    setReplayIndex(parseInt(e.target.value, 10));
                  }}
                  className="no-drag h-1 flex-1 cursor-pointer appearance-none rounded-lg bg-graphite-rail accent-electric-blue"
                />
              </div>

              <div className="flex items-center gap-1.5">
                <span className="text-[11px] text-steel">Speed:</span>
                {[1, 2, 5].map((sp) => (
                  <button
                    key={sp}
                    onClick={() => setSpeed(sp)}
                    className={cn(
                      "no-drag rounded-[6px] border px-2 py-0.5 font-mono text-[11px] font-medium transition-colors",
                      speed === sp
                        ? "border-electric-blue/40 bg-electric-blue/10 text-frost"
                        : "border-graphite-rail bg-transparent text-steel hover:text-frost",
                    )}
                  >
                    {sp}x
                  </button>
                ))}
              </div>
            </div>

            <div className="mt-2 grid grid-cols-1 gap-4 md:grid-cols-3">
              <div
                ref={listContainerRef}
                className="flex max-h-[500px] flex-col gap-1 overflow-y-auto rounded-[12px] border border-graphite-rail bg-black/20 p-2"
              >
                {list.map((m, idx) => {
                  const isObs = m.parts.some((p) => p.kind === "com.parler.observation");
                  const toolPart = m.parts.find((p) => p.kind === "com.parler.observation");
                  const toolName = toolPart?.fields?.tool_name;
                  const isFailed = toolPart?.fields?.status === "failure";
                  return (
                    <button
                      key={m.seq}
                      data-index={idx}
                      onClick={() => {
                        setIsPlaying(false);
                        setReplayIndex(idx);
                      }}
                      className={cn(
                        "no-drag flex w-full items-start gap-2 rounded-[8px] border border-transparent p-2 text-left transition-colors focus:outline-none",
                        idx === replayIndex
                          ? "border-electric-blue/30 bg-electric-blue/10 text-frost"
                          : "text-steel hover:bg-white/5 hover:text-frost",
                      )}
                    >
                      <span className="mt-0.5 shrink-0 rounded bg-graphite-rail/60 px-1 font-mono text-[10px] text-steel">
                        {idx + 1}
                      </span>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center justify-between gap-1">
                          <span className="truncate text-[12px] font-medium text-frost">{m.from.name}</span>
                          <span className="shrink-0 font-mono text-[9px] text-steel">{fmtTime(m.ts)}</span>
                        </div>
                        <p className="mt-0.5 truncate text-[11px]">
                          {isObs ? (
                            <span
                              className={cn(
                                "inline-flex items-center gap-1",
                                isFailed ? "text-bounced-red" : "text-electric-blue",
                              )}
                            >
                              {isFailed ? <AlertCircle className="size-3" /> : <Terminal className="size-3" />}
                              <code>{toolName}</code>
                            </span>
                          ) : (
                            m.parts.map((p) => p.text).join(" ")
                          )}
                        </p>
                      </div>
                    </button>
                  );
                })}
              </div>

              <div className="max-h-[500px] min-h-[300px] overflow-y-auto rounded-[12px] border border-graphite-rail bg-black/40 p-4 md:col-span-2" data-selectable>
                {activeMessage ? (
                  <div>
                    <div className="flex items-center justify-between border-b border-graphite-rail pb-3">
                      <div>
                        <div className="flex items-center gap-2">
                          <span className="text-[15px] font-medium text-frost">{activeMessage.from.name}</span>
                          {activeMessage.from.role && (
                            <span className="rounded bg-graphite-rail/50 px-1.5 py-0.5 text-[11px] text-steel">
                              {activeMessage.from.role}
                            </span>
                          )}
                        </div>
                        <p className="mt-1 font-mono text-[11px] text-steel">Sequence: {activeMessage.seq}</p>
                      </div>
                      <span className="font-mono text-[11px] text-steel">{new Date(activeMessage.ts).toLocaleString()}</span>
                    </div>
                    <div className="mt-4 text-[14px] leading-relaxed">
                      {activeMessage.parts.map((p, i) => (
                        <PartView key={i} part={p} />
                      ))}
                    </div>
                  </div>
                ) : (
                  <p className="py-20 text-center text-[13px] text-steel">Select a step to view details.</p>
                )}
              </div>
            </div>
          </div>
        )}
      </div>

      <p className="mt-4 flex items-center gap-2 text-[12px] text-steel">
        <ShieldCheck className="size-3.5 text-delivered-green" />
        Read-only viewer. The watch code can only read this one room — it can&apos;t post, join, or reach any
        other session.
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

function HowToGetACode({ base }: { base: string }) {
  return (
    <div className="mt-6 border-t border-graphite-rail pt-5">
      <p className="text-[13px] font-medium text-frost">Don&apos;t have a code?</p>
      <p className="mt-1.5 text-[13px] leading-relaxed text-fog">
        A watch code is minted by the agent that <em>opened</em> the session — separate from the join key, so a
        shared key can never quietly expose the conversation. Open one from the <b className="text-frost">Sessions</b>
        {" "}panel above, or from an agent:
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
      <p className="mt-4 font-mono text-[11px] text-steel">{base}</p>
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
