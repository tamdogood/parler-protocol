// Shared IPC contract between the Electron main process and the renderer. Pure types — no runtime.
// Imported by main via `../shared/types`, by preload/renderer via the `@shared/types` alias.

/** Where an agent should connect: the app's own local hub, or the always-on public hub. */
export type HubTarget = "local" | "public";

/** Persisted user settings (JSON in userData/settings.json). */
export interface Settings {
  /** Start the local hub automatically when the app launches. */
  autoStartHub: boolean;
  /** Local hub is world-readable (public directory) vs. token-gated private. Default private. */
  hubPublic: boolean;
  /**
   * Bind the hub on all interfaces (0.0.0.0) so teammates on your network can dial in, vs. loopback
   * only (127.0.0.1, nothing leaves this Mac). A reachable *private* hub is the "team" rung — still
   * gated by the join secret. Default off.
   */
  hubReachable: boolean;
  /** Display name for the local hub, shown in the directory/site. */
  hubName: string;
  /** TCP port the local hub binds on 127.0.0.1. */
  hubPort: number;
  /** Which hub the Connect flow targets by default. */
  connectTarget: HubTarget;
  /** Launch Parler Protocol at login (kept hidden in the tray) so the hub is up before agents dial in. */
  startAtLogin: boolean;
  /** Whether the first-run onboarding has been completed. */
  onboarded: boolean;
}

/** Live state of the supervised local hub process. */
export interface HubStatus {
  /** Lifecycle phase of the child process. */
  phase: "stopped" | "starting" | "running" | "error";
  /** Dialable base URL, e.g. http://127.0.0.1:7071 (present once running). */
  url: string | null;
  /** OS process id, when running. */
  pid: number | null;
  /** "public" | "private" (mirrors the mode the hub booted in). */
  mode: "public" | "private";
  /** Hub display name. */
  name: string;
  /** epoch ms the process started, for uptime. */
  startedAt: number | null;
  /** Human-readable last error, if phase === "error". */
  error: string | null;
  /** True once GET /health returns ok. */
  healthy: boolean;
}

/** On-disk footprint of the local hub's data. */
export interface HubStorage {
  dbPath: string;
  dbBytes: number;
  blobBytes: number;
  dataDir: string;
}

/** One MCP host the app can wire up (Claude Code, Cursor, …). */
export interface McpHost {
  id: string;
  name: string;
  /** The host tooling is present on this machine. */
  installed: boolean;
  /** A `parler` MCP server is already registered with this host. */
  connected: boolean;
  /** If connected, which hub the registration points at (best-effort). */
  connectedTarget: HubTarget | null;
  /** How connection works: "cli" (we can one-click it) or "config" (copy-paste only). */
  method: "cli" | "config";
  /** For config-only hosts, the path to the file the user edits. */
  configPath?: string;
  /** Why the host can't be auto-wired (e.g. not installed). */
  note?: string;
}

/** Result of a connect/disconnect attempt. */
export interface ActionResult {
  ok: boolean;
  message: string;
}

/** One host's outcome from `parler connect --json` (wire or `--remove`). */
export interface ConnectResult {
  id: string;
  name: string;
  /** "wired" | "removed" | "not-configured" | "error". */
  status: string;
  detail: string;
}

/** Result of wiring every detected agent in one action. */
export interface ConnectAllResult {
  ok: boolean;
  /** How many agents were wired. */
  connected: number;
  /** How many hosts the CLI acted on. */
  total: number;
  results: ConnectResult[];
  /** Set only when nothing happened (e.g. no agents detected), for surfacing. */
  message?: string;
}

/** A copy-paste connect snippet for a given target (for manual/unsupported hosts). */
export interface ConnectSnippet {
  /** The exact env used, e.g. { PARLER_HUB, PARLER_JOIN_SECRET? }. */
  env: Record<string, string>;
  /** One-line `claude mcp add …` command. */
  shell: string;
  /** A generic MCP servers.json fragment. */
  json: string;
  /** Absolute path to the bundled `parler` binary. */
  binPath: string;
}

/** A freshly opened live session. */
export interface OpenedSession {
  key: string;
  room: string;
  /** A read-only watch code minted alongside, to paste into the viewer. */
  watch: string | null;
}

/** A persisted session the app opened — remembered across restarts so it can be managed later. */
export interface OpenedSessionRecord {
  room: string;
  key: string;
  /** Read-only watch code minted at open time (null if minting failed). */
  watch: string | null;
  /** Optional short name given at open time. */
  topic: string | null;
  /** Whether joiners must be approved before they can read the conversation. */
  approval: boolean;
  /** The hub URL this session was opened on — scopes which sessions the active hub can manage. */
  hub: string;
  /** epoch ms the session was opened. */
  createdAt: number;
}

/** One agent waiting for approval to join a session (mirrors the CLI's `session requests --json`). */
export interface SessionJoinRequest {
  agent: string;
  name: string;
  role: string | null;
  requestedAt: number;
}

/** The app's own agent identity (used to open/watch sessions). */
export interface Identity {
  id: string;
  name: string;
  role: string | null;
  hub: string;
}

/** The `window.parler` surface exposed by the preload bridge. */
export interface ParlerApi {
  app: {
    version(): Promise<string>;
    platform: string;
  };
  settings: {
    get(): Promise<Settings>;
    set(patch: Partial<Settings>): Promise<Settings>;
  };
  hub: {
    status(): Promise<HubStatus>;
    start(): Promise<HubStatus>;
    stop(): Promise<HubStatus>;
    restart(): Promise<HubStatus>;
    storage(): Promise<HubStorage>;
    logs(): Promise<string[]>;
    /** Reveal the local hub's join secret (private hubs only). */
    joinSecret(): Promise<string | null>;
    /** A directory token for the local hub, so the app can read its full private roster. */
    directoryToken(force?: boolean): Promise<string | null>;
    openDataFolder(): Promise<void>;
    /** epoch-ms → dialable URL for the currently active target. */
    urlFor(target: HubTarget): Promise<string>;
    /** Best-effort LAN IPv4 of this machine (for the teammate connect line), or null. */
    lanAddress(): Promise<string | null>;
    onStatus(cb: (s: HubStatus) => void): () => void;
    onLog(cb: (line: string) => void): () => void;
  };
  agents: {
    detectHosts(): Promise<McpHost[]>;
    connect(hostId: string, target: HubTarget): Promise<ActionResult>;
    /** Wire every detected agent at once — the CLI's `parler connect` in one click. */
    connectAll(target: HubTarget): Promise<ConnectAllResult>;
    disconnect(hostId: string): Promise<ActionResult>;
    snippet(target: HubTarget): Promise<ConnectSnippet>;
  };
  session: {
    open(input: { context?: string; topic?: string; noApproval?: boolean }): Promise<OpenedSession>;
    mintWatch(room: string): Promise<{ token: string; room: string }>;
    whoami(): Promise<Identity>;
    /** The sessions this app has opened, most recent first (persisted across restarts). */
    list(): Promise<OpenedSessionRecord[]>;
    /** Forget a persisted session locally (does not end it on the hub). */
    forget(room: string): Promise<OpenedSessionRecord[]>;
    /** Agents waiting for approval to join a session opened on the active hub. */
    requests(room: string): Promise<SessionJoinRequest[]>;
    /** Admit a pending joiner — they can then read the conversation and participate. */
    approve(room: string, agent: string): Promise<ActionResult>;
    /** Turn away a pending joiner. */
    deny(room: string, agent: string): Promise<ActionResult>;
  };
  clipboard: {
    write(text: string): Promise<void>;
  };
  shell: {
    openExternal(url: string): Promise<void>;
    /** Reveal a file in Finder (used to open a host's MCP config). */
    revealPath(path: string): Promise<void>;
  };
}

/** The public hub URL, mirrored from web/lib/api.ts. */
export const PUBLIC_HUB = "https://parler-hub.fly.dev";
