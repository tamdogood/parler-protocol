import { useCallback, useEffect, useState } from "react";
import type { HubStatus, HubTarget, Settings } from "@shared/types";
import type { HubSummary } from "./types";
import { fetchHub } from "./api";
import { sessionOnActiveHub } from "./utils";
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
 * Poll a hub's `/api/hub` summary (name, counts, since-boot throughput counters) for the live
 * monitoring surface. Pass `active=false` to pause polling (e.g. the hub is stopped).
 */
export function useHubSummary(base: string | null, active: boolean): HubSummary | null {
  const [summary, setSummary] = useState<HubSummary | null>(null);
  useEffect(() => {
    if (!base || !active) {
      setSummary(null);
      return;
    }
    let alive = true;
    const tick = () =>
      fetchHub(base)
        .then((s) => alive && setSummary(s))
        .catch(() => {
          /* transient — keep last known */
        });
    tick();
    const id = setInterval(tick, 5000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [base, active]);
  return summary;
}

/**
 * Total agents waiting for approval across every session the app has opened on the active hub. Drives
 * the "someone wants to join" badge on the Sessions nav item, so the flagship approval never hides on
 * another screen. Self-contained (fetches its own session list) and paced gently — each poll shells
 * out to the CLI once per eligible session.
 */
export function usePendingJoinCount(status: HubStatus | null, intervalMs = 7000): number {
  const [count, setCount] = useState(0);
  const phase = status?.phase;
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout>;
    const tick = async () => {
      try {
        const sessions = await parler.session.list();
        const eligible = sessions.filter((s) => s.approval && sessionOnActiveHub(s, status));
        const counts = await Promise.all(
          eligible.map(async (s) => {
            try {
              return (await parler.session.requests(s.room)).length;
            } catch {
              return 0;
            }
          }),
        );
        if (alive) setCount(counts.reduce((a, b) => a + b, 0));
      } catch {
        /* ignore — try again next tick */
      }
      if (alive) timer = setTimeout(tick, intervalMs);
    };
    void tick();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
    // status is only consulted for phase-based eligibility; re-poll when that flips.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase, intervalMs]);
  return count;
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
