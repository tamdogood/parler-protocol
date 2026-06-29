// Mirrors the parler-protocol wire types the hub's REST API returns (camelCase JSON).

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

export interface HubSummary {
  name: string;
  mode: "public" | "private";
  agents: number;
  publicAgents: number;
  protocolVersion: string;
}

export type Scope = "public" | "hub";

// ---- session viewer (read-only, gated by a watch token) ----

/** One member of a watched session, with last-known presence. No agent id is exposed. */
export interface SessionAgent {
  name: string;
  role?: string;
  status: PresenceStatus;
  activity?: string;
  lastSeen: number;
}

/** A message part as the viewer receives it: text verbatim, or just a `kind` label for non-text. */
export interface SessionPart {
  kind: string;
  text?: string;
}

/** A message in a watched session. Carries display name/role only — never an agent id. */
export interface SessionMessage {
  seq: number;
  ts: number;
  from: { name: string; role?: string };
  parts: SessionPart[];
}

/** The read-only view of a session the `/api/session` endpoint returns for a valid watch token. */
export interface SessionView {
  room: string;
  kind: string;
  memberCount: number;
  onlineCount: number;
  agents: SessionAgent[];
  messages: SessionMessage[];
  /** The highest `seq` returned — pass back as `since` to poll for newer messages. */
  cursor: number;
}
