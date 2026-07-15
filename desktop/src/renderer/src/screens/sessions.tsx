import { useCallback, useEffect, useRef, useState } from "react";
import {
  MessagesSquare,
  Plus,
  KeyRound,
  Eye,
  Loader2,
  ChevronDown,
  Lock,
  Unlock,
  Check,
  X,
  Trash2,
  UserPlus,
  ServerOff,
  Share2,
} from "lucide-react";
import type { HubStatus, OpenedSessionRecord, SessionJoinRequest } from "@shared/types";
import { conversationJoinCommand } from "@shared/conversation";
import { parler } from "@/lib/ipc";
import { cn, relativeTime, sessionOnActiveHub } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { CopyButton } from "@/components/copyable";
import { SessionViewer } from "@/components/session-viewer";

/**
 * Conversations = the whole mid-chat handoff lifecycle in one place. Internal session/room names
 * remain compatibility details; the UI exposes one conversation key and one exact viewer code.
 * Opens on your local hub when it's running, else the public hub.
 */
export function SessionsScreen({
  localUrl,
  publicUrl,
  status,
}: {
  localUrl: string | null;
  publicUrl: string;
  status: HubStatus | null;
}) {
  const openOnLocal = status?.phase === "running";
  const openBase = openOnLocal ? localUrl ?? publicUrl : publicUrl;

  const [sessions, setSessions] = useState<OpenedSessionRecord[] | null>(null);
  const [watchToken, setWatchToken] = useState<string | undefined>(undefined);
  const [highlight, setHighlight] = useState<string | null>(null);
  const viewerRef = useRef<HTMLDivElement>(null);

  const refresh = useCallback(async () => {
    setSessions(await parler.session.list());
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // When a conversation card sends its viewer code across, bring the viewer into view.
  const watchHere = useCallback((token: string) => {
    setWatchToken(token);
    requestAnimationFrame(() => viewerRef.current?.scrollIntoView({ behavior: "smooth", block: "start" }));
  }, []);

  const onOpened = useCallback(
    (room: string) => {
      setHighlight(room);
      void refresh();
    },
    [refresh],
  );

  return (
    <div className="mx-auto max-w-[820px] px-8 py-8">
      <div className="flex items-center gap-3">
        <span className="flex size-11 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
          <MessagesSquare className="size-5 text-electric-blue" />
        </span>
        <div>
          <h1 className="text-[22px] font-semibold tracking-tight text-pure-white">Conversations</h1>
          <p className="text-[13px] text-fog">Share one live conversation, let visible agents join, and watch the exact same roster.</p>
        </div>
      </div>

      <div className="mt-6">
        <OpenSessionPanel openOnLocal={openOnLocal} onOpened={onOpened} />
      </div>

      <SessionList
        sessions={sessions}
        status={status}
        highlight={highlight}
        onWatch={watchHere}
        onChanged={refresh}
      />

      <div className="mt-6" ref={viewerRef}>
        <p className="mb-2 text-[12px] uppercase tracking-wide text-steel">Watch any conversation</p>
        <SessionViewer base={openBase} initialToken={watchToken} />
      </div>
    </div>
  );
}

/** The form that opens a new conversation. */
function OpenSessionPanel({ openOnLocal, onOpened }: { openOnLocal: boolean; onOpened: (room: string) => void }) {
  const [context, setContext] = useState("");
  const [topic, setTopic] = useState("");
  const [requireApproval, setRequireApproval] = useState(false);
  const [showOptions, setShowOptions] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const open = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await parler.session.open({
        context: context.trim() || undefined,
        topic: topic.trim() || undefined,
        noApproval: !requireApproval,
      });
      setContext("");
      setTopic("");
      onOpened(r.room);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to open the conversation.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="rounded-[16px] border border-graphite-rail bg-void-black p-6">
      <div className="flex items-center gap-2 text-[14px] font-semibold text-frost">
        <Plus className="size-4 text-electric-blue" /> Open a conversation
      </div>
      <p className="mt-1 text-[13px] text-fog">
        Opens on {openOnLocal ? "your local hub" : "the public hub"}. The private key admits joiners immediately by default.
      </p>

      <textarea
        value={context}
        onChange={(e) => setContext(e.target.value)}
        rows={4}
        placeholder="Context recap: where we are, what's decided, what's next…"
        className="no-drag mt-4 w-full resize-y rounded-[10px] border border-graphite-rail bg-transparent px-3 py-2.5 text-[14px] text-frost placeholder:text-steel outline-none transition-colors focus:border-electric-blue/70 focus:ring-1 focus:ring-electric-blue/40"
      />

      <button
        onClick={() => setShowOptions((v) => !v)}
        className="no-drag mt-3 inline-flex items-center gap-1.5 text-[12.5px] text-steel transition-colors hover:text-frost"
      >
        <ChevronDown className={cn("size-3.5 transition-transform", showOptions && "rotate-180")} /> Options
      </button>
      {showOptions && (
        <div className="mt-3 flex flex-col gap-3 sm:flex-row sm:items-end">
          <label className="block flex-1">
            <span className="mb-1.5 block text-[11px] uppercase tracking-wide text-steel">Topic</span>
            <Input value={topic} onChange={(e) => setTopic(e.target.value)} placeholder="e.g. payments-refactor" />
          </label>
          <label className="no-drag flex cursor-pointer items-center gap-2 pb-2.5 text-[13px] text-fog">
            <input type="checkbox" checked={requireApproval} onChange={(e) => setRequireApproval(e.target.checked)} className="accent-electric-blue" />
            Require owner approval
          </label>
        </div>
      )}

      {error && <p className="mt-3 text-[13px] text-bounced-red">{error}</p>}

      <div className="mt-4">
        <Button variant="primary" onClick={open} disabled={busy}>
          {busy ? <Loader2 className="size-4 animate-spin" /> : <KeyRound className="size-4" />} Open conversation
        </Button>
      </div>
    </div>
  );
}

function SessionList({
  sessions,
  status,
  highlight,
  onWatch,
  onChanged,
}: {
  sessions: OpenedSessionRecord[] | null;
  status: HubStatus | null;
  highlight: string | null;
  onWatch: (token: string) => void;
  onChanged: () => void;
}) {
  if (sessions === null) {
    return (
      <div className="mt-5 flex items-center gap-2 text-[13px] text-steel">
        <Loader2 className="size-4 animate-spin" /> Loading your conversations…
      </div>
    );
  }
  if (sessions.length === 0) return null;

  return (
    <div className="mt-6">
      <p className="mb-2 text-[12px] uppercase tracking-wide text-steel">Your conversations</p>
      <div className="flex flex-col gap-3">
        {sessions.map((s) => (
          <SessionCard
            key={s.room}
            session={s}
            live={sessionOnActiveHub(s, status)}
            highlighted={s.room === highlight}
            onWatch={onWatch}
            onChanged={onChanged}
          />
        ))}
      </div>
    </div>
  );
}

const REQ_POLL_MS = 6000;

function SessionCard({
  session,
  live,
  highlighted,
  onWatch,
  onChanged,
}: {
  session: OpenedSessionRecord;
  live: boolean;
  highlighted: boolean;
  onWatch: (token: string) => void;
  onChanged: () => void;
}) {
  const [requests, setRequests] = useState<SessionJoinRequest[]>([]);
  const [acting, setActing] = useState<string | null>(null);

  // Poll pending join requests while this session is approval-gated *and* on the active hub —
  // the only time we can actually resolve them. Each poll is one short-lived CLI call.
  const pollable = live && session.approval;
  const load = useCallback(async () => {
    if (!pollable) return;
    try {
      setRequests(await parler.session.requests(session.room));
    } catch {
      /* hub blip / not reachable — keep the last known list */
    }
  }, [pollable, session.room]);

  useEffect(() => {
    if (!pollable) {
      setRequests([]);
      return;
    }
    load();
    const id = setInterval(load, REQ_POLL_MS);
    return () => clearInterval(id);
  }, [pollable, load]);

  const resolve = async (agent: string, approve: boolean) => {
    setActing(agent);
    try {
      const res = approve ? await parler.session.approve(session.room, agent) : await parler.session.deny(session.room, agent);
      if (res.ok) setRequests((prev) => prev.filter((r) => r.agent !== agent));
      await load();
    } finally {
      setActing(null);
    }
  };

  const forget = async () => {
    await parler.session.forget(session.room);
    onChanged();
  };

  const pending = requests.length;
  const joinCommand = conversationJoinCommand(session.key, session.hub);

  return (
    <div
      className={cn(
        "rounded-[16px] border bg-void-black p-5 transition-colors",
        pending > 0 ? "border-complained-yellow/40" : highlighted ? "border-electric-blue/40" : "border-graphite-rail",
      )}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate font-mono text-[13px] text-frost" data-selectable>
              {session.topic || session.room}
            </span>
            {session.approval ? (
              <span className="inline-flex items-center gap-1 rounded-[6px] border border-graphite-rail px-1.5 py-0.5 text-[11px] text-steel">
                <Lock className="size-3" /> Approval
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 rounded-[6px] border border-graphite-rail px-1.5 py-0.5 text-[11px] text-steel">
                <Unlock className="size-3" /> Open join
              </span>
            )}
          </div>
          <p className="mt-1 text-[12px] text-steel">
            Opened {relativeTime(session.createdAt)}
            {!live && (
              <span className="ml-2 inline-flex items-center gap-1 text-steel">
                <ServerOff className="size-3" /> on your {/127\.0\.0\.1|localhost/.test(session.hub) ? "local hub" : "other hub"} — start it to manage
              </span>
            )}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {live && session.watch && (
            <Button variant="outline" size="sm" onClick={() => onWatch(session.watch as string)}>
              <Eye className="size-3.5" /> Watch
            </Button>
          )}
          <button
            onClick={forget}
            title="Forget this conversation (does not end it on the hub)"
            className="no-drag rounded-[6px] border border-graphite-rail p-1.5 text-steel transition-colors hover:border-bounced-red/40 hover:text-bounced-red"
          >
            <Trash2 className="size-3.5" />
          </button>
        </div>
      </div>

      {/* One key joins; one code watches this exact conversation read-only. */}
      <div className="mt-4 grid gap-2 sm:grid-cols-2">
        <TokenRow label="Join command" value={joinCommand} shareRoom={session.room} />
        {session.watch && <TokenRow label="Viewer code" value={session.watch} />}
      </div>

      {/* Pending joiners — approve/deny inline. Only shown for a live, approval-gated session. */}
      {pollable && (
        <div className="mt-4 border-t border-graphite-rail/70 pt-3">
          {pending === 0 ? (
            <p className="flex items-center gap-2 text-[12px] text-steel">
              <UserPlus className="size-3.5" /> Waiting for agents to ask to join — they appear here for approval.
            </p>
          ) : (
            <>
              <p className="mb-2 flex items-center gap-2 text-[12px] font-medium text-complained-yellow">
                <span className="relative inline-flex size-2">
                  <span className="absolute inline-flex size-full animate-ping rounded-full bg-complained-yellow/60" />
                  <span className="relative inline-flex size-2 rounded-full bg-complained-yellow" />
                </span>
                {pending} {pending === 1 ? "agent wants" : "agents want"} to join
              </p>
              <div className="flex flex-col gap-2">
                {requests.map((r) => (
                  <div key={r.agent} className="flex items-center justify-between gap-3 rounded-[10px] border border-graphite-rail bg-black/30 px-3 py-2">
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-[13px] font-medium text-frost">{r.name}</span>
                        {r.role && <span className="text-[12px] text-steel">{r.role}</span>}
                      </div>
                      <p className="truncate font-mono text-[11px] text-steel">{r.agent}</p>
                    </div>
                    <div className="flex shrink-0 items-center gap-1.5">
                      <Button variant="solid" size="sm" disabled={acting === r.agent} onClick={() => resolve(r.agent, true)}>
                        {acting === r.agent ? <Loader2 className="size-3.5 animate-spin" /> : <Check className="size-3.5" />} Approve
                      </Button>
                      <button
                        onClick={() => resolve(r.agent, false)}
                        disabled={acting === r.agent}
                        title="Deny"
                        className="no-drag rounded-[6px] border border-graphite-rail p-1.5 text-steel transition-colors hover:border-bounced-red/40 hover:text-bounced-red disabled:opacity-50"
                      >
                        <X className="size-3.5" />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

function TokenRow({ label, value, shareRoom }: { label: string; value: string; shareRoom?: string }) {
  return (
    <div className="min-w-0">
      <p className="mb-1 text-[11px] uppercase tracking-wide text-steel">{label}</p>
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded-[8px] border border-graphite-rail bg-black/40 px-2.5 py-1.5 font-mono text-[12px] text-mist" data-selectable>
          {value}
        </code>
        <CopyButton value={value} label="" />
        {shareRoom && parler.app.platform === "darwin" && (
          <button
            onClick={() => parler.session.share(shareRoom, value)}
            className="no-drag inline-flex items-center gap-1.5 rounded-[6px] border border-graphite-rail px-2 py-1 text-[12px] text-fog transition-colors hover:border-smoke hover:text-frost"
            title="Share with Mail, Messages, AirDrop, or another Mac app"
          >
            <Share2 className="size-3.5" /> Share
          </button>
        )}
      </div>
    </div>
  );
}
