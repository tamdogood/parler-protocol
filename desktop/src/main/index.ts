import { app, BrowserWindow, nativeImage, session, shell } from "electron";
import { join } from "node:path";
import { existsSync } from "node:fs";
import { HubSupervisor } from "./hub-supervisor";
import { registerIpc } from "./ipc";
import { installTray } from "./tray";
import { loadSettings, syncLoginItem } from "./settings";
import { appIcon } from "./paths";
import { EV } from "../shared/channels";
import type { HubStatus } from "../shared/types";

app.setName("Parler Protocol");

// Only one instance may own the local hub + its SQLite store. A second instance would spawn a
// competing hub over the same database — two writers fighting over one file is a fast path to a
// crash-restart storm. Bounce extra launches and focus the window that's already running.
const hasSingleInstanceLock = app.requestSingleInstanceLock();
if (!hasSingleInstanceLock) {
  app.quit();
  process.exit(0);
}

const supervisor = new HubSupervisor();
let mainWindow: BrowserWindow | null = null;
let isQuitting = false;

function focusMainWindow(): void {
  if (!mainWindow) return;
  if (mainWindow.isMinimized()) mainWindow.restore();
  mainWindow.show();
  mainWindow.focus();
}

app.on("second-instance", focusMainWindow);

function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 1180,
    height: 820,
    minWidth: 940,
    minHeight: 640,
    show: false,
    backgroundColor: "#000000",
    titleBarStyle: "hiddenInset",
    trafficLightPosition: { x: 16, y: 18 },
    title: "Parler Protocol",
    webPreferences: {
      preload: join(__dirname, "../preload/index.js"),
      sandbox: false,
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

  mainWindow.once("ready-to-show", () => mainWindow?.show());

  // Headless smoke instrumentation (CI / `PARLER_SMOKE=1`): surface renderer failures to stdout and
  // self-quit, so a windowless boot can prove the app loads clean. No-op in normal runs.
  if (process.env.PARLER_SMOKE) {
    mainWindow.webContents.on("did-finish-load", () => console.log("[smoke] renderer loaded"));
    mainWindow.webContents.on("console-message", (_e, level, message) => {
      if (level >= 2) console.error(`[smoke] renderer console: ${message}`);
    });
    mainWindow.webContents.on("did-fail-load", (_e, code, desc) =>
      console.error(`[smoke] renderer failed to load: ${code} ${desc}`),
    );
    mainWindow.webContents.on("render-process-gone", (_e, details) =>
      console.error(`[smoke] render process gone: ${details.reason}`),
    );
    if (!process.env.PARLER_SMOKE_HUB) {
      const ms = Number(process.env.PARLER_SMOKE_EXIT_MS || 4000);
      setTimeout(() => {
        console.log("[smoke] exiting");
        isQuitting = true;
        app.quit();
      }, ms);
    }
  }

  // Open external links (docs, github) in the user's browser, never in-app.
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    void shell.openExternal(url);
    return { action: "deny" };
  });

  // On macOS, closing the window hides the app to the tray instead of quitting.
  mainWindow.on("close", (e) => {
    if (!isQuitting && process.platform === "darwin") {
      e.preventDefault();
      mainWindow?.hide();
    }
  });

  const devUrl = process.env.ELECTRON_RENDERER_URL;
  if (devUrl) {
    void mainWindow.loadURL(devUrl);
  } else {
    void mainWindow.loadFile(join(__dirname, "../renderer/index.html"));
  }
}

/** Forward supervisor status/log events to the renderer, guarding against a torn-down window. */
function sendToRenderer(channel: string, payload: unknown): void {
  if (mainWindow && !mainWindow.isDestroyed() && !mainWindow.webContents.isDestroyed()) {
    mainWindow.webContents.send(channel, payload);
  }
}
function wireEvents(): void {
  supervisor.on("status", (s: HubStatus) => sendToRenderer(EV.hubStatus, s));
  supervisor.on("log", (line: string) => sendToRenderer(EV.hubLog, line));
}

app.whenReady().then(() => {
  // A second instance lost the lock above and is on its way out — don't spin up a window or hub.
  if (!hasSingleInstanceLock) return;

  // Dock icon in dev (packaged builds use the bundled .icns).
  if (!app.isPackaged && process.platform === "darwin" && existsSync(appIcon())) {
    try {
      app.dock?.setIcon(nativeImage.createFromPath(appIcon()));
    } catch {
      /* non-fatal */
    }
  }

  // In packaged builds the renderer is fully local, so a strict CSP costs nothing and blocks any
  // injected remote script. (Skipped in dev — Vite's HMR needs inline/websocket flexibility.)
  if (app.isPackaged) {
    session.defaultSession.webRequest.onHeadersReceived((details, cb) => {
      cb({
        responseHeaders: {
          ...details.responseHeaders,
          "Content-Security-Policy": [
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; " +
              "img-src 'self' data:; font-src 'self' data:; connect-src *",
          ],
        },
      });
    });
  }

  registerIpc(supervisor);
  wireEvents();
  createWindow();
  installTray(supervisor, () => mainWindow);

  const settings = loadSettings();
  // Reconcile the OS login item with the persisted preference on every boot (covers a manual
  // System Settings change, or a first run where the default must be written through).
  syncLoginItem(settings.startAtLogin);
  if (settings.autoStartHub && settings.onboarded) {
    void supervisor.start();
  }

  // Full-path smoke (`PARLER_SMOKE_HUB=1`): drive the real supervisor — spawn the bundled hub from
  // resources/, poll to healthy, probe its REST API, then stop + quit. Proves the app can host a hub.
  if (process.env.PARLER_SMOKE_HUB) {
    void (async () => {
      const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
      console.log("[smoke] starting hub via supervisor…");
      await supervisor.start();
      for (let i = 0; i < 40 && supervisor.getStatus().phase !== "running"; i++) await sleep(300);
      const s = supervisor.getStatus();
      console.log(`[smoke] hub phase=${s.phase} url=${s.url} healthy=${s.healthy}`);
      try {
        const res = await fetch(`${s.url}/api/hub`);
        console.log(`[smoke] /api/hub ${res.status} ${await res.text()}`);
      } catch (e) {
        console.error(`[smoke] probe failed: ${(e as Error).message}`);
      }
      await supervisor.stop();
      console.log("[smoke] hub stopped — quitting");
      isQuitting = true;
      app.quit();
    })();
  }

  app.on("activate", () => {
    if (mainWindow) {
      mainWindow.show();
    } else {
      createWindow();
    }
  });
});

// Keep running in the tray after the window closes (macOS convention with a menu-bar item).
app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});

app.on("before-quit", () => {
  isQuitting = true;
  supervisor.shutdownSync();
});
