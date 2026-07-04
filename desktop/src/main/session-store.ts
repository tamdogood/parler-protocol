import { readFileSync, writeFileSync } from "node:fs";
import type { OpenedSessionRecord } from "../shared/types";
import { sessionsPath } from "./paths";

/**
 * A tiny JSON store for the sessions the app has opened, kept separate from user *preferences*
 * (settings.json). It lets the Sessions screen survive a restart: re-copy a key, re-open the watch
 * viewer, and — the point — see and resolve pending join requests. Forgetting a record is local
 * only; it never ends the session on the hub.
 */

let cache: OpenedSessionRecord[] | null = null;

function read(): OpenedSessionRecord[] {
  if (cache) return cache;
  try {
    const raw = JSON.parse(readFileSync(sessionsPath(), "utf8")) as unknown;
    cache = Array.isArray(raw) ? (raw as OpenedSessionRecord[]) : [];
  } catch {
    cache = [];
  }
  return cache;
}

function write(list: OpenedSessionRecord[]): void {
  cache = list;
  try {
    writeFileSync(sessionsPath(), JSON.stringify(list, null, 2), "utf8");
  } catch (e) {
    console.error("failed to persist sessions", e);
  }
}

/** All remembered sessions, most recently opened first. */
export function listSessions(): OpenedSessionRecord[] {
  return [...read()].sort((a, b) => b.createdAt - a.createdAt);
}

/** Remember a freshly opened session (replacing any prior record for the same room). */
export function saveSession(rec: OpenedSessionRecord): OpenedSessionRecord[] {
  const next = [rec, ...read().filter((s) => s.room !== rec.room)];
  write(next);
  return listSessions();
}

/** Drop a session from local memory (does not end it on the hub). */
export function forgetSession(room: string): OpenedSessionRecord[] {
  write(read().filter((s) => s.room !== room));
  return listSessions();
}
