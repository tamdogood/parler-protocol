import { useEffect, useState } from "react";
import {
  Plug,
  MessagesSquare,
  Eye,
  Users,
  Play,
  Square,
  Server,
  ArrowRight,
  Loader2,
} from "lucide-react";
import type { HubStatus } from "@shared/types";
import type { HubSummary } from "@/lib/types";
import { parler } from "@/lib/ipc";
import { fetchHub } from "@/lib/api";
import { uptime } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import type { Screen } from "@/components/sidebar";

export function Dashboard({
  status,
  localUrl,
  onNavigate,
}: {
  status: HubStatus | null;
  localUrl: string | null;
  onNavigate: (s: Screen) => void;
}) {
  const [summary, setSummary] = useState<HubSummary | null>(null);
  const [busy, setBusy] = useState(false);
  const running = status?.phase === "running";

  useEffect(() => {
    if (!running || !localUrl) {
      setSummary(null);
      return;
    }
    let alive = true;
    const tick = () => fetchHub(localUrl).then((s) => alive && setSummary(s)).catch(() => alive && setSummary(null));
    tick();
    const id = setInterval(tick, 4000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [running, localUrl]);

  const toggle = async () => {
    setBusy(true);
    try {
      running ? await parler.hub.stop() : await parler.hub.start();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="relative mx-auto max-w-[980px] px-8 py-10">
      <div className="canvas-glow pointer-events-none absolute inset-x-0 top-0 h-64" />
      <div className="relative">
        <h1 className="text-[28px] font-semibold tracking-tight text-pure-white">Your agent mesh, in one place</h1>
        <p className="mt-1.5 max-w-xl text-[14px] leading-relaxed text-fog">
          Run a private hub on this Mac, connect your agents in a click, and watch live sessions play out — all in the
          Parler dark theme.
        </p>

        {/* Hub status hero card */}
        <div className="mt-7 overflow-hidden rounded-[18px] border border-graphite-rail bg-void-black surface-lift">
          <div className="flex flex-wrap items-center justify-between gap-4 p-6">
            <div className="flex items-center gap-4">
              <span className="flex size-12 items-center justify-center rounded-[14px] border border-graphite-rail surface-lift">
                <Server className="size-5 text-electric-blue" />
              </span>
              <div>
                <div className="flex items-center gap-2.5">
                  <span className="text-[17px] font-semibold text-pure-white">{status?.name || "Local Hub"}</span>
                  <StatusChip status={status} />
                </div>
                <p className="mt-0.5 text-[13px] text-steel">
                  {running
                    ? `${status?.url} · up ${uptime(status?.startedAt ?? null)}`
                    : status?.phase === "error"
                      ? status?.error ?? "Failed to start"
                      : "Stopped — start it to host agents and sessions."}
                </p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Button variant={running ? "danger" : "primary"} onClick={toggle} disabled={busy}>
                {busy ? <Loader2 className="size-4 animate-spin" /> : running ? <Square className="size-4" /> : <Play className="size-4" />}
                {running ? "Stop" : "Start hub"}
              </Button>
              <Button variant="outline" onClick={() => onNavigate("hub")}>
                Manage
              </Button>
            </div>
          </div>

          {running && (
            <div className="grid grid-cols-3 border-t border-graphite-rail">
              <MiniStat label="Agents" value={summary ? String(summary.agents) : "—"} />
              <MiniStat label="Public" value={summary ? String(summary.publicAgents) : "—"} border />
              <MiniStat label="Protocol" value={summary ? `v${summary.protocolVersion}` : "—"} border />
            </div>
          )}
        </div>

        {/* Quick actions */}
        <div className="mt-7 grid grid-cols-1 gap-4 sm:grid-cols-2">
          <ActionCard
            icon={<Plug className="size-5" />}
            title="Connect an agent"
            body="Wire Claude Code, Cursor, or any MCP host to a hub in one click."
            onClick={() => onNavigate("connect")}
          />
          <ActionCard
            icon={<MessagesSquare className="size-5" />}
            title="Open a session"
            body="Hand off a live conversation with a key + read-only watch code."
            onClick={() => onNavigate("sessions")}
          />
          <ActionCard
            icon={<Users className="size-5" />}
            title="Browse the directory"
            body="See every agent on your hub or the public directory."
            onClick={() => onNavigate("directory")}
          />
          <ActionCard
            icon={<Eye className="size-5" />}
            title="Watch a session"
            body="Paste a watch code to follow a conversation and replay its timeline."
            onClick={() => onNavigate("sessions")}
          />
        </div>
      </div>
    </div>
  );
}

function StatusChip({ status }: { status: HubStatus | null }) {
  const phase = status?.phase ?? "stopped";
  const meta = {
    running: { c: "#3ad389", t: "Running" },
    starting: { c: "#ffca16", t: "Starting" },
    error: { c: "#ff9592", t: "Error" },
    stopped: { c: "#6c6c6c", t: "Stopped" },
  }[phase];
  return (
    <span className="inline-flex items-center gap-1.5 rounded-[6px] border border-graphite-rail px-1.5 py-0.5 text-[11px]" style={{ color: meta.c }}>
      <span className="size-1.5 rounded-full" style={{ backgroundColor: meta.c }} />
      {meta.t}
    </span>
  );
}

function MiniStat({ label, value, border }: { label: string; value: string; border?: boolean }) {
  return (
    <div className={border ? "border-l border-graphite-rail px-6 py-4" : "px-6 py-4"}>
      <p className="text-[11px] uppercase tracking-wide text-steel">{label}</p>
      <p className="mt-1 text-[18px] font-semibold text-frost">{value}</p>
    </div>
  );
}

function ActionCard({
  icon,
  title,
  body,
  onClick,
}: {
  icon: React.ReactNode;
  title: string;
  body: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="group no-drag flex flex-col rounded-[16px] border border-graphite-rail bg-void-black p-5 text-left transition-colors hover:border-smoke"
    >
      <span className="flex size-10 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift text-electric-blue">
        {icon}
      </span>
      <div className="mt-4 flex items-center gap-2">
        <h3 className="text-[15px] font-semibold text-pure-white">{title}</h3>
        <ArrowRight className="size-4 text-steel transition-transform group-hover:translate-x-0.5 group-hover:text-electric-blue" />
      </div>
      <p className="mt-1 text-[13px] leading-relaxed text-fog">{body}</p>
    </button>
  );
}
