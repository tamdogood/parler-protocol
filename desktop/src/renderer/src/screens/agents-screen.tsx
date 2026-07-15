import { MessagesSquare, Plug, Play, AlertTriangle } from "lucide-react";
import type { HubStatus } from "@shared/types";
import { parler } from "@/lib/ipc";
import { Button } from "@/components/ui/button";
import { Directory } from "@/components/directory";

/** Home: the agents on your local hub, with one clear action — connect another. */
export function AgentsScreen({
  localUrl,
  status,
  onConnect,
  onStartSession,
}: {
  localUrl: string | null;
  status: HubStatus | null;
  onConnect: () => void;
  onStartSession: () => void;
}) {
  const down = status !== null && status.phase !== "running" && status.phase !== "starting";

  return (
    <div className="mx-auto max-w-[1120px] px-8 py-8">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-[22px] font-semibold tracking-tight text-pure-white">Agents</h1>
          <p className="text-[13px] text-fog">Everything connected to your hub.</p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" onClick={onConnect}>
            <Plug className="size-4" /> Manage connections
          </Button>
          <Button variant="primary" onClick={onStartSession}>
            <MessagesSquare className="size-4" /> Start a conversation
          </Button>
        </div>
      </div>

      {down && (
        <div className="mt-5 flex flex-wrap items-center gap-3 rounded-[12px] border border-complained-yellow/30 bg-complained-yellow/5 px-4 py-3 text-[13px] text-complained-yellow">
          <AlertTriangle className="size-4 shrink-0" />
          <span className="flex-1">Your local hub is stopped — start it to see and connect agents.</span>
          <Button variant="outline" size="sm" onClick={() => parler.hub.start()}>
            <Play className="size-3.5" /> Start hub
          </Button>
        </div>
      )}

      <div className="mt-6">
        {localUrl ? <Directory base={localUrl} onConnect={onConnect} /> : <p className="text-[13px] text-steel">Starting…</p>}
      </div>
    </div>
  );
}
