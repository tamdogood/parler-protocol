"use client";

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
import { fetchSession, HUB_API, HubError } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { StatusDot, statusMeta } from "@/components/status-dot";

const POLL_MS = 4000;

/**
 * Read-only session viewer, gated by a watch token. Lifted out of the old `/session` page so the
 * Hub's Sessions tab can embed it directly beneath the sessions explainer. The token still lives in
 * memory only (never localStorage), and a `#k=<token>` hash prefills + auto-connects.
 */
export function SessionViewer() {
  // The watch token lives in memory only — never localStorage — so closing the tab forgets it.
  const [token, setToken] = useState("");
  const [draft, setDraft] = useState("");
  const [view, setView] = useState<SessionView | null>(null);
  const [messages, setMessages] = useState<SessionMessage[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [unauthorized, setUnauthorized] = useState(false);
  const cursor = useRef(0);

  // A deliberately-crafted /hub#sessions&k=<token> link prefills the box and automatically connects.
  useEffect(() => {
    const h = typeof window !== "undefined" ? window.location.hash : "";
    const m = h.match(/[#&]k=([^&]+)/);
    if (m) {
      const decoded = decodeURIComponent(m[1]);
      setDraft(decoded);
      setToken(decoded);
    }
  }, []);

  const reset = useCallback(() => {
    setToken("");
    setView(null);
    setMessages([]);
    setError(null);
    setUnauthorized(false);
    cursor.current = 0;
    if (typeof window !== "undefined") {
      // replaceState (not location.hash) so we don't scroll-jump to the #sessions anchor.
      window.history.replaceState(null, "", "#sessions");
    }
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
    if (typeof window !== "undefined") {
      // Keep the token in the URL (so a refresh reconnects) without scroll-jumping to an anchor.
      window.history.replaceState(null, "", `#sessions&k=${encodeURIComponent(t)}`);
    }
  };

  const connected = !!view && !unauthorized;

  return (
    <section className="mx-auto max-w-[900px] px-6 py-12">
      <div className="flex items-center gap-3">
        <span className="flex size-10 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
          <Eye className="size-5 text-electric-blue" />
        </span>
        <div>
          <h2 className="text-[24px] font-semibold leading-tight tracking-[-0.02em] text-pure-white">
            Watch a live session
          </h2>
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
  const listContainerRef = useRef<HTMLDivElement>(null);

  // Replay timeline states
  const [viewMode, setViewMode] = useState<"chat" | "timeline">("chat");
  const [replayIndex, setReplayIndex] = useState(0);
  const [isPlaying, setIsPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);

  // Deduplicate adjacent duplicate boundary messages (🚀 Session started / 👋 Session ended)
  const list = useMemo(() => {
    const result: SessionMessage[] = [];
    let lastText = "";
    for (const m of messages) {
      const text = m.parts.map((p) => p.text || "").join(" ");
      const isBoundary = text.startsWith("🚀 Session started") || text === "👋 Session ended.";
      if (isBoundary && text === lastText) {
        continue;
      }
      result.push(m);
      if (isBoundary) {
        lastText = text;
      } else {
        lastText = ""; // Reset boundary tracking on normal messages
      }
    }
    return result;
  }, [messages]);

  useEffect(() => {
    if (viewMode === "chat") {
      endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
    }
  }, [list.length, viewMode]);

  // Replay timer
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

  // Auto-scroll the active step item in the left list container during replay
  useEffect(() => {
    if (viewMode !== "timeline") return;
    const container = listContainerRef.current;
    if (!container) return;
    const activeItem = container.querySelector(`[data-index="${replayIndex}"]`);
    if (activeItem) {
      activeItem.scrollIntoView({
        behavior: "smooth",
        block: "nearest",
      });
    }
  }, [replayIndex, viewMode]);

  const activeMessage = list[replayIndex] || null;

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

      {/* Mode Switcher */}
      <div className="flex border-x border-graphite-rail bg-black/10 px-5 py-2 border-b border-graphite-rail">
        <button
          onClick={() => setViewMode("chat")}
          className={cn(
            "px-4 py-1.5 text-[13px] font-medium transition-colors border-b-2 -mb-[9px] focus:outline-none",
            viewMode === "chat" ? "text-electric-blue border-electric-blue" : "text-steel border-transparent hover:text-frost"
          )}
        >
          💬 Chat View
        </button>
        <button
          onClick={() => {
            setViewMode("timeline");
            if (list.length > 0) {
              setReplayIndex(list.length - 1);
            }
          }}
          className={cn(
            "px-4 py-1.5 text-[13px] font-medium transition-colors border-b-2 -mb-[9px] ml-2 focus:outline-none",
            viewMode === "timeline" ? "text-electric-blue border-electric-blue" : "text-steel border-transparent hover:text-frost"
          )}
        >
          ⏱️ Timeline Replay
        </button>
      </div>

      {/* Conversation / Replay Panel. */}
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
            <div className="flex flex-col gap-5">
              {list.map((m) => (
                <MessageRow key={m.seq} message={m} />
              ))}
              <div ref={endRef} />
            </div>
          )
        ) : (
          /* Timeline Replay View */
          list.length === 0 ? (
            <p className="py-10 text-center text-[14px] text-steel">No messages to replay.</p>
          ) : (
            <div className="flex flex-col gap-4">
              {/* Controls bar */}
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
                  <Button
                    variant="primary"
                    size="sm"
                    className="h-8 px-3"
                    onClick={() => setIsPlaying(!isPlaying)}
                  >
                    {isPlaying ? <Pause className="size-3.5 mr-1" /> : <Play className="size-3.5 mr-1" />}
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

                {/* Scrubber slider */}
                <div className="flex flex-1 items-center gap-3 px-2 min-w-[150px]">
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
                    className="h-1 flex-1 cursor-pointer appearance-none rounded-lg bg-graphite-rail accent-electric-blue"
                  />
                </div>

                {/* Speed buttons */}
                <div className="flex items-center gap-1.5">
                  <span className="text-[11px] text-steel">Speed:</span>
                  {[1, 2, 5].map((sp) => (
                    <button
                      key={sp}
                      onClick={() => setSpeed(sp)}
                      className={cn(
                        "rounded-[6px] border px-2 py-0.5 font-mono text-[11px] font-medium transition-colors",
                        speed === sp
                          ? "border-electric-blue/40 bg-electric-blue/10 text-frost"
                          : "border-graphite-rail bg-transparent text-steel hover:text-frost"
                      )}
                    >
                      {sp}x
                    </button>
                  ))}
                </div>
              </div>

              {/* Grid content */}
              <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mt-2">
                {/* Left pane: steps list */}
                <div ref={listContainerRef} className="max-h-[500px] overflow-y-auto rounded-[12px] border border-graphite-rail bg-black/20 p-2 flex flex-col gap-1">
                  {list.map((m, idx) => {
                    const isObs = m.parts.some((p) => p.kind === "com.parler.observation");
                    const toolPart = m.parts.find((p) => p.kind === "com.parler.observation");
                    const toolName = toolPart?.fields?.tool_name;
                    const status = toolPart?.fields?.status;
                    const isFailed = status === "failure";

                    return (
                      <button
                        key={m.seq}
                        data-index={idx}
                        onClick={() => {
                          setIsPlaying(false);
                          setReplayIndex(idx);
                        }}
                        className={cn(
                          "w-full rounded-[8px] p-2 text-left transition-colors flex items-start gap-2 focus:outline-none border border-transparent",
                          idx === replayIndex
                            ? "bg-electric-blue/10 border-electric-blue/30 text-frost"
                            : "text-steel hover:bg-white/5 hover:text-frost"
                        )}
                      >
                        <span className="font-mono text-[10px] bg-graphite-rail/60 rounded px-1 text-steel shrink-0 mt-0.5">
                          {idx + 1}
                        </span>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center justify-between gap-1">
                            <span className="font-medium text-[12px] truncate text-frost">{m.from.name}</span>
                            <span className="font-mono text-[9px] text-steel shrink-0">{fmtTime(m.ts)}</span>
                          </div>
                          <p className="text-[11px] truncate mt-0.5">
                            {isObs ? (
                              <span className={cn(
                                "inline-flex items-center gap-1",
                                isFailed ? "text-bounced-red" : "text-electric-blue"
                              )}>
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

                {/* Right pane: step details */}
                <div className="md:col-span-2 min-h-[300px] max-h-[500px] overflow-y-auto rounded-[12px] border border-graphite-rail bg-black/40 p-4">
                  {activeMessage ? (
                    <div>
                      <div className="flex items-center justify-between border-b border-graphite-rail pb-3">
                        <div>
                          <div className="flex items-center gap-2">
                            <span className="font-medium text-[15px] text-frost">{activeMessage.from.name}</span>
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

                      <div className="mt-4 leading-relaxed text-[14px]">
                        {activeMessage.parts.map((p, i) => (
                          <PartView key={i} part={p} />
                        ))}
                      </div>
                    </div>
                  ) : (
                    <p className="text-center text-steel py-20 text-[13px]">Select a step to view details.</p>
                  )}
                </div>
              </div>
            </div>
          )
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
