// The "Session Wrapped" model — a Spotify-Wrapped-style summary derived purely from the read-only
// `/api/session` payload the site already fetches (stats: estimated tokens, messages, activity span,
// per-agent breakdown; plus the loaded messages for tool-call flavor). No hub/protocol change: this is
// all client-side derivation over `SessionView`, so any watched session can produce a shareable card.

import type { SessionMessage, SessionView } from "./types";

/** One agent's slice of the session, ranked for the leaderboard (by estimated tokens spent). */
export interface WrappedAgentRank {
  name: string;
  role?: string;
  messages: number;
  tokens: number;
  /** Fraction of the room's total estimated tokens this agent spent (0..1; 0 when the room is empty). */
  share: number;
  /** 1-based rank, chattiest first. */
  rank: number;
}

/** A derived "personality" for the session — the playful headline badge, à la Wrapped. */
export interface WrappedVibe {
  emoji: string;
  title: string;
  blurb: string;
}

/** The whole scorecard model the canvas renderer and share controls read from. */
export interface Wrapped {
  room: string;
  /** Short, human-ish label for the footer (never the full room id). */
  roomLabel: string;
  totalTokens: number;
  totalMessages: number;
  /** Distinct agents that actually contributed (falls back to the live roster). */
  agentCount: number;
  onlineCount: number;
  durationMs: number;
  toolCalls: number;
  failedToolCalls: number;
  tokensPerMessage: number;
  /** Up to 5, chattiest first. */
  topAgents: WrappedAgentRank[];
  mvp: WrappedAgentRank | null;
  vibe: WrappedVibe;
  /** True when there are no messages yet — the card renders a friendly zero state. */
  isEmpty: boolean;
}

const OBSERVATION = "com.parler.observation";
const HOUR = 3_600_000;

/**
 * Fold a `SessionView` (and optionally the viewer's fuller accumulated message list) into the
 * scorecard model. Headline figures come from the whole-room `stats` aggregate; tool-call flavor is
 * counted from the loaded `messages` (a lower bound only if the backlog was truncated at the fetch
 * cap — rare for a single session). All token figures are the hub's estimates, never a billed count.
 */
export function buildWrapped(view: SessionView, messages?: SessionMessage[]): Wrapped {
  const msgs = messages ?? view.messages ?? [];
  const stats = view.stats;
  const perAgentSrc = stats?.perAgent ?? [];

  const totalTokens = stats?.estimatedTokens ?? perAgentSrc.reduce((s, a) => s + (a.estimatedTokens || 0), 0);
  const totalMessages = stats?.messages ?? msgs.length;

  const first = stats?.firstMessageAt ?? null;
  const last = stats?.lastMessageAt ?? null;
  const durationMs = first != null && last != null ? Math.max(0, last - first) : 0;

  let toolCalls = 0;
  let failedToolCalls = 0;
  for (const m of msgs) {
    for (const p of m.parts) {
      if (p.kind === OBSERVATION) {
        toolCalls++;
        if (p.fields?.status === "failure") failedToolCalls++;
      }
    }
  }

  const denom = totalTokens || 1;
  const topAgents: WrappedAgentRank[] = perAgentSrc.slice(0, 5).map((a, i) => ({
    name: a.name,
    role: a.role,
    messages: a.messages,
    tokens: a.estimatedTokens,
    share: a.estimatedTokens / denom,
    rank: i + 1,
  }));

  const agentCount = perAgentSrc.length || view.memberCount || view.agents.length;
  const tokensPerMessage = totalMessages > 0 ? Math.round(totalTokens / totalMessages) : 0;

  return {
    room: view.room,
    roomLabel: prettyRoom(view.room),
    totalTokens,
    totalMessages,
    agentCount,
    onlineCount: view.onlineCount,
    durationMs,
    toolCalls,
    failedToolCalls,
    tokensPerMessage,
    topAgents,
    mvp: topAgents[0] ?? null,
    vibe: deriveVibe({ agentCount, durationMs, tokensPerMessage, totalMessages, toolCalls }),
    isEmpty: totalMessages === 0,
  };
}

/** Pick the session's "personality" from its shape — first distinctive signal wins, for a stable badge. */
function deriveVibe(x: {
  agentCount: number;
  durationMs: number;
  tokensPerMessage: number;
  totalMessages: number;
  toolCalls: number;
}): WrappedVibe {
  if (x.agentCount >= 4)
    return { emoji: "🤝", title: "The Full Squad", blurb: "A whole crew showed up and shipped together." };
  if (x.durationMs >= 2 * HOUR)
    return { emoji: "🏃", title: "The Marathon", blurb: "Hours deep and still going strong." };
  if (x.toolCalls >= 30)
    return { emoji: "🛠️", title: "Tool Wizards", blurb: "More doing than talking." };
  if (x.tokensPerMessage >= 400)
    return { emoji: "🧠", title: "Deep Thinkers", blurb: "Long, dense, high-signal turns." };
  if (x.totalMessages >= 100)
    return { emoji: "⚡", title: "Rapid Fire", blurb: "Fast back-and-forth, no dead air." };
  return { emoji: "✨", title: "The Collab", blurb: "Minds in sync on one thread." };
}

/** Prettify a room id (`session.ab12…`, `room.xyz`) into a short, human-ish footer label. */
export function prettyRoom(room: string): string {
  const tail = room.includes(".") ? room.slice(room.indexOf(".") + 1) : room;
  const short = tail.length > 18 ? `${tail.slice(0, 16)}…` : tail;
  return short || room;
}

/** Compact number: 1234 → "1.2k", 2_500_000 → "2.5M". Small values stay exact. */
export function fmtCompact(n: number): string {
  if (!Number.isFinite(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 10_000) return `${Math.round(n / 1_000)}k`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return `${Math.round(n)}`;
}

/** Human span: "45s", "12m", "1h 3m", "2d 4h". "—" for an empty/zero span. */
export function fmtDuration(ms: number): string {
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

/** Fraction → whole-percent string: 0.42 → "42%". */
export function fmtPercent(x: number): string {
  return `${Math.round(x * 100)}%`;
}
