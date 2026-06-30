"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Activity,
  BadgeCheck,
  ChevronDown,
  Globe,
  KeyRound,
  LayoutGrid,
  List as ListIcon,
  Lock,
  Search,
  ServerCrash,
  SlidersHorizontal,
  Users,
  X,
} from "lucide-react";
import type { DirectoryEntry, HubSummary, Scope } from "@/lib/types";
import {
  fetchDirectory,
  fetchHub,
  getDirectoryToken,
  HUB_API,
  HubError,
  IS_LOCAL_HUB,
} from "@/lib/api";
import { cn, shortId } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { AgentCard } from "@/components/agent-card";
import { AgentDetail } from "@/components/agent-detail";
import { TokenDialog } from "@/components/token-dialog";
import { StatusDot, statusMeta } from "@/components/status-dot";
import { VerifiedBadge } from "@/components/verified-badge";

const STATUS_FILTERS = ["working", "idle", "waiting", "offline"] as const;
const REFRESH_MS = 5000;
const TAG_LIMIT = 16;

type SortKey = "recent" | "name" | "status";
type ViewMode = "grid" | "list";

const SORT_LABELS: Record<SortKey, string> = {
  recent: "Recently active",
  name: "Name A–Z",
  status: "Status",
};

// Order used by the "Status" sort: live work first, offline last.
const STATUS_RANK: Record<string, number> = { working: 0, waiting: 1, idle: 2, offline: 3 };

/**
 * The full-screen Agents console — the Hub page's Agents tab. The directory dominates the viewport
 * (wide shell, up-to-4-col grid) and gains agent-focused features the home Directory doesn't have:
 * live headline metrics, status/tag facets with live counts, a "live activity" strip, sorting, and a
 * grid⇄list view toggle. It fetches the whole scope+query set once and facets status/tags
 * client-side so every count stays coherent as you filter.
 */
export function AgentsConsole() {
  const [hub, setHub] = useState<HubSummary | null>(null);
  const [scope, setScope] = useState<Scope>("public");
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [statusFilter, setStatusFilter] = useState<string | null>(null);
  const [tagFilter, setTagFilter] = useState<string | null>(null);
  const [sort, setSort] = useState<SortKey>("recent");
  const [view, setView] = useState<ViewMode>("grid");

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

  // Fetch the whole scope+query set; status and tags are faceted client-side so the sidebar counts
  // and headline metrics always describe the same roster you're looking at.
  const load = useCallback(async () => {
    try {
      const data = await fetchDirectory({ scope, q: debounced || undefined });
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
  }, [scope, debounced]);

  // Initial + polling refresh for a live feel.
  useEffect(() => {
    load();
    const id = setInterval(load, REFRESH_MS);
    return () => clearInterval(id);
  }, [load]);

  const all = useMemo(() => entries ?? [], [entries]);

  const statusCounts = useMemo(() => {
    const c: Record<string, number> = {};
    for (const e of all) c[e.status] = (c[e.status] ?? 0) + 1;
    return c;
  }, [all]);

  const metrics = useMemo(() => {
    let online = 0;
    let pub = 0;
    let verified = 0;
    for (const e of all) {
      if (e.status !== "offline") online++;
      if (e.visibility === "public") pub++;
      if (e.verified) verified++;
    }
    return { total: all.length, online, pub, verified };
  }, [all]);

  const allTags = useMemo(() => {
    const m = new Map<string, number>();
    for (const e of all) for (const t of e.card.tags ?? []) m.set(t, (m.get(t) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  }, [all]);

  const visible = useMemo(() => {
    let list = all;
    if (statusFilter) list = list.filter((e) => e.status === statusFilter);
    if (tagFilter) list = list.filter((e) => (e.card.tags ?? []).includes(tagFilter));
    const sorted = [...list];
    if (sort === "name") {
      sorted.sort((a, b) => a.card.name.localeCompare(b.card.name));
    } else if (sort === "status") {
      sorted.sort(
        (a, b) =>
          (STATUS_RANK[a.status] ?? 9) - (STATUS_RANK[b.status] ?? 9) || b.lastSeen - a.lastSeen,
      );
    } else {
      sorted.sort((a, b) => b.lastSeen - a.lastSeen);
    }
    return sorted;
  }, [all, statusFilter, tagFilter, sort]);

  const activeNow = useMemo(
    () =>
      all
        .filter((e) => e.status === "working" || e.status === "waiting")
        .sort((a, b) => b.lastSeen - a.lastSeen),
    [all],
  );

  const onScopeChange = (s: Scope) => {
    if (s === scope) return;
    setScope(s);
    setEntries(null);
    setStatusFilter(null);
    setTagFilter(null);
  };

  const onTokenSaved = () => {
    setHasToken(!!getDirectoryToken());
    load();
  };

  const loading = entries === null;

  return (
    <section className="mx-auto w-full max-w-[1600px] px-4 pb-20 pt-8 sm:px-6">
      {/* Header + live metrics */}
      <div className="flex flex-col gap-6 lg:flex-row lg:items-end lg:justify-between">
        <div>
          <h1 className="text-[30px] font-semibold leading-tight tracking-[-0.02em] text-pure-white">
            Agents
          </h1>
          <p className="mt-1.5 text-[14px] text-fog">
            {hub ? (
              <>
                Live on <span className="text-frost">{hub.name}</span> ·{" "}
                <span className="capitalize">{hub.mode}</span> hub · protocol{" "}
                <span className="font-mono text-frost">v{hub.protocolVersion}</span>
              </>
            ) : (
              "Browse every agent on the hub — live presence, signed identities, full-text search."
            )}
          </p>
        </div>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4 lg:flex">
          <Metric icon={<Users className="size-4 text-electric-blue" />} label="Agents" value={metrics.total} loading={loading} />
          <Metric icon={<Activity className="size-4 text-delivered-green" />} label="Online now" value={metrics.online} loading={loading} accent="#3ad389" />
          <Metric icon={<Globe className="size-4 text-opened-blue" />} label="Public" value={metrics.pub} loading={loading} />
          <Metric icon={<BadgeCheck className="size-4 text-resend-violet" />} label="Verified" value={metrics.verified} loading={loading} accent="#9281f7" />
        </div>
      </div>

      {/* App shell: sticky filter rail + dominant main column */}
      <div className="mt-8 grid gap-6 lg:grid-cols-[232px_minmax(0,1fr)]">
        {/* Sidebar */}
        <aside className="lg:sticky lg:top-[116px] lg:max-h-[calc(100vh-132px)] lg:self-start lg:overflow-y-auto lg:pr-1">
          <div className="flex flex-col gap-6">
            <SidebarSection title="Scope">
              <div className="inline-flex w-full rounded-[10px] border border-graphite-rail p-0.5">
                <ScopeTab active={scope === "public"} onClick={() => onScopeChange("public")} icon={<Globe className="size-3.5" />}>
                  Public
                </ScopeTab>
                <ScopeTab active={scope === "hub"} onClick={() => onScopeChange("hub")} icon={<Users className="size-3.5" />}>
                  Hub
                </ScopeTab>
              </div>
              {scope === "hub" && (
                <Button variant={hasToken ? "outline" : "primary"} size="sm" className="mt-2 w-full" onClick={() => setTokenOpen(true)}>
                  <KeyRound className="size-3.5" />
                  {hasToken ? "Token saved" : "Add directory token"}
                </Button>
              )}
            </SidebarSection>

            <SidebarSection title="Status">
              <div className="flex flex-col gap-0.5">
                <StatusFacet label="All agents" count={metrics.total} active={!statusFilter} onClick={() => setStatusFilter(null)} />
                {STATUS_FILTERS.map((s) => (
                  <StatusFacet
                    key={s}
                    status={s}
                    label={statusMeta(s).label}
                    count={statusCounts[s] ?? 0}
                    active={statusFilter === s}
                    onClick={() => setStatusFilter(statusFilter === s ? null : s)}
                  />
                ))}
              </div>
            </SidebarSection>

            {allTags.length > 0 && (
              <SidebarSection
                title="Tags"
                action={
                  tagFilter ? (
                    <button onClick={() => setTagFilter(null)} className="text-[12px] text-electric-blue hover:underline">
                      Clear
                    </button>
                  ) : undefined
                }
              >
                <div className="flex flex-wrap gap-1.5">
                  {allTags.slice(0, TAG_LIMIT).map(([t, n]) => (
                    <TagFacet key={t} label={t} count={n} active={tagFilter === t} onClick={() => setTagFilter(tagFilter === t ? null : t)} />
                  ))}
                </div>
              </SidebarSection>
            )}
          </div>
        </aside>

        {/* Main column — the directory, occupying most of the screen */}
        <div className="min-w-0">
          {activeNow.length > 0 && <ActivityStrip agents={activeNow} onSelect={setSelected} />}

          {/* Toolbar */}
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
            <div className="relative min-w-0 flex-1">
              <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-steel" />
              <Input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search agents by name, tag, or skill…"
                className="h-9 rounded-[10px] pl-9"
              />
            </div>
            <div className="flex items-center gap-2">
              <SortSelect value={sort} onChange={setSort} />
              <ViewToggle value={view} onChange={setView} />
            </div>
          </div>

          {/* Result count + active filters */}
          <div className="mt-4 flex flex-wrap items-center gap-2 text-[13px] text-steel">
            {!loading && !needsToken && !error && (
              <span>
                <span className="text-frost">{visible.length}</span> of {metrics.total} agent
                {metrics.total === 1 ? "" : "s"}
                {scope === "public" ? " · public directory" : " · this hub"}
              </span>
            )}
            {statusFilter && <FilterPill onClear={() => setStatusFilter(null)}>{statusMeta(statusFilter).label}</FilterPill>}
            {tagFilter && <FilterPill onClear={() => setTagFilter(null)}>#{tagFilter}</FilterPill>}
          </div>

          {/* Body */}
          <div className="mt-4">
            {needsToken ? (
              <LockedPanel onUnlock={() => setTokenOpen(true)} />
            ) : error ? (
              <ErrorPanel message={error} />
            ) : loading ? (
              view === "grid" ? (
                <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
                  {Array.from({ length: 8 }).map((_, i) => (
                    <Skeleton key={i} className="h-48 rounded-[16px]" />
                  ))}
                </div>
              ) : (
                <div className="overflow-hidden rounded-[16px] border border-graphite-rail">
                  {Array.from({ length: 8 }).map((_, i) => (
                    <Skeleton key={i} className="h-[57px] rounded-none border-b border-graphite-rail/60 last:border-b-0" />
                  ))}
                </div>
              )
            ) : visible.length === 0 ? (
              <EmptyPanel scope={scope} filtered={!!(statusFilter || tagFilter || debounced)} />
            ) : view === "grid" ? (
              <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
                {visible.map((e) => (
                  <AgentCard key={e.card.id} entry={e} onSelect={setSelected} />
                ))}
              </div>
            ) : (
              <div className="overflow-hidden rounded-[16px] border border-graphite-rail">
                {visible.map((e) => (
                  <AgentRow key={e.card.id} entry={e} onSelect={setSelected} />
                ))}
              </div>
            )}
          </div>
        </div>
      </div>

      <AgentDetail entry={selected} onClose={() => setSelected(null)} />
      <TokenDialog open={tokenOpen} onOpenChange={setTokenOpen} onSaved={onTokenSaved} hasToken={hasToken} />
    </section>
  );
}

function Metric({
  icon,
  label,
  value,
  loading,
  accent,
}: {
  icon: React.ReactNode;
  label: string;
  value: number;
  loading?: boolean;
  accent?: string;
}) {
  return (
    <div className="flex items-center gap-3 rounded-[14px] border border-graphite-rail bg-void-black px-4 py-3 lg:min-w-[132px]">
      <span className="flex size-9 shrink-0 items-center justify-center rounded-[10px] border border-graphite-rail surface-lift">
        {icon}
      </span>
      <div className="min-w-0">
        <div
          className="text-[20px] font-semibold leading-none tabular-nums text-pure-white"
          style={accent && !loading ? { color: accent } : undefined}
        >
          {loading ? "—" : value}
        </div>
        <div className="mt-1 truncate text-[12px] text-steel">{label}</div>
      </div>
    </div>
  );
}

function SidebarSection({
  title,
  action,
  children,
}: {
  title: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div className="mb-2.5 flex items-center justify-between">
        <h3 className="text-[11px] font-medium uppercase tracking-wide text-steel">{title}</h3>
        {action}
      </div>
      {children}
    </div>
  );
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
        "inline-flex flex-1 items-center justify-center gap-1.5 rounded-[8px] px-3 py-1.5 text-[13px] transition-colors",
        active ? "bg-white/[0.06] text-pure-white" : "text-fog hover:text-frost",
      )}
    >
      {icon}
      {children}
    </button>
  );
}

function StatusFacet({
  status,
  label,
  count,
  active,
  onClick,
}: {
  status?: string;
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex items-center justify-between gap-2 rounded-[8px] px-2.5 py-1.5 text-[13px] transition-colors",
        active ? "bg-white/[0.06] text-pure-white" : "text-fog hover:bg-white/[0.03] hover:text-frost",
      )}
    >
      <span className="inline-flex items-center gap-2">
        {status ? <StatusDot status={status} /> : <span className="size-2 rounded-full bg-smoke" />}
        {label}
      </span>
      <span className="font-mono text-[12px] tabular-nums text-steel">{count}</span>
    </button>
  );
}

function TagFacet({
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
        "inline-flex items-center gap-1.5 rounded-[10px] border px-2 py-1 text-[12px] transition-colors",
        active
          ? "border-electric-blue/60 text-pure-white"
          : "border-graphite-rail text-fog hover:border-smoke hover:text-frost",
      )}
    >
      #{label}
      <span className="font-mono text-[11px] tabular-nums text-steel">{count}</span>
    </button>
  );
}

function SortSelect({ value, onChange }: { value: SortKey; onChange: (v: SortKey) => void }) {
  return (
    <div className="relative">
      <SlidersHorizontal className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-steel" />
      <select
        value={value}
        onChange={(e) => onChange(e.target.value as SortKey)}
        aria-label="Sort agents"
        className="h-9 appearance-none rounded-[10px] border border-graphite-rail bg-transparent pl-8 pr-8 text-[13px] text-frost outline-none transition-colors hover:border-smoke focus:border-electric-blue/70"
      >
        {(Object.keys(SORT_LABELS) as SortKey[]).map((k) => (
          <option key={k} value={k} className="bg-void-black text-frost">
            {SORT_LABELS[k]}
          </option>
        ))}
      </select>
      <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 size-3.5 -translate-y-1/2 text-steel" />
    </div>
  );
}

function ViewToggle({ value, onChange }: { value: ViewMode; onChange: (v: ViewMode) => void }) {
  return (
    <div className="inline-flex rounded-[10px] border border-graphite-rail p-0.5">
      {(["grid", "list"] as ViewMode[]).map((m) => (
        <button
          key={m}
          onClick={() => onChange(m)}
          aria-label={`${m} view`}
          aria-pressed={value === m}
          className={cn(
            "flex size-8 items-center justify-center rounded-[8px] transition-colors",
            value === m ? "bg-white/[0.06] text-pure-white" : "text-steel hover:text-frost",
          )}
        >
          {m === "grid" ? <LayoutGrid className="size-4" /> : <ListIcon className="size-4" />}
        </button>
      ))}
    </div>
  );
}

function FilterPill({ children, onClear }: { children: React.ReactNode; onClear: () => void }) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-[10px] border border-electric-blue/40 bg-electric-blue/5 px-2 py-0.5 text-[12px] text-frost">
      {children}
      <button onClick={onClear} aria-label="Clear filter" className="text-steel transition-colors hover:text-frost">
        <X className="size-3" />
      </button>
    </span>
  );
}

/** "What's happening right now" — the agents currently working or waiting, with their activity. */
function ActivityStrip({
  agents,
  onSelect,
}: {
  agents: DirectoryEntry[];
  onSelect: (e: DirectoryEntry) => void;
}) {
  return (
    <div className="mb-4 rounded-[14px] border border-graphite-rail bg-void-black p-3">
      <div className="mb-2 flex items-center gap-2 px-1">
        <Activity className="size-3.5 text-delivered-green" />
        <span className="text-[12px] font-medium uppercase tracking-wide text-steel">Live activity</span>
        <span className="font-mono text-[11px] tabular-nums text-steel">{agents.length} active</span>
      </div>
      <div className="flex gap-2 overflow-x-auto pb-1">
        {agents.slice(0, 8).map((e) => (
          <button
            key={e.card.id}
            onClick={() => onSelect(e)}
            className="flex min-w-[200px] max-w-[260px] shrink-0 items-start gap-2 rounded-[10px] border border-graphite-rail bg-black/30 px-3 py-2 text-left transition-colors hover:border-smoke"
          >
            <StatusDot status={e.status} className="mt-1" />
            <span className="min-w-0">
              <span className="block truncate text-[13px] font-medium text-frost">{e.card.name}</span>
              <span className="block truncate text-[12px] text-fog">
                {e.activity || statusMeta(e.status).label}
              </span>
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

/** Compact one-line row for the list view — denser scanning of many agents than the card grid. */
function AgentRow({ entry, onSelect }: { entry: DirectoryEntry; onSelect: (e: DirectoryEntry) => void }) {
  const { card } = entry;
  const status = statusMeta(entry.status);
  const tags = card.tags ?? [];
  return (
    <button
      onClick={() => onSelect(entry)}
      className="group flex w-full items-center gap-4 border-b border-graphite-rail/60 bg-void-black px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-white/[0.02] focus:outline-none focus-visible:bg-white/[0.03]"
    >
      <StatusDot status={entry.status} />
      <div className="flex min-w-0 flex-[2] flex-col">
        <span className="flex items-center gap-1.5">
          <span className="truncate text-[14px] font-medium text-pure-white">{card.name}</span>
          {entry.visibility === "private" && <Lock className="size-3 shrink-0 text-steel" aria-label="private" />}
        </span>
        {card.role && <span className="truncate text-[12px] text-fog">{card.role}</span>}
      </div>
      <span className="hidden min-w-0 flex-[3] truncate text-[13px] text-fog md:block">{card.description}</span>
      <span className="hidden flex-1 truncate font-mono text-[12px] text-steel lg:block">{shortId(card.id, 8, 6)}</span>
      <div className="hidden flex-1 flex-wrap gap-1 xl:flex">
        {tags.slice(0, 3).map((t) => (
          <Badge key={t}>{t}</Badge>
        ))}
      </div>
      <span className="hidden w-[88px] shrink-0 text-right text-[12px] font-medium sm:block" style={{ color: status.color }}>
        {status.label}
      </span>
      <VerifiedBadge verified={entry.verified} className="shrink-0" />
    </button>
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

function EmptyPanel({ scope, filtered }: { scope: Scope; filtered: boolean }) {
  return (
    <Panel icon={<Users className="size-6 text-steel" />} title={filtered ? "No agents match" : "No agents here yet"}>
      <p>
        {filtered
          ? "Try clearing a filter or search term."
          : scope === "public"
            ? "No agent has published a public card on this hub."
            : "No agents are registered on this hub."}
      </p>
    </Panel>
  );
}
