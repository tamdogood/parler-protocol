import { ipcMain, shell, clipboard, app, BrowserWindow, ShareMenu } from "electron";
import { networkInterfaces } from "node:os";
import { PUBLIC_HUB, type HubTarget } from "../shared/types";
import { conversationShareText } from "../shared/conversation";
import { CH } from "../shared/channels";
import { HubSupervisor } from "./hub-supervisor";
import { loadSettings, saveSettings, syncLoginItem } from "./settings";
import { parlerBinary, dataDir } from "./paths";
import * as mcp from "./mcp";
import * as cli from "./parler-cli";
import { listSessions, saveSession, forgetSession } from "./session-store";
import { hostsNeedingConnection } from "./agent-reconcile-policy";

const AGENT_SCAN_MS = 60_000;

function httpToWs(url: string): string {
  return url.replace(/^http/, "ws");
}

/** Best-effort private LAN IPv4 for the teammate connect line (skips loopback/link-local/internal). */
function lanIp(): string | null {
  const ifaces = networkInterfaces();
  for (const addrs of Object.values(ifaces)) {
    for (const a of addrs ?? []) {
      if (a.family === "IPv4" && !a.internal && !a.address.startsWith("169.254.")) return a.address;
    }
  }
  return null;
}

/** Build the app-facing IPC surface around a running supervisor. */
export function registerIpc(supervisor: HubSupervisor): void {
  const localHttpUrl = (): string => {
    const s = supervisor.getStatus();
    if (s.url) return s.url;
    return `http://127.0.0.1:${loadSettings().hubPort}`;
  };
  const localWsUrl = (): string => httpToWs(localHttpUrl());

  const urlFor = (target: HubTarget): string => (target === "local" ? localHttpUrl() : PUBLIC_HUB);

  /** Env to inject into an MCP host for a given target hub (used to render the manual snippet). */
  const envFor = (target: HubTarget): Record<string, string> => {
    if (target === "public") return { PARLER_HUB: PUBLIC_HUB };
    const env: Record<string, string> = { PARLER_HUB: localWsUrl() };
    const secret = supervisor.joinSecret();
    if (secret) env.PARLER_JOIN_SECRET = secret;
    return env;
  };

  /** `parler connect` hub-selection flags for a target — the app drives the CLI as the source of truth. */
  const hubArgsFor = (target: HubTarget): string[] => {
    if (target === "public") return ["--hub", PUBLIC_HUB];
    const args = ["--hub", localWsUrl()];
    const secret = supervisor.joinSecret();
    if (secret) args.push("--join-secret", secret);
    return args;
  };

  const mcpContext = (target: HubTarget): mcp.McpContext => ({
    hubArgs: hubArgsFor(target),
    env: envFor(target),
    binPath: parlerBinary(),
  });

  /** Hub context for the app's own identity follows the user's selected connection target. */
  const hubContext = (): cli.HubContext => {
    const s = supervisor.getStatus();
    if (loadSettings().connectTarget === "local" && s.phase === "running") {
      return { url: localWsUrl(), joinSecret: supervisor.joinSecret() };
    }
    return { url: PUBLIC_HUB, joinSecret: null };
  };

  // Keep the opt-in automation alive after onboarding: periodically discover newly installed MCP
  // hosts and wire only those that are missing or pointed at another hub. One run at a time avoids
  // concurrent config writers; already-correct hosts are never touched.
  let reconcilingAgents = false;
  const reconcileAgents = async (): Promise<void> => {
    const settings = loadSettings();
    if (!settings.onboarded || !settings.autoConnectAgents || reconcilingAgents) return;
    const target = settings.connectTarget;
    if (target === "local" && supervisor.getStatus().phase !== "running") return;

    reconcilingAgents = true;
    try {
      const hosts = await mcp.detectHosts();
      for (const host of hostsNeedingConnection(hosts, target)) {
        const result = await mcp.connect(host.id, mcpContext(target));
        if (!result.ok) console.warn(`automatic agent connection failed for ${host.name}: ${result.message}`);
      }
    } catch (e) {
      console.warn("automatic agent connection scan failed", e);
    } finally {
      reconcilingAgents = false;
    }
  };

  supervisor.on("status", (status) => {
    if (status.phase === "running") void reconcileAgents();
  });
  const agentScan = setInterval(() => void reconcileAgents(), AGENT_SCAN_MS);
  agentScan.unref();

  ipcMain.handle(CH.appVersion, () => app.getVersion());

  ipcMain.handle(CH.settingsGet, () => loadSettings());
  ipcMain.handle(CH.settingsSet, (_e, patch) => {
    const next = saveSettings(patch);
    // Keep the OS login item in lockstep whenever the toggle changes.
    if (patch && Object.prototype.hasOwnProperty.call(patch, "startAtLogin")) {
      syncLoginItem(next.startAtLogin);
    }
    if (next.autoConnectAgents) void reconcileAgents();
    return next;
  });

  ipcMain.handle(CH.hubStatus, () => supervisor.getStatus());
  ipcMain.handle(CH.hubStart, () => supervisor.start());
  ipcMain.handle(CH.hubStop, () => supervisor.stop());
  ipcMain.handle(CH.hubRestart, () => supervisor.restart());
  ipcMain.handle(CH.hubStorage, () => supervisor.storage());
  ipcMain.handle(CH.hubLogs, () => supervisor.getLogs());
  ipcMain.handle(CH.hubJoinSecret, () => supervisor.joinSecret());

  // Cache a directory token so the Agents view reads the private hub's full roster with no paste.
  // Cleared whenever the hub leaves `running` (a fresh DB would invalidate it).
  let dirToken: string | null = null;
  supervisor.on("status", (s) => {
    if (s.phase !== "running") dirToken = null;
  });
  ipcMain.handle(CH.hubDirectoryToken, async (_e, force?: boolean) => {
    if (dirToken && !force) return dirToken;
    if (supervisor.getStatus().phase !== "running") return null;
    try {
      dirToken = await cli.mintDirectoryToken({ url: localWsUrl(), joinSecret: supervisor.joinSecret() });
      return dirToken;
    } catch {
      dirToken = null;
      return null;
    }
  });
  ipcMain.handle(CH.hubOpenDataFolder, () => shell.openPath(dataDir()));
  ipcMain.handle(CH.hubUrlFor, (_e, target: HubTarget) => urlFor(target));
  ipcMain.handle(CH.hubLanAddress, () => lanIp());

  ipcMain.handle(CH.agentsDetect, () => mcp.detectHosts());
  ipcMain.handle(CH.agentsConnect, (_e, hostId: string, target: HubTarget) =>
    mcp.connect(hostId, mcpContext(target)),
  );
  ipcMain.handle(CH.agentsConnectAll, (_e, target: HubTarget) => mcp.connectAll(mcpContext(target)));
  ipcMain.handle(CH.agentsDisconnect, (_e, hostId: string) => mcp.disconnect(hostId));
  ipcMain.handle(CH.agentsSnippet, (_e, target: HubTarget) => mcp.snippet(mcpContext(target)));

  ipcMain.handle(CH.sessionOpen, async (_e, input) => {
    const ctx = hubContext();
    const opened = await cli.openSession(input, ctx);
    // Remember it so the Conversations screen can re-copy, watch, or approve joiners later.
    saveSession({
      room: opened.room,
      key: opened.key,
      watch: opened.watch,
      topic: input?.topic?.trim() || null,
      approval: !input?.noApproval,
      hub: ctx.url,
      createdAt: Date.now(),
    });
    return opened;
  });
  ipcMain.handle(CH.sessionMintWatch, (_e, room: string) => cli.mintWatch(room, hubContext()));
  ipcMain.handle(CH.sessionWhoami, () => cli.whoami(hubContext()));
  ipcMain.handle(CH.sessionList, () => listSessions());
  ipcMain.handle(CH.sessionForget, (_e, room: string) => forgetSession(room));
  ipcMain.handle(CH.sessionRequests, (_e, room: string) => cli.sessionRequests(room, hubContext()));
  ipcMain.handle(CH.sessionApprove, async (_e, room: string, agent: string) => {
    try {
      await cli.resolveJoin(room, agent, true, hubContext());
      return { ok: true, message: `Approved ${agent}.` };
    } catch (e) {
      return { ok: false, message: e instanceof Error ? e.message : "Failed to approve." };
    }
  });
  ipcMain.handle(CH.sessionDeny, async (_e, room: string, agent: string) => {
    try {
      await cli.resolveJoin(room, agent, false, hubContext());
      return { ok: true, message: `Denied ${agent}.` };
    } catch (e) {
      return { ok: false, message: e instanceof Error ? e.message : "Failed to deny." };
    }
  });
  ipcMain.handle(CH.sessionShare, (event, room: string, key: string) => {
    if (!room || !key || room.length > 256 || key.length > 1024) {
      return { ok: false, message: "Invalid conversation key." };
    }
    const savedHub = listSessions().find((session) => session.room === room)?.hub;
    const text = conversationShareText(key, savedHub ?? hubContext().url);
    if (process.platform !== "darwin") {
      clipboard.writeText(text);
      return { ok: true, message: "Conversation invitation copied." };
    }
    const window = BrowserWindow.fromWebContents(event.sender) ?? undefined;
    new ShareMenu({ texts: [text] }).popup(window ? { window } : undefined);
    return { ok: true, message: "Conversation share menu opened." };
  });

  ipcMain.handle(CH.clipboardWrite, (_e, text: string) => clipboard.writeText(text));
  ipcMain.handle(CH.shellOpenExternal, (_e, url: string) => shell.openExternal(url));
  ipcMain.handle(CH.shellRevealPath, (_e, path: string) => shell.showItemInFolder(path));
}
