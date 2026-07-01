import { execFile } from "node:child_process";
import { existsSync, readFileSync, writeFileSync, mkdirSync, renameSync } from "node:fs";
import { homedir } from "node:os";
import { join, dirname } from "node:path";
import type { McpHost, ConnectSnippet, HubTarget } from "../shared/types";

/** Context the IPC layer supplies: the env to inject, the bundled `parler` path, and the local URL. */
export interface McpContext {
  /** Env for the target hub, e.g. { PARLER_HUB, PARLER_JOIN_SECRET? }. */
  env: Record<string, string>;
  /** Absolute path to the bundled `parler` binary. */
  binPath: string;
  /** The local hub's ws URL (to classify a host's stored registration as local vs public). */
  localUrl: string | null;
}

const SERVER_NAME = "parler";

// GUI apps on macOS don't inherit the login shell's PATH, so a host CLI like `claude` won't be on
// process.env.PATH. Augment PATH with the usual install locations before shelling out.
const EXTRA_PATHS = [
  join(homedir(), ".local/bin"),
  join(homedir(), ".claude/local"),
  "/opt/homebrew/bin",
  "/usr/local/bin",
  "/usr/bin",
  "/bin",
];

function augmentedEnv(): NodeJS.ProcessEnv {
  const path = [process.env.PATH || "", ...EXTRA_PATHS].filter(Boolean).join(":");
  return { ...process.env, PATH: path };
}

/** Resolve the absolute path to the `claude` CLI, or null if not installed. */
function resolveClaude(): string | null {
  const candidates = [
    join(homedir(), ".local/bin/claude"),
    join(homedir(), ".claude/local/claude"),
    "/opt/homebrew/bin/claude",
    "/usr/local/bin/claude",
    "/usr/bin/claude",
  ];
  for (const c of candidates) {
    if (existsSync(c)) return c;
  }
  return null;
}

function execClaude(claude: string, args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    execFile(claude, args, { env: augmentedEnv(), timeout: 15000 }, (err, stdout, stderr) => {
      const code = err && typeof (err as { code?: number }).code === "number" ? (err as { code: number }).code : err ? 1 : 0;
      resolve({ code, stdout: stdout ?? "", stderr: stderr ?? "" });
    });
  });
}

/** Classify a stored PARLER_HUB value as pointing at this machine's local hub or the public hub. */
function classifyTarget(hub: string | undefined): HubTarget | null {
  if (!hub) return null;
  return /127\.0\.0\.1|localhost|\[::1\]/.test(hub) ? "local" : "public";
}

// ---- config-file hosts (Cursor, Claude Desktop) ----

interface ConfigHostDef {
  id: string;
  name: string;
  configPath: string;
  /** Detect the app is installed even before any config exists. */
  installedHint: string[];
}

function configHosts(): ConfigHostDef[] {
  const home = homedir();
  return [
    {
      id: "cursor",
      name: "Cursor",
      configPath: join(home, ".cursor/mcp.json"),
      installedHint: [join(home, ".cursor"), "/Applications/Cursor.app"],
    },
    {
      id: "claude-desktop",
      name: "Claude Desktop",
      configPath: join(home, "Library/Application Support/Claude/claude_desktop_config.json"),
      installedHint: [
        join(home, "Library/Application Support/Claude"),
        "/Applications/Claude.app",
      ],
    },
  ];
}

function readMcpConfig(path: string): { mcpServers?: Record<string, { env?: Record<string, string> }> } {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return {};
  }
}

function serverEntry(ctx: McpContext) {
  return { command: ctx.binPath, args: ["mcp"], env: ctx.env };
}

function writeConfigServer(path: string, ctx: McpContext): void {
  mkdirSync(dirname(path), { recursive: true });
  const cfg = readMcpConfig(path) as Record<string, unknown> & {
    mcpServers?: Record<string, unknown>;
  };
  cfg.mcpServers = { ...(cfg.mcpServers || {}), [SERVER_NAME]: serverEntry(ctx) };
  // Back up any existing file before overwriting, so a bad merge is recoverable.
  if (existsSync(path)) {
    try {
      renameSync(path, `${path}.parler-backup`);
    } catch {
      /* best-effort */
    }
  }
  writeFileSync(path, JSON.stringify(cfg, null, 2), "utf8");
}

function removeConfigServer(path: string): void {
  if (!existsSync(path)) return;
  const cfg = readMcpConfig(path) as Record<string, unknown> & {
    mcpServers?: Record<string, unknown>;
  };
  if (cfg.mcpServers && SERVER_NAME in cfg.mcpServers) {
    delete cfg.mcpServers[SERVER_NAME];
    writeFileSync(path, JSON.stringify(cfg, null, 2), "utf8");
  }
}

// ---- public API ----

/** Discover which MCP hosts are present and whether a `parler` server is already wired. */
export async function detectHosts(): Promise<McpHost[]> {
  const hosts: McpHost[] = [];

  // Claude Code (CLI-driven, one-click).
  const claude = resolveClaude();
  if (claude) {
    const list = await execClaude(claude, ["mcp", "get", SERVER_NAME]);
    const connected = list.code === 0;
    const hub = connected ? list.stdout.match(/PARLER_HUB[=:]\s*(\S+)/)?.[1] : undefined;
    hosts.push({
      id: "claude-code",
      name: "Claude Code",
      installed: true,
      connected,
      connectedTarget: classifyTarget(hub),
      method: "cli",
    });
  } else {
    hosts.push({
      id: "claude-code",
      name: "Claude Code",
      installed: false,
      connected: false,
      connectedTarget: null,
      method: "cli",
      note: "Claude Code CLI not found. Install it, then reopen this screen.",
    });
  }

  // Config-file hosts.
  for (const def of configHosts()) {
    const installed = def.installedHint.some((p) => existsSync(p));
    const cfg = readMcpConfig(def.configPath);
    const entry = cfg.mcpServers?.[SERVER_NAME];
    hosts.push({
      id: def.id,
      name: def.name,
      installed,
      connected: !!entry,
      connectedTarget: classifyTarget(entry?.env?.PARLER_HUB),
      method: "config",
      configPath: def.configPath,
      note: installed ? undefined : `${def.name} not detected — the snippet still works if you install it.`,
    });
  }

  return hosts;
}

/** Wire `parler` into a host for `ctx.env`. Idempotent (repoints if already present). */
export async function connect(hostId: string, ctx: McpContext): Promise<{ ok: boolean; message: string }> {
  if (hostId === "claude-code") {
    const claude = resolveClaude();
    if (!claude) return { ok: false, message: "Claude Code CLI not found on this machine." };
    // Remove any prior registration so a re-connect cleanly repoints the env.
    await execClaude(claude, ["mcp", "remove", SERVER_NAME, "--scope", "user"]);
    const args = ["mcp", "add", SERVER_NAME, "--scope", "user"];
    for (const [k, v] of Object.entries(ctx.env)) args.push("--env", `${k}=${v}`);
    args.push("--", ctx.binPath, "mcp");
    const r = await execClaude(claude, args);
    if (r.code !== 0) {
      return { ok: false, message: (r.stderr || r.stdout).trim().split("\n")[0] || "claude mcp add failed" };
    }
    return { ok: true, message: "Connected to Claude Code. Restart Claude Code to load the server." };
  }

  const def = configHosts().find((d) => d.id === hostId);
  if (def) {
    try {
      writeConfigServer(def.configPath, ctx);
    } catch (e) {
      return { ok: false, message: `Could not write ${def.name} config: ${(e as Error).message}` };
    }
    return { ok: true, message: `Added to ${def.name}. Restart ${def.name} to load the server.` };
  }
  return { ok: false, message: `Unknown host '${hostId}'.` };
}

/** Remove the `parler` server from a host. */
export async function disconnect(hostId: string): Promise<{ ok: boolean; message: string }> {
  if (hostId === "claude-code") {
    const claude = resolveClaude();
    if (!claude) return { ok: false, message: "Claude Code CLI not found." };
    const r = await execClaude(claude, ["mcp", "remove", SERVER_NAME, "--scope", "user"]);
    if (r.code !== 0) {
      return { ok: false, message: (r.stderr || r.stdout).trim().split("\n")[0] || "claude mcp remove failed" };
    }
    return { ok: true, message: "Disconnected from Claude Code." };
  }
  const def = configHosts().find((d) => d.id === hostId);
  if (def) {
    try {
      removeConfigServer(def.configPath);
    } catch (e) {
      return { ok: false, message: `Could not edit ${def.name} config: ${(e as Error).message}` };
    }
    return { ok: true, message: `Removed from ${def.name}.` };
  }
  return { ok: false, message: `Unknown host '${hostId}'.` };
}

/** Build a copy-paste snippet (shell + generic JSON) for the given target env. */
export function snippet(ctx: McpContext): ConnectSnippet {
  const envPairs = Object.entries(ctx.env);
  const shellEnv = envPairs.map(([k, v]) => `--env ${k}=${v}`).join(" ");
  const shell = `claude mcp add parler --scope user ${shellEnv} -- ${ctx.binPath} mcp`.replace(/\s+/g, " ");
  const json = JSON.stringify(
    { mcpServers: { [SERVER_NAME]: serverEntry(ctx) } },
    null,
    2,
  );
  return { env: ctx.env, shell, json, binPath: ctx.binPath };
}
