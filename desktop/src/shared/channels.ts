// IPC channel names shared by the main process (ipc handlers) and the preload bridge.

export const CH = {
  appVersion: "app:version",
  settingsGet: "settings:get",
  settingsSet: "settings:set",
  hubStatus: "hub:status",
  hubStart: "hub:start",
  hubStop: "hub:stop",
  hubRestart: "hub:restart",
  hubStorage: "hub:storage",
  hubLogs: "hub:logs",
  hubJoinSecret: "hub:joinSecret",
  hubDirectoryToken: "hub:directoryToken",
  hubOpenDataFolder: "hub:openDataFolder",
  hubUrlFor: "hub:urlFor",
  hubLanAddress: "hub:lanAddress",
  agentsDetect: "agents:detect",
  agentsConnect: "agents:connect",
  agentsConnectAll: "agents:connectAll",
  agentsDisconnect: "agents:disconnect",
  agentsSnippet: "agents:snippet",
  sessionOpen: "session:open",
  sessionMintWatch: "session:mintWatch",
  sessionWhoami: "session:whoami",
  sessionList: "session:list",
  sessionForget: "session:forget",
  sessionRequests: "session:requests",
  sessionApprove: "session:approve",
  sessionDeny: "session:deny",
  clipboardWrite: "clipboard:write",
  shellOpenExternal: "shell:openExternal",
  shellRevealPath: "shell:revealPath",
} as const;

/** Push channels (main → renderer). */
export const EV = {
  hubStatus: "ev:hub:status",
  hubLog: "ev:hub:log",
} as const;
