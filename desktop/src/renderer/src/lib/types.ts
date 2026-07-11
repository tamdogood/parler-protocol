// Mirrors the parler-protocol wire types the hub's REST API returns (camelCase JSON).
// Ported from web/lib/types.ts.

export type Visibility = "public" | "private";
export type PresenceStatus = "idle" | "working" | "waiting" | "offline" | string;

export interface AgentSkill {
  id: string;
  name: string;
  description?: string;
}

export interface AgentCard {
  id: string;
  name: string;
  kind: "agent" | "endpoint";
  role?: string;
  description?: string;
  tags?: string[];
  skills?: AgentSkill[];
  meta?: Record<string, unknown>;
  protocolVersion?: string;
}

export interface DirectoryEntry {
  card: AgentCard;
  visibility: Visibility;
  status: PresenceStatus;
  activity?: string;
  hub: string;
  verified: boolean;
  sig?: string;
  firstSeen: number;
  lastSeen: number;
}

/** Cumulative-since-boot counters + live gauge the hub surfaces under `/api/hub` for monitoring. */
export interface HubStats {
  liveConnections: number;
  connectionsTotal: number;
  messagesTotal: number;
  estimatedTokensTotal: number;
  pushesTotal: number;
}

export interface HubSummary {
  name: string;
  mode: "public" | "private";
  agents: number;
  publicAgents: number;
  protocolVersion: string;
  /** Optional so an older hub without the counters still renders. */
  stats?: HubStats;
}

export type Scope = "public" | "hub";

// ---- session viewer (read-only, gated by a watch token) ----

export interface SessionAgent {
  name: string;
  role?: string;
  status: PresenceStatus;
  activity?: string;
  lastSeen: number;
}

/** A file exchanged in the session — a code bundle (`com.parler.bundle`) or a handed-off file
 * (`com.parler.file`). Reference metadata only; the bytes come from `GET /api/session/blob/:blob`. */
export interface SessionFile {
  /** Content id (sha256) — the key used to download the bytes. */
  blob: string;
  /** Original basename (present for a file handoff; a code bundle carries none). */
  name?: string;
  /** Byte length. */
  size: number;
  /** IANA media type, when known. */
  mediaType?: string;
  /** One-line human description, when set. */
  summary?: string;
  /** Code bundle only: the artifact kind ("git", "patch", …) and the bundled tip/base commits. */
  vcs?: string;
  tip?: string;
  base?: string;
}

export interface SessionPart {
  kind: string;
  text?: string;
  fields?: Record<string, any>;
  /** Present on a `com.parler.bundle` / `com.parler.file` part: the exchanged file's metadata. */
  file?: SessionFile;
}

export interface SessionMessage {
  seq: number;
  ts: number;
  from: { name: string; role?: string };
  parts: SessionPart[];
}

/** One participant's slice of the session's activity, by display identity (never an agent id). */
export interface SessionAgentStat {
  name: string;
  role?: string;
  messages: number;
  /** Estimated tokens this agent has spent talking in the room (~4 chars/token; an estimate). */
  estimatedTokens: number;
}

/** Whole-room activity metrics — "how much have my agents been talking / spending." */
export interface SessionStats {
  messages: number;
  /** Estimated total tokens spent communicating in this room (an estimate, not a billed count). */
  estimatedTokens: number;
  /** Epoch-ms of the first / last message, or null for an empty room (the activity span). */
  firstMessageAt: number | null;
  lastMessageAt: number | null;
  /** Per-agent breakdown, most estimated tokens first. */
  perAgent: SessionAgentStat[];
}

export interface SessionView {
  room: string;
  kind: string;
  memberCount: number;
  onlineCount: number;
  agents: SessionAgent[];
  messages: SessionMessage[];
  cursor: number;
  /** Whole-room activity metrics. Optional so an older hub without it still renders the viewer. */
  stats?: SessionStats;
}
