import { useCallback, useEffect, useState } from "react";
import type { HubStatus, HubTarget, Settings } from "@shared/types";
import { parler } from "./ipc";

/** Live hub status, seeded from the main process and kept fresh via the status event stream. */
export function useHubStatus(): HubStatus | null {
  const [status, setStatus] = useState<HubStatus | null>(null);
  useEffect(() => {
    let alive = true;
    parler.hub.status().then((s) => alive && setStatus(s));
    const off = parler.hub.onStatus(setStatus);
    return () => {
      alive = false;
      off();
    };
  }, []);
  return status;
}

/** Settings with an updater that persists through the main process. */
export function useSettings(): {
  settings: Settings | null;
  update: (patch: Partial<Settings>) => Promise<void>;
} {
  const [settings, setSettings] = useState<Settings | null>(null);
  useEffect(() => {
    let alive = true;
    parler.settings.get().then((s) => alive && setSettings(s));
    return () => {
      alive = false;
    };
  }, []);
  const update = useCallback(async (patch: Partial<Settings>) => {
    const next = await parler.settings.set(patch);
    setSettings(next);
  }, []);
  return { settings, update };
}

/**
 * Resolve the dialable base URL for a target hub. For the local hub it re-resolves whenever hub
 * status changes (the running port can differ from the configured one).
 */
export function useHubUrl(target: HubTarget, status: HubStatus | null): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    parler.hub.urlFor(target).then((u) => alive && setUrl(u));
    return () => {
      alive = false;
    };
  }, [target, status?.url]);
  return url;
}
