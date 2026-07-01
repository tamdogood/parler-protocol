import { Cloud, HardDrive } from "lucide-react";
import type { HubStatus, HubTarget } from "@shared/types";
import { cn } from "@/lib/utils";

/** The frameless title bar: drag region, global hub-target switch, and a live hub status pill. */
export function TitleBar({
  status,
  target,
  onTarget,
  onOpenHub,
}: {
  status: HubStatus | null;
  target: HubTarget;
  onTarget: (t: HubTarget) => void;
  onOpenHub: () => void;
}) {
  return (
    <header className="drag flex h-[44px] shrink-0 items-center justify-between border-b border-graphite-rail bg-black pl-[86px] pr-3">
      <div className="flex items-center gap-2">
        <span className="text-[13px] font-semibold tracking-tight text-frost">Parler</span>
        <span className="text-[12px] text-steel">Desktop</span>
      </div>

      <div className="flex items-center gap-2">
        {/* Which hub the Directory + Sessions views inspect. */}
        <div className="no-drag flex rounded-[9px] border border-graphite-rail p-0.5">
          <TargetTab active={target === "local"} onClick={() => onTarget("local")} icon={<HardDrive className="size-3.5" />} label="Local hub" />
          <TargetTab active={target === "public"} onClick={() => onTarget("public")} icon={<Cloud className="size-3.5" />} label="Public hub" />
        </div>
        <HubPill status={status} onClick={onOpenHub} />
      </div>
    </header>
  );
}

function TargetTab({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex items-center gap-1.5 rounded-[7px] px-2.5 py-1 text-[12px] font-medium transition-colors",
        active ? "bg-electric-blue/15 text-pure-white" : "text-steel hover:text-frost",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

function HubPill({ status, onClick }: { status: HubStatus | null; onClick: () => void }) {
  const phase = status?.phase ?? "stopped";
  const meta = {
    running: { color: "#3ad389", label: status?.healthy ? "Hub running" : "Hub starting" },
    starting: { color: "#ffca16", label: "Hub starting" },
    error: { color: "#ff9592", label: "Hub error" },
    stopped: { color: "#6c6c6c", label: "Hub stopped" },
  }[phase];
  const live = phase === "running" || phase === "starting";
  return (
    <button
      onClick={onClick}
      className="no-drag inline-flex items-center gap-2 rounded-[9px] border border-graphite-rail px-2.5 py-1 text-[12px] text-fog transition-colors hover:border-smoke hover:text-frost"
      title="Open Local Hub"
    >
      <span className="relative inline-flex size-2 items-center justify-center">
        {live && (
          <span className="absolute inline-flex size-full animate-ping rounded-full opacity-60" style={{ backgroundColor: meta.color }} />
        )}
        <span className="relative inline-flex size-2 rounded-full" style={{ backgroundColor: meta.color }} />
      </span>
      {meta.label}
    </button>
  );
}
