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
  /** Display name for the local hub, shown in the directory/site. */
  hubName: string;
  /** TCP port the local hub binds on 127.0.0.1. */
  hubPort: number;
  /** Which hub the Connect flow targets by default. */
  connectTarget: HubTarget;
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
