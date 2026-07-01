import { useCallback, useEffect, useMemo, useState } from "react";
import { Search, ServerCrash, Users, Plug } from "lucide-react";
import type { DirectoryEntry } from "@/lib/types";
import { fetchDirectory, HubError } from "@/lib/api";
import { parler } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { AgentCard } from "@/components/agent-card";
import { AgentDetail } from "@/components/agent-detail";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";

const STATUS_ORDER = ["working", "waiting", "idle", "offline"];

/**
 * The agents on your local hub. The app silently mints + uses a directory token so the full private
 * roster shows with no paste; if the hub isn't up yet it falls back to the public-scope view. Just a
 * search + light status filter — no scope/sort/layout toggles.
 */
export function Directory({ base, onConnect }: { base: string; onConnect?: () => void }) {
  const [entries, setEntries] = useState<DirectoryEntry[] | null>(null);
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<DirectoryEntry | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      // Prefer the full (hub-scope) roster via an auto-minted token; fall back to public if unavailable.
      const token = await parler.hub.directoryToken();
      const list = token
        ? await fetchDirectory(base, { scope: "hub" }, token)
        : await fetchDirectory(base, { scope: "public" });
      setEntries(list);
    } catch (e) {
      // A stale cached token → refresh once before giving up.
      if (e instanceof HubError && e.status === 401) {
        try {
          const fresh = await parler.hub.directoryToken(true);
          if (fresh) {
            setEntries(await fetchDirectory(base, { scope: "hub" }, fresh));
            return;
          }
        } catch {
          /* fall through */
        }
      }
      setEntries([]);
      setError(e instanceof Error ? e.message : "Could not reach the hub.");
    }
  }, [base]);

  useEffect(() => {
    setEntries(null);
    load();
    const id = setInterval(load, 5000);
    return () => clearInterval(id);
  }, [load]);

  const statusCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const e of entries ?? []) m.set(e.status, (m.get(e.status) ?? 0) + 1);
    return m;
  }, [entries]);

  const filtered = useMemo(() => {
    let list = entries ?? [];
    if (status) list = list.filter((e) => e.status === status);
    if (query.trim()) {
      const q = query.toLowerCase();
      list = list.filter(
        (e) =>
          e.card.name.toLowerCase().includes(q) ||
          e.card.role?.toLowerCase().includes(q) ||
          e.card.description?.toLowerCase().includes(q) ||
          e.card.id.toLowerCase().includes(q),
      );
    }
    return [...list].sort(
      (a, b) => STATUS_ORDER.indexOf(a.status) - STATUS_ORDER.indexOf(b.status) || b.lastSeen - a.lastSeen,
    );
  }, [entries, status, query]);

  const hasAgents = (entries?.length ?? 0) > 0;

  return (
    <div>
      {/* Search + status facets (only once there are agents) */}
      {hasAgents && (
        <div className="mb-6 flex flex-wrap items-center gap-3">
          <div className="relative min-w-[220px] flex-1">
            <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-steel" />
            <Input className="pl-9" placeholder="Search agents…" value={query} onChange={(e) => setQuery(e.target.value)} />
          </div>
          <div className="flex flex-wrap gap-1.5">
            <FacetChip active={!status} onClick={() => setStatus(null)} label="All" count={entries?.length ?? 0} />
            {STATUS_ORDER.filter((s) => statusCounts.get(s)).map((s) => (
              <FacetChip key={s} active={status === s} onClick={() => setStatus(status === s ? null : s)} label={s} count={statusCounts.get(s) ?? 0} />
            ))}
          </div>
        </div>
      )}

      {error ? (
        <ErrorState message={error} onRetry={load} />
      ) : entries === null ? (
        <CardGrid>
          {Array.from({ length: 6 }).map((_, i) => (
            <Skeleton key={i} className="h-[176px] rounded-[16px]" />
          ))}
        </CardGrid>
      ) : filtered.length === 0 ? (
        <EmptyState onConnect={onConnect} filtered={query.length > 0 || !!status} />
      ) : (
        <CardGrid>
          {filtered.map((e) => (
            <AgentCard key={e.card.id} entry={e} onSelect={setSelected} />
          ))}
        </CardGrid>
      )}

      {selected && <AgentDetail entry={selected} onClose={() => setSelected(null)} />}
    </div>
  );
}

function CardGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">{children}</div>;
}

function FacetChip({ label, count, active, onClick }: { label: string; count: number; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "no-drag inline-flex items-center gap-1.5 rounded-[10px] border px-2.5 py-1 text-[12px] capitalize transition-colors",
        active ? "border-electric-blue/40 bg-electric-blue/10 text-frost" : "border-graphite-rail text-steel hover:border-smoke hover:text-frost",
      )}
    >
      {label}
      <span className={cn("font-mono text-[11px]", active ? "text-electric-blue" : "text-steel/70")}>{count}</span>
    </button>
  );
}

function ErrorState({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="flex flex-col items-center gap-3 rounded-[16px] border border-graphite-rail bg-void-black py-16 text-center">
      <ServerCrash className="size-6 text-bounced-red" />
      <p className="text-[14px] text-fog">{message}</p>
      <Button variant="outline" size="sm" onClick={onRetry}>
        Retry
      </Button>
    </div>
  );
}

function EmptyState({ onConnect, filtered }: { onConnect?: () => void; filtered: boolean }) {
  if (filtered) {
    return (
      <div className="flex flex-col items-center gap-2 rounded-[16px] border border-graphite-rail bg-void-black py-16 text-center">
        <Users className="size-6 text-steel" />
        <p className="text-[14px] text-fog">No agents match.</p>
      </div>
    );
  }
  return (
    <div className="flex flex-col items-center gap-3 rounded-[16px] border border-dashed border-graphite-rail bg-void-black py-20 text-center">
      <span className="flex size-12 items-center justify-center rounded-[14px] border border-graphite-rail surface-lift">
        <Users className="size-5 text-electric-blue" />
      </span>
      <p className="text-[15px] font-medium text-frost">No agents yet</p>
      <p className="max-w-xs text-[13px] text-fog">Connect Claude Code or another agent and it&apos;ll appear here.</p>
      {onConnect && (
        <Button variant="primary" size="sm" className="mt-1" onClick={onConnect}>
          <Plug className="size-3.5" /> Connect an agent
        </Button>
      )}
    </div>
  );
}
