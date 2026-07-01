import { Tray, Menu, nativeImage, app, type BrowserWindow } from "electron";
import type { HubStatus } from "../shared/types";
import { HubSupervisor } from "./hub-supervisor";
import { trayIcon } from "./paths";

/**
 * A menu-bar presence: a template icon plus a menu that reflects live hub status and offers
 * start/stop + show/quit. Keeps the app one click away even when the window is closed.
 */
export function createTray(supervisor: HubSupervisor, showWindow: () => void): Tray {
  const img = nativeImage.createFromPath(trayIcon());
  img.setTemplateImage(true);
  const tray = new Tray(img.isEmpty() ? nativeImage.createEmpty() : img);
  tray.setToolTip("Parler");

  const render = (s: HubStatus): void => {
    const running = s.phase === "running";
    const statusLabel =
      s.phase === "running"
        ? s.healthy
          ? `Hub running · ${s.url ?? ""}`
          : "Hub running (starting up)…"
        : s.phase === "starting"
          ? "Hub starting…"
          : s.phase === "error"
            ? `Hub error: ${s.error ?? "unknown"}`
            : "Hub stopped";

    const menu = Menu.buildFromTemplate([
      { label: statusLabel, enabled: false },
      { type: "separator" },
      running || s.phase === "starting"
        ? { label: "Stop hub", click: () => void supervisor.stop() }
        : { label: "Start hub", click: () => void supervisor.start() },
      { label: "Open Parler", click: showWindow },
      { type: "separator" },
      { label: "Quit Parler", click: () => app.quit() },
    ]);
    tray.setContextMenu(menu);
  };

  render(supervisor.getStatus());
  supervisor.on("status", render);
  tray.on("click", showWindow);
  return tray;
}

// Keep a reference so the tray isn't garbage-collected.
let trayRef: Tray | null = null;
export function installTray(supervisor: HubSupervisor, win: () => BrowserWindow | null): void {
  trayRef = createTray(supervisor, () => {
    const w = win();
    if (w) {
      if (w.isMinimized()) w.restore();
      w.show();
      w.focus();
    }
  });
}
export function trayInstance(): Tray | null {
  return trayRef;
}
