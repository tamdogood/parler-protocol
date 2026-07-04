import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";
import type { HubStatus, OpenedSessionRecord } from "@shared/types";

/** Tailwind-aware className combiner (shadcn convention). */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * Whether a persisted session lives on the hub the app is currently pointed at — the only time it
 * can be managed live (watch, approve joiners). Local sessions are loopback; the app targets the
 * local hub when it's running, else the public hub.
 */
export function sessionOnActiveHub(session: OpenedSessionRecord, status: HubStatus | null): boolean {
  const activeLocal = status?.phase === "running";
  const sessionLocal = /127\.0\.0\.1|localhost/.test(session.hub);
  return sessionLocal === activeLocal;
}

/** A short, copy-friendly form of an nkey agent id: `UABC…WXYZ`. */
export function shortId(id: string, head = 6, tail = 4): string {
  if (id.length <= head + tail + 1) return id;
  return `${id.slice(0, head)}…${id.slice(-tail)}`;
}

/** "just now" / "3m ago" / "2h ago" / "5d ago" from epoch ms. */
export function relativeTime(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < 45_000) return "just now";
  const mins = Math.round(diff / 60_000);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return `${days}d ago`;
}

/** Human file size: 1.2 MB, 812 KB, 0 B. */
export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

/** Compact number: 1234 → "1.2k", 25_000 → "25k", 2_500_000 → "2.5M". Small values stay exact. */
export function compactNumber(n: number): string {
  if (!Number.isFinite(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 10_000) return `${Math.round(n / 1_000)}k`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return `${n}`;
}

/** "3h 12m" uptime from a start epoch-ms. */
export function uptime(startedAt: number | null): string {
  if (!startedAt) return "—";
  const s = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${sec}s`;
  return `${sec}s`;
}
