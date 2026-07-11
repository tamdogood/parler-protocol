import type { DirectoryEntry, HubSummary, Scope, SessionView } from "./types";

/**
 * REST client for a parler-hub. Unlike the website (fixed env base URL), the desktop app talks to
 * whichever hub is active — the local one it runs, or the public hub — so every call takes an
 * explicit `base` URL (resolved from the main process via `window.parler.hub.urlFor(target)`).
 */

export class HubError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

async function getJson<T>(base: string, path: string, token?: string): Promise<T> {
  const res = await fetch(`${base}${path}`, {
    headers: token ? { Authorization: `Bearer ${token}` } : undefined,
    cache: "no-store",
  });
  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) msg = body.error;
    } catch {
      /* non-JSON error body */
    }
    throw new HubError(msg, res.status);
  }
  return (await res.json()) as T;
}

export function fetchHub(base: string): Promise<HubSummary> {
  return getJson<HubSummary>(base, "/api/hub");
}

export interface DiscoverParams {
  scope: Scope;
  q?: string;
  tag?: string;
  skill?: string;
  status?: string;
}

export function fetchDirectory(
  base: string,
  params: DiscoverParams,
  token?: string,
): Promise<DirectoryEntry[]> {
  const qs = new URLSearchParams();
  qs.set("scope", params.scope);
  if (params.q) qs.set("q", params.q);
  if (params.tag) qs.set("tag", params.tag);
  if (params.skill) qs.set("skill", params.skill);
  if (params.status) qs.set("status", params.status);
  return getJson<DirectoryEntry[]>(base, `/api/directory?${qs.toString()}`, params.scope === "hub" ? token : undefined);
}

/**
 * Download one file the session exchanged (a code bundle or a handed-off file), gated by the same
 * watch token as {@link fetchSession} and scoped to that room's blobs. Returns the raw bytes as a Blob
 * so the caller can save or open them. `name` only suggests the download filename (sanitized by the
 * hub); the token rides as a Bearer header, out of the URL.
 */
export async function fetchSessionBlob(
  base: string,
  token: string,
  blob: string,
  name?: string,
): Promise<Blob> {
  const qs = name ? `?name=${encodeURIComponent(name)}` : "";
  const res = await fetch(`${base}/api/session/blob/${encodeURIComponent(blob)}${qs}`, {
    headers: { Authorization: `Bearer ${token.trim()}` },
    cache: "no-store",
  });
  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) msg = body.error;
    } catch {
      /* non-JSON error body */
    }
    throw new HubError(msg, res.status);
  }
  return res.blob();
}

/** Read a session the caller holds a watch token for (Bearer, kept out of the URL). */
export function fetchSession(base: string, token: string, since?: number): Promise<SessionView> {
  const qs = since ? `?since=${since}` : "";
  return fetch(`${base}/api/session${qs}`, {
    headers: { Authorization: `Bearer ${token.trim()}` },
    cache: "no-store",
  }).then(async (res) => {
    if (!res.ok) {
      let msg = `${res.status} ${res.statusText}`;
      try {
        const body = (await res.json()) as { error?: string };
        if (body.error) msg = body.error;
      } catch {
        /* non-JSON error body */
      }
      throw new HubError(msg, res.status);
    }
    return (await res.json()) as SessionView;
  });
}
