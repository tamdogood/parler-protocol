import { contextBridge, ipcRenderer } from "electron";
import { CH, EV } from "../shared/channels";
import type { ParlerApi } from "../shared/types";

function subscribe<T>(channel: string, cb: (payload: T) => void): () => void {
  const listener = (_e: unknown, payload: T): void => cb(payload);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
}

const api: ParlerApi = {
  app: {
    version: () => ipcRenderer.invoke(CH.appVersion),
    platform: process.platform,
  },
  settings: {
    get: () => ipcRenderer.invoke(CH.settingsGet),
    set: (patch) => ipcRenderer.invoke(CH.settingsSet, patch),
  },
  hub: {
    status: () => ipcRenderer.invoke(CH.hubStatus),
    start: () => ipcRenderer.invoke(CH.hubStart),
    stop: () => ipcRenderer.invoke(CH.hubStop),
    restart: () => ipcRenderer.invoke(CH.hubRestart),
    storage: () => ipcRenderer.invoke(CH.hubStorage),
    logs: () => ipcRenderer.invoke(CH.hubLogs),
    joinSecret: () => ipcRenderer.invoke(CH.hubJoinSecret),
    directoryToken: (force) => ipcRenderer.invoke(CH.hubDirectoryToken, force),
    openDataFolder: () => ipcRenderer.invoke(CH.hubOpenDataFolder),
    urlFor: (target) => ipcRenderer.invoke(CH.hubUrlFor, target),
    lanAddress: () => ipcRenderer.invoke(CH.hubLanAddress),
    onStatus: (cb) => subscribe(EV.hubStatus, cb),
    onLog: (cb) => subscribe(EV.hubLog, cb),
  },
  agents: {
    detectHosts: () => ipcRenderer.invoke(CH.agentsDetect),
    connect: (hostId, target) => ipcRenderer.invoke(CH.agentsConnect, hostId, target),
    connectAll: (target) => ipcRenderer.invoke(CH.agentsConnectAll, target),
    disconnect: (hostId) => ipcRenderer.invoke(CH.agentsDisconnect, hostId),
    snippet: (target) => ipcRenderer.invoke(CH.agentsSnippet, target),
  },
  session: {
    open: (input) => ipcRenderer.invoke(CH.sessionOpen, input),
    mintWatch: (room) => ipcRenderer.invoke(CH.sessionMintWatch, room),
    whoami: () => ipcRenderer.invoke(CH.sessionWhoami),
    list: () => ipcRenderer.invoke(CH.sessionList),
    forget: (room) => ipcRenderer.invoke(CH.sessionForget, room),
    requests: (room) => ipcRenderer.invoke(CH.sessionRequests, room),
    approve: (room, agent) => ipcRenderer.invoke(CH.sessionApprove, room, agent),
    deny: (room, agent) => ipcRenderer.invoke(CH.sessionDeny, room, agent),
  },
  clipboard: {
    write: (text) => ipcRenderer.invoke(CH.clipboardWrite, text),
  },
  shell: {
    openExternal: (url) => ipcRenderer.invoke(CH.shellOpenExternal, url),
    revealPath: (path) => ipcRenderer.invoke(CH.shellRevealPath, path),
  },
};

contextBridge.exposeInMainWorld("parler", api);
