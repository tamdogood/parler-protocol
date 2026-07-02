import { ipcMain, shell, clipboard, app } from "electron";
import { PUBLIC_HUB, type HubTarget } from "../shared/types";
import { CH } from "../shared/channels";
import { HubSupervisor } from "./hub-supervisor";
import { loadSettings, saveSettings } from "./settings";
import { parlerBinary, dataDir } from "./paths";
import * as mcp from "./mcp";
import * as cli from "./parler-cli";

function httpToWs(url: string): string {
  return url.replace(/^http/, "ws");
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

  /** Hub context for the app's *own* identity: prefer the local hub when running, else public. */
  const hubContext = (): cli.HubContext => {
    const s = supervisor.getStatus();
    if (s.phase === "running") {
      return { url: localWsUrl(), joinSecret: supervisor.joinSecret() };
    }
    return { url: PUBLIC_HUB, joinSecret: null };
  };

  ipcMain.handle(CH.appVersion, () => app.getVersion());

  ipcMain.handle(CH.settingsGet, () => loadSettings());
  ipcMain.handle(CH.settingsSet, (_e, patch) => saveSettings(patch));

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

  ipcMain.handle(CH.agentsDetect, () => mcp.detectHosts());
  ipcMain.handle(CH.agentsConnect, (_e, hostId: string, target: HubTarget) =>
    mcp.connect(hostId, mcpContext(target)),
  );
  ipcMain.handle(CH.agentsConnectAll, (_e, target: HubTarget) => mcp.connectAll(mcpContext(target)));
  ipcMain.handle(CH.agentsDisconnect, (_e, hostId: string) => mcp.disconnect(hostId));
  ipcMain.handle(CH.agentsSnippet, (_e, target: HubTarget) => mcp.snippet(mcpContext(target)));

  ipcMain.handle(CH.sessionOpen, (_e, input) => cli.openSession(input, hubContext()));
  ipcMain.handle(CH.sessionMintWatch, (_e, room: string) => cli.mintWatch(room, hubContext()));
  ipcMain.handle(CH.sessionWhoami, () => cli.whoami(hubContext()));

  ipcMain.handle(CH.clipboardWrite, (_e, text: string) => clipboard.writeText(text));
  ipcMain.handle(CH.shellOpenExternal, (_e, url: string) => shell.openExternal(url));
  ipcMain.handle(CH.shellRevealPath, (_e, path: string) => shell.showItemInFolder(path));
}
