import { LayoutDashboard, Server, Users, MessagesSquare, Plug, Settings as Cog } from "lucide-react";
import type { HubStatus } from "@shared/types";
import { cn } from "@/lib/utils";

export type Screen = "dashboard" | "hub" | "directory" | "sessions" | "connect" | "settings";

const NAV: { id: Screen; label: string; icon: React.ReactNode }[] = [
  { id: "dashboard", label: "Dashboard", icon: <LayoutDashboard className="size-[18px]" /> },
  { id: "hub", label: "Local Hub", icon: <Server className="size-[18px]" /> },
  { id: "directory", label: "Directory", icon: <Users className="size-[18px]" /> },
  { id: "sessions", label: "Sessions", icon: <MessagesSquare className="size-[18px]" /> },
  { id: "connect", label: "Connect", icon: <Plug className="size-[18px]" /> },
  { id: "settings", label: "Settings", icon: <Cog className="size-[18px]" /> },
];

export function Sidebar({
  active,
  onNavigate,
  status,
  version,
}: {
  active: Screen;
  onNavigate: (s: Screen) => void;
  status: HubStatus | null;
  version: string;
}) {
  return (
    <nav className="flex w-[216px] shrink-0 flex-col border-r border-graphite-rail bg-black">
      <div className="flex flex-col gap-0.5 p-3">
        {NAV.map((item) => (
          <button
            key={item.id}
            onClick={() => onNavigate(item.id)}
            className={cn(
              "no-drag flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13.5px] font-medium transition-colors",
              active === item.id
                ? "bg-white/[0.06] text-pure-white"
                : "text-steel hover:bg-white/[0.03] hover:text-frost",
            )}
          >
            <span className={cn(active === item.id ? "text-electric-blue" : "text-steel")}>{item.icon}</span>
            {item.label}
          </button>
        ))}
      </div>

      <div className="mt-auto border-t border-graphite-rail p-4">
        <div className="flex items-center gap-2 text-[12px] text-steel">
          <span
            className="size-2 rounded-full"
            style={{
              backgroundColor:
                status?.phase === "running" ? "#3ad389" : status?.phase === "error" ? "#ff9592" : "#6c6c6c",
            }}
          />
          {status?.phase === "running" ? "Local hub online" : status?.phase === "starting" ? "Starting…" : "Local hub offline"}
        </div>
        <p className="mt-2 font-mono text-[11px] text-steel/70">v{version}</p>
      </div>
    </nav>
  );
}
