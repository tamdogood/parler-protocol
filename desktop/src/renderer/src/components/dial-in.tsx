import { useEffect, useState } from "react";
import { Check, Loader2 } from "lucide-react";
import { fetchDirectory } from "@/lib/api";
import { parler } from "@/lib/ipc";

/**
 * The missing "did it actually work?" half of connecting an agent. After a wire, an agent only
 * appears on the hub once its MCP server launches and authenticates (its `Hello` upserts a directory
 * card). We poll the local hub's directory and latch each expected agent to "dialed in" the moment
 * it shows — turning the old dead-end ("restart them") into a visible success.
 *
 * Local hub only: a freshly wired agent registers private-by-default, which the app can read on its
 * own hub (auto-minted directory token) but not on the shared public hub.
 */
export function DialInList({ base, hosts }: { base: string; hosts: { id: string; name: string }[] }) {
  const seen = useDialIn(base, hosts);
  if (hosts.length === 0) return null;
  return (
    <div className="mt-4 border-t border-electric-blue/15 pt-3">
      <p className="mb-2 text-[11px] uppercase tracking-wide text-steel">Restart each agent — they light up here as they connect</p>
      <div className="flex flex-col gap-1.5">
        {hosts.map((h) => {
          const online = seen.has(h.id.toLowerCase());
          return (
            <div key={h.id} className="flex items-center gap-2 text-[12.5px]">
              {online ? (
                <Check className="size-3.5 shrink-0 text-delivered-green" />
              ) : (
                <Loader2 className="size-3.5 shrink-0 animate-spin text-steel" />
              )}
              <span className="text-frost">{h.name}</span>
              <span className={online ? "text-delivered-green" : "text-steel"}>
                {online ? "dialed in" : "waiting — restart it to load Parler Protocol"}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/**
 * Poll the local directory and accumulate the set of agent names seen online. Latching (a seen agent
 * stays seen) so a brief idle→offline blip after connecting doesn't flip a confirmed agent back — this
 * is a one-time "it connected" signal, not a live presence indicator. Names are matched against the
 * `PARLER_NAME` each host is wired with, which `parler connect` defaults to the host id.
 */
function useDialIn(base: string, hosts: { id: string; name: string }[]): Set<string> {
  const [seen, setSeen] = useState<Set<string>>(new Set());
  const key = hosts.map((h) => h.id).join(",");
  useEffect(() => {
    setSeen(new Set());
    let alive = true;
    let timer: ReturnType<typeof setTimeout>;
    const tick = async () => {
      try {
        const token = await parler.hub.directoryToken();
        const list = token
          ? await fetchDirectory(base, { scope: "hub" }, token)
          : await fetchDirectory(base, { scope: "public" });
        if (!alive) return;
        setSeen((prev) => {
          const next = new Set(prev);
          for (const e of list) next.add(e.card.name.toLowerCase());
          return next;
        });
      } catch {
        /* hub not ready / transient — keep polling */
      }
      if (alive) timer = setTimeout(tick, 2500);
    };
    void tick();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [base, key]);
  return seen;
}
