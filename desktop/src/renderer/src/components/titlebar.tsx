import type { HubStatus } from "@shared/types";

/** A clean frameless title bar: drag region + app name + a small hub status pill (→ Settings). */
export function TitleBar({ status, onOpenHub }: { status: HubStatus | null; onOpenHub: () => void }) {
  const phase = status?.phase ?? "stopped";
  const meta = {
    running: { color: "#3ad389", label: status?.healthy ? "Hub running" : "Hub starting" },
    starting: { color: "#ffca16", label: "Hub starting" },
    error: { color: "#ff9592", label: "Hub error" },
    stopped: { color: "#6c6c6c", label: "Hub stopped" },
  }[phase];
  const live = phase === "running" || phase === "starting";

  return (
    <header className="drag flex h-[44px] shrink-0 items-center justify-between border-b border-graphite-rail bg-black pl-[86px] pr-3">
      <div className="flex items-center gap-2">
        <span className="text-[13px] font-semibold tracking-tight text-frost">Parler Protocol</span>
      </div>
      <button
        onClick={onOpenHub}
        className="no-drag inline-flex items-center gap-2 rounded-[9px] border border-graphite-rail px-2.5 py-1 text-[12px] text-fog transition-colors hover:border-smoke hover:text-frost"
        title="Local hub settings"
      >
        <span className="relative inline-flex size-2 items-center justify-center">
          {live && <span className="absolute inline-flex size-full animate-ping rounded-full opacity-60" style={{ backgroundColor: meta.color }} />}
          <span className="relative inline-flex size-2 rounded-full" style={{ backgroundColor: meta.color }} />
        </span>
        {meta.label}
      </button>
    </header>
  );
}
