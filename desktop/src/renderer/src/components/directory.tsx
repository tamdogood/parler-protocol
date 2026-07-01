import { useCallback, useEffect, useMemo, useState } from "react";
import { Globe, Lock, Search, ServerCrash, Users, KeyRound } from "lucide-react";
import type { DirectoryEntry, HubSummary, Scope } from "@/lib/types";
import { fetchDirectory, fetchHub, HubError } from "@/lib/api";
import { cn } from "@/lib/utils";
import { AgentCard } from "@/components/agent-card";
import { AgentDetail } from "@/components/agent-detail";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";

const STATUS_ORDER = ["working", "waiting", "idle", "offline"];

/**
 * A directory browser for a single hub. Fetches the scope+query set once, then facets status/tags
 * client-side so counts stay coherent — the model the website's agents-console uses.
 */
export function Directory({ base, canViewHub = true }: { base: string; canViewHub?: boolean }) {
  const [scope, setScope] = useState<Scope>("public");
  const [hub, setHub] = useState<HubSummary | null>(null);
  const [entries, setEntries] = useState<DirectoryEntry[] | null>(null);
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [tag, setTag] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [needsToken, setNeedsToken] = useState(false);
  const [token, setToken] = useState("");
  const [selected, setSelected] = useState<DirectoryEntry | null>(null);

  useEffect(() => {
    let alive = true;
    fetchHub(base)
      .then((h) => alive && setHub(h))
      .catch(() => alive && setHub(null));
    return () => {
      alive = false;
    };
  }, [base]);

  const load = useCallback(async () => {
    setError(null);
    setNeedsToken(false);
    try {
      const list = await fetchDirectory(base, { scope }, scope === "hub" ? token || undefined : undefined);
      setEntries(list);
    } catch (e) {
      setEntries([]);
      if (e instanceof HubError && e.status === 401) {
        setNeedsToken(true);
      } else {
        setError(e instanceof Error ? e.message : "Could not reach this hub.");
      }
    }
  }, [base, scope, token]);

  useEffect(() => {
    setEntries(null);
    load();
  }, [load]);

  const statusCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const e of entries ?? []) m.set(e.status, (m.get(e.status) ?? 0) + 1);
    return m;
  }, [entries]);

  const tagCounts = useMemo(() => {
    const m = new Map<string, number>();
    for (const e of entries ?? []) for (const t of e.card.tags ?? []) m.set(t, (m.get(t) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1]).slice(0, 14);
  }, [entries]);

  const filtered = useMemo(() => {
    let list = entries ?? [];
    if (status) list = list.filter((e) => e.status === status);
    if (tag) list = list.filter((e) => (e.card.tags ?? []).includes(tag));
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
  }, [entries, status, tag, query]);

  const online = (entries ?? []).filter((e) => e.status !== "offline").length;
  const verified = (entries ?? []).filter((e) => e.verified).length;

  return (
    <div>
      {/* Hub summary + scope toggle */}
      <div className="flex flex-wrap items-center justify-between gap-4">
        <div className="flex items-center gap-4">
          <span className="flex size-11 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
            {hub?.mode === "public" ? (
              <Globe className="size-5 text-opened-blue" strokeWidth={1.75} />
            ) : (
              <Lock className="size-5 text-complained-yellow" strokeWidth={1.75} />
            )}
          </span>
          <div>
            <div className="flex items-center gap-2.5">
              <h2 className="text-[20px] font-semibold tracking-tight text-pure-white">
                {hub?.name ?? "Directory"}
              </h2>
              {hub && (
                <span className="rounded-[6px] border border-graphite-rail px-1.5 py-0.5 text-[11px] uppercase tracking-wide text-fog">
                  {hub.mode} hub
                </span>
              )}
            </div>
            <p className="mt-0.5 text-[13px] text-steel">
              {entries ? (
                <>
                  <span className="text-fog">{entries.length}</span> agents ·{" "}
                  <span className="text-delivered-green">{online}</span> online ·{" "}
                  <span className="text-resend-violet">{verified}</span> verified
                </>
              ) : (
                "loading…"
              )}
            </p>
          </div>
        </div>

        {canViewHub && (
          <div className="no-drag flex rounded-[10px] border border-graphite-rail p-0.5">
            {(["public", "hub"] as Scope[]).map((s) => (
              <button
                key={s}
                onClick={() => setScope(s)}
                className={cn(
                  "rounded-[8px] px-3 py-1.5 text-[13px] font-medium transition-colors",
                  scope === s ? "bg-electric-blue/15 text-pure-white" : "text-steel hover:text-frost",
                )}
              >
                {s === "public" ? "Public" : "This hub"}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Search + status facets */}
      <div className="mt-6 flex flex-wrap items-center gap-3">
        <div className="relative min-w-[220px] flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-steel" />
          <Input
            className="pl-9"
            placeholder="Search agents, roles, ids…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <div className="flex flex-wrap gap-1.5">
          <FacetChip active={!status} onClick={() => setStatus(null)} label="All" count={entries?.length ?? 0} />
          {STATUS_ORDER.filter((s) => statusCounts.get(s)).map((s) => (
            <FacetChip key={s} active={status === s} onClick={() => setStatus(status === s ? null : s)} label={s} count={statusCounts.get(s) ?? 0} />
          ))}
        </div>
      </div>

      {tagCounts.length > 0 && (
        <div className="mt-3 flex flex-wrap gap-1.5">
          {tagCounts.map(([t, c]) => (
            <FacetChip key={t} active={tag === t} onClick={() => setTag(tag === t ? null : t)} label={`#${t}`} count={c} />
          ))}
        </div>
      )}

      {/* Body */}
      <div className="mt-6">
        {needsToken ? (
          <TokenGate token={token} setToken={setToken} onSubmit={load} />
        ) : error ? (
          <ErrorState message={error} onRetry={load} />
        ) : entries === null ? (
          <CardGrid>
            {Array.from({ length: 6 }).map((_, i) => (
              <Skeleton key={i} className="h-[188px] rounded-[16px]" />
            ))}
          </CardGrid>
        ) : filtered.length === 0 ? (
          <EmptyState scope={scope} />
        ) : (
          <CardGrid>
            {filtered.map((e) => (
              <AgentCard key={e.card.id} entry={e} onSelect={setSelected} />
            ))}
          </CardGrid>
        )}
      </div>

      {selected && <AgentDetail entry={selected} onClose={() => setSelected(null)} />}
    </div>
  );
}

function CardGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">{children}</div>;
}

function FacetChip({
  label,
  count,
  active,
  onClick,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "no-drag inline-flex items-center gap-1.5 rounded-[10px] border px-2.5 py-1 text-[12px] capitalize transition-colors",
        active
          ? "border-electric-blue/40 bg-electric-blue/10 text-frost"
          : "border-graphite-rail text-steel hover:border-smoke hover:text-frost",
      )}
    >
      {label}
      <span className={cn("font-mono text-[11px]", active ? "text-electric-blue" : "text-steel/70")}>{count}</span>
    </button>
  );
}

function TokenGate({
  token,
  setToken,
  onSubmit,
}: {
  token: string;
  setToken: (v: string) => void;
  onSubmit: () => void;
}) {
  return (
    <div className="rounded-[16px] border border-graphite-rail bg-void-black p-6">
      <div className="flex items-center gap-2 text-[14px] font-medium text-frost">
        <Lock className="size-4 text-complained-yellow" />
        This hub&apos;s full directory is private
      </div>
      <p className="mt-1.5 text-[13px] leading-relaxed text-fog">
        Paste a directory token to view every agent (including private ones), or switch this hub to public in
        Settings. Public agents are always visible under the <b className="text-frost">Public</b> scope.
      </p>
      <div className="mt-4 flex flex-col gap-3 sm:flex-row">
        <div className="relative flex-1">
          <KeyRound className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-steel" />
          <Input
            className="pl-9 font-mono"
            placeholder="Directory token…"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && onSubmit()}
          />
        </div>
        <Button variant="primary" onClick={onSubmit}>
          View hub directory
        </Button>
      </div>
    </div>
  );
}

function ErrorState({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="flex flex-col items-center gap-3 rounded-[16px] border border-graphite-rail bg-void-black py-16 text-center">
      <ServerCrash className="size-6 text-bounced-red" />
      <p className="text-[14px] text-fog">{message}</p>
      <p className="max-w-sm text-[12px] text-steel">
        If this is your local hub, make sure it&apos;s running (Local Hub → Start).
      </p>
      <Button variant="outline" size="sm" onClick={onRetry}>
        Retry
      </Button>
    </div>
  );
}

function EmptyState({ scope }: { scope: Scope }) {
  return (
    <div className="flex flex-col items-center gap-2 rounded-[16px] border border-graphite-rail bg-void-black py-16 text-center">
      <Users className="size-6 text-steel" />
      <p className="text-[14px] text-fog">No agents{scope === "public" ? " published publicly" : " on this hub"} yet.</p>
      <p className="text-[12px] text-steel">Connect an agent from the Connect tab and it&apos;ll show up here.</p>
    </div>
  );
}
