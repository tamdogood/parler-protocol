"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Globe, KeyRound, Lock, Search, ServerCrash, Users } from "lucide-react";
import type { DirectoryEntry, HubSummary, Scope } from "@/lib/types";
import {
  fetchDirectory,
  fetchHub,
  getDirectoryToken,
  HUB_API,
  HubError,
  IS_LOCAL_HUB,
} from "@/lib/api";
import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { HubHeader } from "@/components/hub-header";
import { AgentCard } from "@/components/agent-card";
import { AgentDetail } from "@/components/agent-detail";
import { TokenDialog } from "@/components/token-dialog";

const STATUS_FILTERS = ["working", "idle", "waiting", "offline"] as const;
const REFRESH_MS = 5000;

export function Directory() {
  const [hub, setHub] = useState<HubSummary | null>(null);
  const [scope, setScope] = useState<Scope>("public");
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [statusFilter, setStatusFilter] = useState<string | null>(null);
  const [tagFilter, setTagFilter] = useState<string | null>(null);

  const [entries, setEntries] = useState<DirectoryEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [needsToken, setNeedsToken] = useState(false);
  const [hasToken, setHasToken] = useState(false);
  const [tokenOpen, setTokenOpen] = useState(false);
  const [selected, setSelected] = useState<DirectoryEntry | null>(null);

  useEffect(() => {
    setHasToken(!!getDirectoryToken());
    fetchHub().then(setHub).catch(() => setHub(null));
  }, []);

  // Debounce the free-text query.
  useEffect(() => {
    const id = setTimeout(() => setDebounced(query), 250);
    return () => clearTimeout(id);
  }, [query]);

  const load = useCallback(async () => {
    try {
      const data = await fetchDirectory({
        scope,
        q: debounced || undefined,
        tag: tagFilter || undefined,
        status: statusFilter || undefined,
      });
      setEntries(data);
      setError(null);
      setNeedsToken(false);
    } catch (e) {
      if (e instanceof HubError && e.status === 401) {
        setNeedsToken(true);
        setEntries([]);
        setError(null);
      } else {
        setError(e instanceof Error ? e.message : "Failed to reach the hub.");
        setEntries([]);
      }
    }
  }, [scope, debounced, tagFilter, statusFilter]);

  // Initial + polling refresh for a live feel.
  useEffect(() => {
    load();
    const id = setInterval(load, REFRESH_MS);
    return () => clearInterval(id);
  }, [load]);

  const allTags = useMemo(() => {
    const set = new Map<string, number>();
    for (const e of entries ?? []) for (const t of e.card.tags ?? []) set.set(t, (set.get(t) ?? 0) + 1);
    return [...set.entries()].sort((a, b) => b[1] - a[1]).slice(0, 10).map(([t]) => t);
  }, [entries]);

  const onScopeChange = (s: Scope) => {
    setScope(s);
    setEntries(null);
  };

  const onTokenSaved = () => {
    setHasToken(!!getDirectoryToken());
    load();
  };

  return (
    <section id="directory" className="mx-auto max-w-[1200px] scroll-mt-20 px-6 py-16">
      <HubHeader hub={hub} />

      {/* Controls */}
      <div className="mt-8 flex flex-col gap-4">
        <div className="flex flex-wrap items-center gap-3">
          {/* Scope segmented control */}
          <div className="inline-flex rounded-[10px] border border-graphite-rail p-0.5">
            <ScopeTab active={scope === "public"} onClick={() => onScopeChange("public")} icon={<Globe className="size-3.5" />}>
              Public
            </ScopeTab>
            <ScopeTab active={scope === "hub"} onClick={() => onScopeChange("hub")} icon={<Users className="size-3.5" />}>
              Hub
            </ScopeTab>
          </div>

          {/* Search */}
          <div className="relative min-w-[220px] flex-1">
            <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-steel" />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search agents by name, tag, or skill…"
              className="pl-9"
            />
          </div>

          {/* Token control (hub scope) */}
          {scope === "hub" && (
            <Button variant={hasToken ? "outline" : "primary"} size="sm" onClick={() => setTokenOpen(true)}>
              <KeyRound className="size-3.5" />
              {hasToken ? "Token saved" : "Add token"}
            </Button>
          )}
        </div>

        {/* Status + tag chips */}
        <div className="flex flex-wrap items-center gap-1.5">
          <FilterChip active={!statusFilter} onClick={() => setStatusFilter(null)}>
            All status
          </FilterChip>
          {STATUS_FILTERS.map((s) => (
            <FilterChip key={s} active={statusFilter === s} onClick={() => setStatusFilter(statusFilter === s ? null : s)}>
              {s}
            </FilterChip>
          ))}
          {allTags.length > 0 && <span className="mx-1 h-4 w-px bg-graphite-rail" />}
          {allTags.map((t) => (
            <FilterChip key={t} active={tagFilter === t} onClick={() => setTagFilter(tagFilter === t ? null : t)}>
              #{t}
            </FilterChip>
          ))}
        </div>
      </div>

      {/* Body */}
      <div className="mt-8">
        {needsToken ? (
          <LockedPanel onUnlock={() => setTokenOpen(true)} />
        ) : error ? (
          <ErrorPanel message={error} />
        ) : entries === null ? (
          <CardGrid>
            {Array.from({ length: 6 }).map((_, i) => (
              <Skeleton key={i} className="h-48 rounded-[16px]" />
            ))}
          </CardGrid>
        ) : entries.length === 0 ? (
          <EmptyPanel scope={scope} />
        ) : (
          <>
            <div className="mb-4 text-[13px] text-steel">
              {entries.length} agent{entries.length === 1 ? "" : "s"}
              {scope === "public" ? " in the public directory" : " in this hub"}
            </div>
            <CardGrid>
              {entries.map((e) => (
                <AgentCard key={e.card.id} entry={e} onSelect={setSelected} />
              ))}
            </CardGrid>
          </>
        )}
      </div>

      <AgentDetail entry={selected} onClose={() => setSelected(null)} />
      <TokenDialog open={tokenOpen} onOpenChange={setTokenOpen} onSaved={onTokenSaved} hasToken={hasToken} />
    </section>
  );
}

function CardGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">{children}</div>;
}

function ScopeTab({
  active,
  onClick,
  icon,
  children,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-[8px] px-3 py-1.5 text-[13px] transition-colors",
        active ? "bg-white/[0.06] text-pure-white" : "text-fog hover:text-frost",
      )}
    >
      {icon}
      {children}
    </button>
  );
}

function FilterChip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "rounded-[10px] border px-2.5 py-1 text-[12px] capitalize transition-colors",
        active
          ? "border-electric-blue/60 text-pure-white"
          : "border-graphite-rail text-fog hover:border-smoke hover:text-frost",
      )}
    >
      {children}
    </button>
  );
}

function LockedPanel({ onUnlock }: { onUnlock: () => void }) {
  return (
    <Panel icon={<Lock className="size-6 text-complained-yellow" />} title="This hub's directory is private">
      <p>Paste a directory token to view every agent in the hub — including private ones.</p>
      <Button variant="primary" size="sm" className="mt-5" onClick={onUnlock}>
        <KeyRound className="size-3.5" />
        Add a directory token
      </Button>
    </Panel>
  );
}

function ErrorPanel({ message }: { message: string }) {
  return (
    <Panel icon={<ServerCrash className="size-6 text-bounced-red" />} title="Can't reach the hub">
      <p>{message}</p>
      <p className="mt-2 font-mono text-[12px] text-steel">{HUB_API}</p>
      <p className="mt-3 text-[13px] text-steel">
        {IS_LOCAL_HUB ? (
          <>
            Start one with{" "}
            <code className="rounded-[4px] border border-graphite-rail px-1 py-0.5 font-mono text-resend-violet">
              ./scripts/seed-demo.sh
            </code>
          </>
        ) : (
          "The hub may be waking up — retry in a moment."
        )}
      </p>
    </Panel>
  );
}

function EmptyPanel({ scope }: { scope: Scope }) {
  return (
    <Panel icon={<Users className="size-6 text-steel" />} title="No agents here yet">
      <p>
        {scope === "public"
          ? "No agent has published a public card on this hub."
          : "No agents match your filters."}
      </p>
    </Panel>
  );
}

function Panel({
  icon,
  title,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col items-center rounded-[16px] border border-graphite-rail px-6 py-16 text-center">
      <span className="flex size-12 items-center justify-center rounded-[14px] border border-graphite-rail surface-lift">
        {icon}
      </span>
      <h3 className="mt-4 text-[18px] font-semibold text-pure-white">{title}</h3>
      <div className="mt-1.5 max-w-sm text-[14px] text-fog">{children}</div>
    </div>
  );
}
