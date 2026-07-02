import { execFile } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";
import type { McpHost, ConnectSnippet, HubTarget, ActionResult, ConnectAllResult, ConnectResult } from "../shared/types";
import { parlerBinary } from "./paths";

/**
 * Everything here is a thin driver over the bundled `parler connect` — the **single source of truth**
 * for MCP-host wiring (see `crates/parler-cli/src/connect.rs`). The app used to re-implement detection
 * + config writing in TypeScript, which drifted from the CLI (no Codex, no per-agent identity). Now
 * the GUI's one-click Connect and the terminal's `parler connect` are literally the same code path,
 * so every agent the CLI supports the app supports too, wired identically.
 */

/** Context the IPC layer supplies for a wire action: the hub-selection flags + env for the snippet. */
export interface McpContext {
  /** `parler connect` hub flags for the chosen target, e.g. `["--hub", "ws://127.0.0.1:7071", "--join-secret", "…"]`. */
  hubArgs: string[];
  /** Env for the target hub, e.g. { PARLER_HUB, PARLER_JOIN_SECRET? } — used only to render the manual snippet. */
  env: Record<string, string>;
  /** Absolute path to the bundled `parler` binary (for the manual snippet). */
  binPath: string;
}

const SERVER_NAME = "parler";

// GUI apps on macOS don't inherit the login shell's PATH, so a host CLI like `claude` (which
// `parler connect` shells out to) won't be on process.env.PATH. Augment it before spawning.
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

interface RunResult {
  code: number;
  stdout: string;
  stderr: string;
}

/** Run the bundled `parler` with the augmented PATH. */
function runParler(args: string[], timeout = 30000): Promise<RunResult> {
  return new Promise((resolve) => {
    execFile(parlerBinary(), args, { env: augmentedEnv(), timeout }, (err, stdout, stderr) => {
      const code = err && typeof (err as { code?: number }).code === "number" ? (err as { code: number }).code : err ? 1 : 0;
      resolve({ code, stdout: stdout ?? "", stderr: stderr ?? "" });
    });
  });
}

/** Parse the `results` array `parler connect --json` prints (both wire + `--remove` use this shape). */
function parseResults(r: RunResult): ConnectResult[] | null {
  try {
    const parsed = JSON.parse(r.stdout) as { results?: ConnectResult[] };
    return Array.isArray(parsed.results) ? parsed.results : null;
  } catch {
    return null;
  }
}

/** Strip a leading `error:` prefix and take the first line, for friendlier surfacing. */
function firstLine(s: string): string {
  return s.trim().replace(/^error:\s*/i, "").split("\n")[0] ?? "";
}

/** Classify a stored PARLER_HUB value as pointing at this machine's local hub or the shared hub. */
function classifyTarget(hub: string | null | undefined): HubTarget | null {
  if (!hub) return null;
  return /127\.0\.0\.1|localhost|\[::1\]/.test(hub) ? "local" : "public";
}

/** A human sentence for one wire result. */
function wireMessage(host: ConnectResult): string {
  if (host.status === "wired") return `Connected. Restart ${host.name} to load the server.`;
  if (host.status === "error") return host.detail;
  return host.detail;
}

// ---- public API (all thin wrappers over `parler connect`) ----

/** Discover which MCP hosts are present and whether a `parler` server is already wired, via the CLI. */
export async function detectHosts(): Promise<McpHost[]> {
  const r = await runParler(["connect", "--list", "--json"]);
  let hosts: Array<{ id: string; name: string; installed: boolean; connected: boolean; config: string; hub: string | null }>;
  try {
    hosts = JSON.parse(r.stdout).hosts ?? [];
  } catch {
    return [];
  }
  return hosts.map((h) => {
    const isPath = h.config.startsWith("/") || h.config.startsWith("~");
    return {
      id: h.id,
      name: h.name,
      installed: h.installed,
      connected: h.connected,
      connectedTarget: h.connected ? classifyTarget(h.hub) : null,
      method: h.id === "claude-code" ? "cli" : "config",
      configPath: isPath ? h.config : undefined,
      note: h.installed ? undefined : `${h.name} not detected — the snippet still works once you install it.`,
    };
  });
}

/** Wire one host to `ctx`'s hub. Idempotent (repoints if already present). */
export async function connect(hostId: string, ctx: McpContext): Promise<ActionResult> {
  const r = await runParler(["connect", hostId, ...ctx.hubArgs, "--json"]);
  const results = parseResults(r);
  const host = results?.find((h) => h.id === hostId) ?? results?.[0];
  if (!host) return { ok: false, message: firstLine(r.stderr || r.stdout) || "Connect failed." };
  return { ok: host.status === "wired", message: wireMessage(host) };
}

/** Wire **every detected agent** in one shot — the app's headline action, mirroring `parler connect`. */
export async function connectAll(ctx: McpContext): Promise<ConnectAllResult> {
  const r = await runParler(["connect", ...ctx.hubArgs, "--json"]);
  const results = parseResults(r) ?? [];
  const connected = results.filter((h) => h.status === "wired").length;
  return {
    ok: connected > 0,
    connected,
    total: results.length,
    results,
    message: results.length === 0 ? firstLine(r.stderr || r.stdout) || "No agents detected." : undefined,
  };
}

/** Remove the `parler` server from a host (`parler connect --remove`). */
export async function disconnect(hostId: string): Promise<ActionResult> {
  const r = await runParler(["connect", "--remove", hostId, "--json"]);
  const host = parseResults(r)?.find((h) => h.id === hostId);
  if (!host) return { ok: false, message: firstLine(r.stderr || r.stdout) || "Disconnect failed." };
  if (host.status === "removed") return { ok: true, message: `Disconnected ${host.name}.` };
  if (host.status === "not-configured") return { ok: true, message: `${host.name} wasn't connected.` };
  return { ok: false, message: host.detail };
}

/** Build a copy-paste connect snippet (shell + generic JSON) for the given target env. Display-only. */
export function snippet(ctx: McpContext): ConnectSnippet {
  const envPairs = Object.entries(ctx.env);
  const shellEnv = envPairs.map(([k, v]) => `--env ${k}=${v}`).join(" ");
  const shell = `claude mcp add ${SERVER_NAME} --scope user ${shellEnv} -- ${ctx.binPath} mcp`.replace(/\s+/g, " ");
  const json = JSON.stringify(
    { mcpServers: { [SERVER_NAME]: { command: ctx.binPath, args: ["mcp"], env: ctx.env } } },
    null,
    2,
  );
  return { env: ctx.env, shell, json, binPath: ctx.binPath };
}
