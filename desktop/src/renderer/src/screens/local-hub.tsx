import { useCallback, useEffect, useRef, useState } from "react";
import {
  Server,
  Play,
  Square,
  RotateCw,
  FolderOpen,
  Eye,
  EyeOff,
  Database,
  Package,
  Globe,
  Lock,
  Loader2,
} from "lucide-react";
import type { HubStatus, HubStorage, Settings } from "@shared/types";
import { parler } from "@/lib/ipc";
import { bytes, cn, uptime } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { CodeBlock, CopyButton } from "@/components/copyable";

export function LocalHubScreen({
  status,
  settings,
  onUpdateSettings,
}: {
  status: HubStatus | null;
  settings: Settings | null;
  onUpdateSettings: (patch: Partial<Settings>) => Promise<void>;
}) {
  const [storage, setStorage] = useState<HubStorage | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [secret, setSecret] = useState<string | null>(null);
  const [showSecret, setShowSecret] = useState(false);
  const [snippet, setSnippet] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const logRef = useRef<HTMLDivElement>(null);

  const running = status?.phase === "running";
  const starting = status?.phase === "starting";

  // Storage footprint, refreshed while the hub is up.
  useEffect(() => {
    let alive = true;
    const tick = () => parler.hub.storage().then((s) => alive && setStorage(s));
    tick();
    const id = setInterval(tick, 5000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [status?.phase]);

  // Logs: seed + stream.
  useEffect(() => {
    parler.hub.logs().then(setLogs);
    const off = parler.hub.onLog((line) => setLogs((prev) => [...prev.slice(-400), line]));
    return off;
  }, []);
  useEffect(() => {
    logRef.current?.scrollTo({ top: logRef.current.scrollHeight });
  }, [logs]);

  // Join secret + connect snippet (only meaningful once the hub exists).
  useEffect(() => {
    parler.hub.joinSecret().then(setSecret);
    parler.agents.snippet("local").then((s) => setSnippet(s.shell));
  }, [status?.phase, status?.mode]);

  const run = useCallback(async (fn: () => Promise<HubStatus>) => {
    setBusy(true);
    try {
      await fn();
    } finally {
      setBusy(false);
    }
  }, []);

  const toggleMode = async (makePublic: boolean) => {
    await onUpdateSettings({ hubPublic: makePublic });
    if (running || starting) await run(() => parler.hub.restart());
  };

  return (
    <div className="mx-auto max-w-[900px] px-8 py-8">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <span className="flex size-11 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
            <Server className="size-5 text-electric-blue" />
          </span>
          <div>
            <h1 className="text-[22px] font-semibold tracking-tight text-pure-white">{status?.name || "Local Hub"}</h1>
            <p className="text-[13px] text-fog">
              A full Parler hub — WebSocket bus, SQLite directory + memory, blob storage — running on this Mac.
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {running || starting ? (
            <>
              <Button variant="outline" size="sm" onClick={() => run(() => parler.hub.restart())} disabled={busy}>
                <RotateCw className="size-3.5" /> Restart
              </Button>
              <Button variant="danger" size="sm" onClick={() => run(() => parler.hub.stop())} disabled={busy}>
                <Square className="size-3.5" /> Stop
              </Button>
            </>
          ) : (
            <Button variant="primary" onClick={() => run(() => parler.hub.start())} disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : <Play className="size-4" />} Start hub
            </Button>
          )}
        </div>
      </div>

      {status?.phase === "error" && (
        <div className="mt-4 rounded-[12px] border border-bounced-red/30 bg-bounced-red/5 px-4 py-3 text-[13px] text-bounced-red">
          {status.error ?? "The hub failed to start. Check the logs below."}
        </div>
      )}

      {/* Stat grid */}
      <div className="mt-6 grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Stat label="Status" value={running ? "Running" : starting ? "Starting" : status?.phase === "error" ? "Error" : "Stopped"} accent={running ? "#3ad389" : status?.phase === "error" ? "#ff9592" : undefined} />
        <Stat label="Uptime" value={running ? uptime(status?.startedAt ?? null) : "—"} />
        <Stat label="Database" value={storage ? bytes(storage.dbBytes) : "—"} icon={<Database className="size-3.5" />} />
        <Stat label="Blobs" value={storage ? bytes(storage.blobBytes) : "—"} icon={<Package className="size-3.5" />} />
      </div>

      {/* URL + mode */}
      <div className="mt-4 grid grid-cols-1 gap-3 md:grid-cols-2">
        <div className="rounded-[14px] border border-graphite-rail bg-void-black p-4">
          <p className="text-[11px] uppercase tracking-wide text-steel">Hub URL</p>
          <div className="mt-2 flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-[8px] border border-graphite-rail bg-black/40 px-2.5 py-1.5 font-mono text-[12.5px] text-mist" data-selectable>
              {status?.url ?? `http://127.0.0.1:${settings?.hubPort ?? 7071}`}
            </code>
            {status?.url && <CopyButton value={status.url} label="" />}
          </div>
        </div>

        <div className="rounded-[14px] border border-graphite-rail bg-void-black p-4">
          <p className="text-[11px] uppercase tracking-wide text-steel">Visibility</p>
          <div className="mt-2 flex rounded-[10px] border border-graphite-rail p-0.5">
            <ModeTab active={!settings?.hubPublic} onClick={() => toggleMode(false)} icon={<Lock className="size-3.5" />} label="Private" />
            <ModeTab active={!!settings?.hubPublic} onClick={() => toggleMode(true)} icon={<Globe className="size-3.5" />} label="Public" />
          </div>
          <p className="mt-2 text-[11.5px] leading-relaxed text-steel">
            {settings?.hubPublic
              ? "Directory is world-readable. No join secret required."
              : "Token-gated directory + a join secret. Recommended."}
          </p>
        </div>
      </div>

      {/* Join secret (private only) */}
      {!settings?.hubPublic && secret && (
        <div className="mt-3 rounded-[14px] border border-graphite-rail bg-void-black p-4">
          <div className="flex items-center justify-between">
            <p className="text-[11px] uppercase tracking-wide text-steel">Join secret</p>
            <button onClick={() => setShowSecret((s) => !s)} className="no-drag text-steel hover:text-frost">
              {showSecret ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
            </button>
          </div>
          <div className="mt-2 flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-[8px] border border-graphite-rail bg-black/40 px-2.5 py-1.5 font-mono text-[12.5px] text-mist" data-selectable>
              {showSecret ? secret : "•".repeat(Math.min(secret.length, 40))}
            </code>
            <CopyButton value={secret} label="" />
          </div>
          <p className="mt-2 text-[11.5px] text-steel">Agents present this via <code className="font-mono">PARLER_JOIN_SECRET</code>. Share it out-of-band.</p>
        </div>
      )}

      {/* Connect line */}
      {snippet && (
        <div className="mt-4">
          <p className="mb-1.5 text-[12px] uppercase tracking-wide text-steel">Connect an agent to this hub</p>
          <CodeBlock code={snippet} />
        </div>
      )}

      {/* Data folder + advanced */}
      <div className="mt-4 flex flex-wrap items-center gap-3">
        <Button variant="outline" size="sm" onClick={() => parler.hub.openDataFolder()}>
          <FolderOpen className="size-3.5" /> Open data folder
        </Button>
        {storage && <span className="font-mono text-[11.5px] text-steel">{storage.dataDir}</span>}
      </div>

      <AdvancedPort settings={settings} onUpdateSettings={onUpdateSettings} running={running || starting} onRestart={() => run(() => parler.hub.restart())} />

      {/* Logs */}
      <div className="mt-8">
        <p className="mb-2 text-[12px] uppercase tracking-wide text-steel">Hub logs</p>
        <div
          ref={logRef}
          className="h-[240px] overflow-y-auto rounded-[12px] border border-graphite-rail bg-black/60 p-3 font-mono text-[11.5px] leading-relaxed text-mist"
          data-selectable
        >
          {logs.length === 0 ? (
            <p className="text-steel">No output yet. Start the hub to see its logs.</p>
          ) : (
            logs.map((l, i) => (
              <div key={i} className={cn("whitespace-pre-wrap break-all", l.startsWith("$") && "text-electric-blue")}>
                {l}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

function AdvancedPort({
  settings,
  onUpdateSettings,
  running,
  onRestart,
}: {
  settings: Settings | null;
  onUpdateSettings: (patch: Partial<Settings>) => Promise<void>;
  running: boolean;
  onRestart: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState(settings?.hubName ?? "");
  const [port, setPort] = useState(String(settings?.hubPort ?? 7071));
  useEffect(() => {
    setName(settings?.hubName ?? "");
    setPort(String(settings?.hubPort ?? 7071));
  }, [settings?.hubName, settings?.hubPort]);

  const dirty = name !== settings?.hubName || port !== String(settings?.hubPort);
  const save = async () => {
    const p = parseInt(port, 10);
    await onUpdateSettings({ hubName: name.trim() || settings?.hubName, hubPort: Number.isFinite(p) ? p : settings?.hubPort });
    if (running) onRestart();
  };

  return (
    <div className="mt-4">
      <button onClick={() => setOpen((o) => !o)} className="no-drag text-[12.5px] text-steel hover:text-frost">
        {open ? "▾" : "▸"} Advanced settings
      </button>
      {open && (
        <div className="mt-3 grid grid-cols-1 gap-3 rounded-[14px] border border-graphite-rail bg-void-black p-4 sm:grid-cols-[1fr_140px_auto] sm:items-end">
          <label className="block">
            <span className="mb-1.5 block text-[11px] uppercase tracking-wide text-steel">Hub name</span>
            <Input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <label className="block">
            <span className="mb-1.5 block text-[11px] uppercase tracking-wide text-steel">Port</span>
            <Input value={port} onChange={(e) => setPort(e.target.value)} />
          </label>
          <Button variant="primary" size="sm" onClick={save} disabled={!dirty} className="h-10">
            Save{running ? " & restart" : ""}
          </Button>
        </div>
      )}
    </div>
  );
}

function Stat({ label, value, icon, accent }: { label: string; value: string; icon?: React.ReactNode; accent?: string }) {
  return (
    <div className="rounded-[14px] border border-graphite-rail bg-void-black p-4">
      <p className="flex items-center gap-1.5 text-[11px] uppercase tracking-wide text-steel">
        {icon} {label}
      </p>
      <p className="mt-1.5 text-[17px] font-semibold text-frost" style={accent ? { color: accent } : undefined}>
        {value}
      </p>
    </div>
  );
}

function ModeTab({ active, onClick, icon, label }: { active: boolean; onClick: () => void; icon: React.ReactNode; label: string }) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "no-drag flex flex-1 items-center justify-center gap-1.5 rounded-[8px] px-3 py-1.5 text-[13px] font-medium transition-colors",
        active ? "bg-electric-blue/15 text-pure-white" : "text-steel hover:text-frost",
      )}
    >
      {icon}
      {label}
    </button>
  );
}
