import { execFile } from "node:child_process";
import { readFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { userInfo } from "node:os";
import type { Identity, OpenedSession, SessionJoinRequest } from "../shared/types";
import { parlerBinary, appParlerHome } from "./paths";

/** Which hub the app's own identity talks to, plus its join secret (private hubs). */
export interface HubContext {
  /** Dialable hub URL (any of http/https/ws/wss/host:port). */
  url: string;
  joinSecret: string | null;
}

interface RunResult {
  code: number;
  stdout: string;
  stderr: string;
}

/** Run the bundled `parler` with the app's dedicated PARLER_HOME + the target hub's env. */
function run(args: string[], ctx: HubContext): Promise<RunResult> {
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    PARLER_HOME: appParlerHome(),
    PARLER_HUB: ctx.url,
  };
  if (ctx.joinSecret) env.PARLER_JOIN_SECRET = ctx.joinSecret;
  return new Promise((resolve) => {
    execFile(parlerBinary(), args, { env, timeout: 20000 }, (err, stdout, stderr) => {
      const code = err && typeof (err as { code?: number }).code === "number" ? (err as { code: number }).code : err ? 1 : 0;
      resolve({ code, stdout: stdout ?? "", stderr: stderr ?? "" });
    });
  });
}

function configHubUrl(): string | null {
  try {
    const raw = readFileSync(join(appParlerHome(), "config.json"), "utf8");
    return (JSON.parse(raw) as { hub_url?: string }).hub_url ?? null;
  } catch {
    return null;
  }
}

/** Ensure the app has an identity pointed at `ctx.url`, (re)initializing it if missing or repointed. */
async function ensureIdentity(ctx: HubContext): Promise<void> {
  const cfgPath = join(appParlerHome(), "config.json");
  const exists = existsSync(cfgPath);
  const current = configHubUrl();
  // parler stores the hub in a normalized form; a substring check is enough to detect a repoint
  // (e.g. the local port changed) without depending on the exact normalization.
  const sameHub = current ? current.includes(ctx.url.replace(/^\w+:\/\//, "").replace(/\/$/, "")) : false;
  if (exists && sameHub) return;

  let who = "user";
  try {
    who = userInfo().username || "user";
  } catch {
    /* ignore */
  }
  const args = ["init", "--hub", ctx.url, "--name", `${who} (Desktop)`, "--role", "operator"];
  if (exists) args.push("--force");
  const r = await run(args, ctx);
  if (r.code !== 0) {
    throw new Error(cleanErr(r.stderr || r.stdout) || "failed to initialize the desktop agent identity");
  }
}

/** Strip the leading `Error:` cargo/anyhow prefix for friendlier surfacing. */
function cleanErr(s: string): string {
  return s.trim().replace(/^error:\s*/i, "").split("\n")[0] ?? "";
}

/** Open a live session on the given hub and mint a watch code alongside. */
export async function openSession(
  input: { context?: string; topic?: string; noApproval?: boolean },
  ctx: HubContext,
): Promise<OpenedSession> {
  await ensureIdentity(ctx);
  const args = ["session", "open"];
  if (input.context) args.push("--context", input.context);
  if (input.topic) args.push("--topic", input.topic);
  if (input.noApproval) args.push("--no-approval");

  const r = await run(args, ctx);
  if (r.code !== 0) throw new Error(cleanErr(r.stderr || r.stdout) || "failed to open session");

  const room = r.stdout.match(/room '([^']+)'/)?.[1];
  const key = r.stdout.match(/KEY:\s*(\S+)/)?.[1];
  if (!room || !key) throw new Error("could not parse the session key from parler output");

  // Best-effort watch code so the user can immediately paste it into the viewer.
  let watch: string | null = null;
  try {
    const w = await mintWatch(room, ctx);
    watch = w.token;
  } catch {
    /* non-fatal: they can mint one later */
  }
  return { key, room, watch };
}

/**
 * Mint a directory token so the app can read this (private) hub's full roster without a manual paste.
 * Long TTL — the app caches it and re-mints on demand.
 */
export async function mintDirectoryToken(ctx: HubContext, ttlSecs = 86400): Promise<string> {
  await ensureIdentity(ctx);
  const r = await run(["token", "--ttl", String(ttlSecs)], ctx);
  if (r.code !== 0) throw new Error(cleanErr(r.stderr || r.stdout) || "failed to mint directory token");
  const lines = r.stdout.split(/\r?\n/);
  const headerIdx = lines.findIndex((l) => l.includes("directory token"));
  const token = lines
    .slice(headerIdx + 1)
    .map((l) => l.trim())
    .find((l) => l.length >= 12 && !/\s/.test(l));
  if (!token) throw new Error("could not parse the directory token from parler output");
  return token;
}

/** List the agents waiting for approval to join a session this identity opened (structured JSON). */
export async function sessionRequests(room: string, ctx: HubContext): Promise<SessionJoinRequest[]> {
  await ensureIdentity(ctx);
  const r = await run(["session", "requests", "--room", room, "--json"], ctx);
  if (r.code !== 0) throw new Error(cleanErr(r.stderr || r.stdout) || "failed to read join requests");
  try {
    const parsed = JSON.parse(r.stdout.trim()) as { requests?: SessionJoinRequest[] };
    return (parsed.requests ?? []).map((q) => ({
      agent: q.agent,
      name: q.name,
      role: q.role ?? null,
      requestedAt: q.requestedAt,
    }));
  } catch {
    throw new Error("could not parse join requests from parler output");
  }
}

/** Admit (`approve=true`) or turn away (`approve=false`) a pending joiner. */
export async function resolveJoin(
  room: string,
  agent: string,
  approve: boolean,
  ctx: HubContext,
): Promise<void> {
  await ensureIdentity(ctx);
  const r = await run(["session", approve ? "approve" : "deny", "--room", room, agent], ctx);
  if (r.code !== 0) {
    throw new Error(cleanErr(r.stderr || r.stdout) || `failed to ${approve ? "approve" : "deny"} the joiner`);
  }
}

/** Mint a read-only watch code for a session this identity owns. */
export async function mintWatch(room: string, ctx: HubContext): Promise<{ token: string; room: string }> {
  const r = await run(["session", "watch", "--room", room], ctx);
  if (r.code !== 0) throw new Error(cleanErr(r.stderr || r.stdout) || "failed to mint watch code");
  // Output: a "✓ read-only watch code…" header, a blank line, then the indented token.
  const lines = r.stdout.split(/\r?\n/);
  const headerIdx = lines.findIndex((l) => l.includes("watch code"));
  const token = lines
    .slice(headerIdx + 1)
    .map((l) => l.trim())
    .find((l) => l.length >= 12 && !/\s/.test(l));
  if (!token) throw new Error("could not parse the watch code from parler output");
  return { token, room };
}

/** Read the app identity's card (id/name/role/hub). Requires an initialized identity. */
export async function whoami(ctx: HubContext): Promise<Identity> {
  await ensureIdentity(ctx);
  const r = await run(["whoami"], ctx);
  if (r.code !== 0) throw new Error(cleanErr(r.stderr || r.stdout) || "no identity yet");
  const id = r.stdout.match(/id:\s*(\S+)/)?.[1] ?? "";
  const nameLine = r.stdout.match(/name:\s*(.+)/)?.[1]?.trim() ?? "";
  const hub = r.stdout.match(/hub:\s*(\S+)/)?.[1] ?? ctx.url;
  const roleMatch = nameLine.match(/^(.*?)\s*\(([^)]+)\)\s*$/);
  return {
    id,
    name: roleMatch ? roleMatch[1] : nameLine,
    role: roleMatch ? roleMatch[2] : null,
    hub,
  };
}
