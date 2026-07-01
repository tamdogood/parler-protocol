import { useCallback, useEffect, useState } from "react";
import { Plug, Check, RefreshCw, AlertTriangle, Terminal, Loader2, ChevronDown, Cloud } from "lucide-react";
import type { ConnectSnippet, HubStatus, HubTarget, McpHost } from "@shared/types";
import { parler } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { CodeBlock } from "@/components/copyable";

/** Connect an agent — the app's primary job. Local hub by default; public is a quiet advanced option. */
export function ConnectScreen({
  status,
  onStartHub,
  onGoToAgents,
}: {
  status: HubStatus | null;
  onStartHub: () => void;
  onGoToAgents: () => void;
}) {
  const [target, setTarget] = useState<HubTarget>("local");
  const [hosts, setHosts] = useState<McpHost[] | null>(null);
  const [snippet, setSnippet] = useState<ConnectSnippet | null>(null);
  const [showManual, setShowManual] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [msg, setMsg] = useState<{ host: string; ok: boolean; text: string } | null>(null);

  const refresh = useCallback(async () => {
    const [h, s] = await Promise.all([parler.agents.detectHosts(), parler.agents.snippet(target)]);
    setHosts(h);
    setSnippet(s);
  }, [target]);

  useEffect(() => {
    setHosts(null);
    refresh();
  }, [refresh]);

  const localNotReady = target === "local" && status?.phase !== "running";

  const act = async (host: McpHost, action: "connect" | "disconnect") => {
    setBusy(host.id);
    setMsg(null);
    try {
      const res = action === "connect" ? await parler.agents.connect(host.id, target) : await parler.agents.disconnect(host.id);
      setMsg({ host: host.id, ok: res.ok, text: res.message });
      await refresh();
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="mx-auto max-w-[720px] px-8 py-8">
      <div className="flex items-center gap-3">
        <span className="flex size-11 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
          <Plug className="size-5 text-electric-blue" />
        </span>
        <div>
          <h1 className="text-[22px] font-semibold tracking-tight text-pure-white">Connect an agent</h1>
          <p className="text-[13px] text-fog">Adding the MCP server is the whole setup — no config to write.</p>
        </div>
      </div>

      {localNotReady && (
        <div className="mt-6 flex flex-wrap items-center gap-3 rounded-[12px] border border-complained-yellow/30 bg-complained-yellow/5 px-4 py-3 text-[13px] text-complained-yellow">
          <AlertTriangle className="size-4 shrink-0" />
          <span className="flex-1">Your local hub isn&apos;t running yet.</span>
          <Button variant="outline" size="sm" onClick={onStartHub}>
            Start hub
          </Button>
        </div>
      )}

      <div className="mt-6 flex flex-col gap-3">
        {hosts === null ? (
          <div className="flex items-center gap-2 py-8 text-[13px] text-steel">
            <Loader2 className="size-4 animate-spin" /> Detecting hosts…
          </div>
        ) : (
          hosts.map((host) => (
            <HostRow
              key={host.id}
              host={host}
              target={target}
              busy={busy === host.id}
              disabled={localNotReady}
              message={msg?.host === host.id ? msg : null}
              onConnect={() => act(host, "connect")}
              onDisconnect={() => act(host, "disconnect")}
            />
          ))
        )}
      </div>

      <div className="mt-3 flex items-center justify-between">
        <button
          onClick={() => setTarget(target === "local" ? "public" : "local")}
          className="no-drag inline-flex items-center gap-1.5 text-[12.5px] text-steel transition-colors hover:text-frost"
        >
          <Cloud className="size-3.5" />
          {target === "local" ? "Connect to the public hub instead" : "Connect to my local hub instead"}
        </button>
        <Button variant="subtle" size="sm" onClick={refresh}>
          <RefreshCw className="size-3.5" /> Refresh
        </Button>
      </div>

      {/* Manual setup — collapsed by default. */}
      {snippet && (
        <div className="mt-6 rounded-[14px] border border-graphite-rail bg-void-black">
          <button
            onClick={() => setShowManual((v) => !v)}
            className="no-drag flex w-full items-center justify-between px-4 py-3 text-left text-[13px] font-medium text-frost"
          >
            Set up another MCP host manually
            <ChevronDown className={cn("size-4 text-steel transition-transform", showManual && "rotate-180")} />
          </button>
          {showManual && (
            <div className="border-t border-graphite-rail p-4">
              <p className="mb-2 text-[12px] uppercase tracking-wide text-steel">Claude Code (one line)</p>
              <CodeBlock code={snippet.shell} />
              <p className="mb-2 mt-4 text-[12px] uppercase tracking-wide text-steel">Generic MCP config</p>
              <CodeBlock code={snippet.json} />
              <p className="mt-3 text-[12px] text-steel">
                Connected agents appear under{" "}
                <button onClick={onGoToAgents} className="no-drag text-electric-blue hover:underline">
                  Agents
                </button>
                .
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function HostRow({
  host,
  target,
  busy,
  disabled,
  message,
  onConnect,
  onDisconnect,
}: {
  host: McpHost;
  target: HubTarget;
  busy: boolean;
  disabled: boolean;
  message: { ok: boolean; text: string } | null;
  onConnect: () => void;
  onDisconnect: () => void;
}) {
  const connectedHere = host.connected && host.connectedTarget === target;
  return (
    <div className="rounded-[14px] border border-graphite-rail bg-void-black p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <span className="flex size-9 items-center justify-center rounded-[10px] border border-graphite-rail surface-lift">
            <Terminal className="size-4 text-fog" />
          </span>
          <div>
            <div className="flex items-center gap-2">
              <span className="text-[14px] font-semibold text-frost">{host.name}</span>
              {connectedHere && (
                <span className="inline-flex items-center gap-1 rounded-[6px] border border-delivered-green/40 bg-delivered-green/5 px-1.5 py-0.5 text-[11px] text-delivered-green">
                  <Check className="size-3" /> Connected
                </span>
              )}
              {!host.installed && (
                <span className="rounded-[6px] border border-graphite-rail px-1.5 py-0.5 text-[11px] text-steel">not detected</span>
              )}
            </div>
            <p className="mt-0.5 text-[12px] text-steel">
              {connectedHere ? "Restart it to load the server." : host.installed ? "Ready to connect." : "Install it, or use manual setup below."}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {host.connected && (
            <Button variant="danger" size="sm" onClick={onDisconnect} disabled={busy}>
              Disconnect
            </Button>
          )}
          <Button variant={connectedHere ? "outline" : "primary"} size="sm" onClick={onConnect} disabled={busy || disabled}>
            {busy ? <Loader2 className="size-3.5 animate-spin" /> : null}
            {connectedHere ? "Reconnect" : "Connect"}
          </Button>
        </div>
      </div>
      {message && (
        <p className={cn("mt-3 text-[12.5px]", message.ok ? "text-delivered-green" : "text-bounced-red")}>{message.text}</p>
      )}
    </div>
  );
}
