import { join } from "node:path";
import { app } from "electron";

/**
 * Resolve the bundled Rust binaries. In a packaged .app they live under
 * `Contents/Resources/bin` (electron-builder `extraResources`); in dev they live in the project's
 * `resources/bin` (produced by `scripts/build-binaries.sh`).
 */
function binDir(): string {
  return app.isPackaged ? join(process.resourcesPath, "bin") : join(app.getAppPath(), "resources", "bin");
}

/** Absolute path to the bundled `parler-hub` binary. */
export function hubBinary(): string {
  return join(binDir(), "parler-hub");
}

/** Absolute path to the bundled `parler` binary (CLI + `parler mcp` server). */
export function parlerBinary(): string {
  // Named 'parler-cli' in staging/packaging to avoid case-insensitive collisions
  // with the app bundle 'Parler' (.app) on macOS, which causes recursive app launches.
  return join(binDir(), "parler-cli");
}

/** The runtime directory holding shipped resources (tray icon, etc.). */
function resourceDir(): string {
  return app.isPackaged ? process.resourcesPath : join(app.getAppPath(), "resources");
}

/** Menu-bar tray template icon (nativeImage auto-loads the @2x variant beside it). */
export function trayIcon(): string {
  return join(resourceDir(), "trayTemplate.png");
}

/** The app icon PNG (dock icon in dev). */
export function appIcon(): string {
  return join(app.getAppPath(), "build", "icon.png");
}

/** The app's writable data directory (per-user, survives updates). */
export function dataDir(): string {
  return app.getPath("userData");
}

/** SQLite file for the local hub. */
export function hubDbPath(): string {
  return join(dataDir(), "hub.sqlite");
}

/** Directory the hub writes handed-off blob bytes into. */
export function hubBlobDir(): string {
  return join(dataDir(), "hub.sqlite.blobs");
}

/** File the hub reads/generates+persists its join secret from (private hubs). */
export function joinSecretPath(): string {
  return join(dataDir(), "join-secret");
}

/**
 * A dedicated PARLER_HOME for the *app's own* agent identity (used to open/watch sessions), kept
 * separate from `~/.parler` so it never clobbers the identity Claude Code/other hosts bootstrap.
 */
export function appParlerHome(): string {
  return join(dataDir(), "parler-home");
}

/** settings.json path. */
export function settingsPath(): string {
  return join(dataDir(), "settings.json");
}
