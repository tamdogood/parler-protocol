import { Users, Plug, MessagesSquare, Server, Settings as Cog } from "lucide-react";
import type { HubStatus } from "@shared/types";
import { cn } from "@/lib/utils";

export type Screen = "agents" | "connect" | "sessions" | "settings" | "hub";

const NAV: { id: Screen; label: string; icon: React.ReactNode }[] = [
  { id: "agents", label: "Agents", icon: <Users className="size-[18px]" /> },
  { id: "connect", label: "Connect", icon: <Plug className="size-[18px]" /> },
  { id: "sessions", label: "Conversations", icon: <MessagesSquare className="size-[18px]" /> },
  { id: "hub", label: "Hub", icon: <Server className="size-[18px]" /> },
];

export function Sidebar({
  active,
  onNavigate,
  status,
  pendingJoins = 0,
}: {
  active: Screen;
  onNavigate: (s: Screen) => void;
  status: HubStatus | null;
  /** Agents waiting for approval across conversations — badged on the Conversations item. */
  pendingJoins?: number;
}) {
  const settingsActive = active === "settings";
  return (
    <nav className="flex w-[210px] shrink-0 flex-col border-r border-graphite-rail bg-black">
      <div className="flex flex-col gap-0.5 p-3">
        {NAV.map((item) => {
          const badge = item.id === "sessions" && pendingJoins > 0 ? pendingJoins : 0;
          return (
            <button
              key={item.id}
              onClick={() => onNavigate(item.id)}
              className={cn(
                "no-drag flex items-center gap-3 rounded-[10px] px-3 py-2 text-[13.5px] font-medium transition-colors",
                active === item.id ? "bg-white/[0.06] text-pure-white" : "text-steel hover:bg-white/[0.03] hover:text-frost",
              )}
            >
              <span className={cn(active === item.id ? "text-electric-blue" : "text-steel")}>{item.icon}</span>
              {item.label}
              {badge > 0 && (
                <span
                  className="ml-auto inline-flex min-w-5 items-center justify-center rounded-full bg-complained-yellow px-1.5 text-[11px] font-semibold text-black"
                  title={`${badge} ${badge === 1 ? "agent is" : "agents are"} waiting to join a conversation`}
                >
                  {badge}
                </span>
              )}
            </button>
          );
        })}
      </div>

      <div className="mt-auto p-3">
        <button
          onClick={() => onNavigate("settings")}
          className={cn(
            "no-drag flex w-full items-center gap-3 rounded-[10px] px-3 py-2 text-[13.5px] font-medium transition-colors",
            settingsActive ? "bg-white/[0.06] text-pure-white" : "text-steel hover:bg-white/[0.03] hover:text-frost",
          )}
        >
          <span className={settingsActive ? "text-electric-blue" : "text-steel"}>
            <Cog className="size-[18px]" />
          </span>
          Settings
        </button>
        <div className="mt-3 flex items-center gap-2 px-3 text-[12px] text-steel">
          <span
            className="size-2 rounded-full"
            style={{
              backgroundColor:
                status?.phase === "running" ? "#3ad389" : status?.phase === "error" ? "#ff9592" : "#6c6c6c",
            }}
          />
          {status?.phase === "running" ? "Hub online" : status?.phase === "starting" ? "Starting…" : "Hub offline"}
        </div>
      </div>
    </nav>
  );
}
