import { readFileSync, writeFileSync } from "node:fs";
import { userInfo } from "node:os";
import { app } from "electron";
import type { Settings } from "../shared/types";
import { settingsPath } from "./paths";

/** The default local-hub port. 7071 (not 7070) so it never collides with a dev/seed hub. */
export const DEFAULT_HUB_PORT = 7071;

function defaults(): Settings {
  let who = "My";
  try {
    who = userInfo().username || "My";
  } catch {
    /* sandboxed environments may throw */
  }
  return {
    autoStartHub: true,
    hubPublic: false,
    hubReachable: false,
    hubName: `${who}'s Hub`,
    hubPort: DEFAULT_HUB_PORT,
    connectTarget: "local",
    startAtLogin: false,
    onboarded: false,
  };
}

let cache: Settings | null = null;

/** Load settings (merging over defaults so new keys always have a value). */
export function loadSettings(): Settings {
  if (cache) return cache;
  let stored: Partial<Settings> = {};
  try {
    stored = JSON.parse(readFileSync(settingsPath(), "utf8")) as Partial<Settings>;
  } catch {
    /* first run / unreadable — fall back to defaults */
  }
  cache = { ...defaults(), ...stored };
  return cache;
}

/**
 * Reflect the "start at login" preference into the OS login-item registry. Opens hidden (straight to
 * the tray) so a reboot brings the hub up before agents dial in, without a window stealing focus.
 * Best-effort: unsupported on some Linux setups, where Electron makes this a no-op.
 */
export function syncLoginItem(enabled: boolean): void {
  try {
    app.setLoginItemSettings({ openAtLogin: enabled, openAsHidden: enabled });
  } catch (e) {
    console.error("failed to set login item", e);
  }
}

/** Merge a patch, persist, and return the full settings. */
export function saveSettings(patch: Partial<Settings>): Settings {
  const next = { ...loadSettings(), ...patch };
  cache = next;
  try {
    writeFileSync(settingsPath(), JSON.stringify(next, null, 2), "utf8");
  } catch (e) {
    console.error("failed to persist settings", e);
  }
  return next;
}
