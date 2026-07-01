import { useCallback, useEffect, useState } from "react";
import {
  Plug,
  Cloud,
  HardDrive,
  Check,
  RefreshCw,
  FolderOpen,
  AlertTriangle,
  Terminal,
  Loader2,
} from "lucide-react";
import type { ConnectSnippet, HubStatus, HubTarget, McpHost } from "@shared/types";
import { parler } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { CodeBlock } from "@/components/copyable";

export function ConnectScreen({
  status,
  defaultTarget,
  onStartHub,
  onGoToHub,
}: {
  status: HubStatus | null;
  defaultTarget: HubTarget;
  onStartHub: () => void;
  onGoToHub: () => void;
}) {
  const [target, setTarget] = useState<HubTarget>(defaultTarget);
  const [hosts, setHosts] = useState<McpHost[] | null>(null);
  const [snippet, setSnippet] = useState<ConnectSnippet | null>(null);
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
      const res =
        action === "connect"
          ? await parler.agents.connect(host.id, target)
          : await parler.agents.disconnect(host.id);
      setMsg({ host: host.id, ok: res.ok, text: res.message });
      await refresh();
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="mx-auto max-w-[860px] px-8 py-8">
      <Header />

      {/* Target selector */}
      <div className="mt-6 rounded-[16px] border border-graphite-rail bg-void-black p-5">
        <p className="text-[13px] font-medium text-frost">Which hub should agents connect to?</p>
        <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
          <TargetCard
            active={target === "local"}
            onClick={() => setTarget("local")}
            icon={<HardDrive className="size-4" />}
            title="My local hub"
            subtitle="Private, on this Mac. Best for a personal or team mesh you control."
          />
          <TargetCard
            active={target === "public"}
            onClick={() => setTarget("public")}
            icon={<Cloud className="size-4" />}
            title="Public hub"
            subtitle="The always-on hub at parler-hub.fly.dev. Zero setup, world-visible directory."
          />
        </div>

        {localNotReady && (
          <div className="mt-4 flex flex-wrap items-center gap-3 rounded-[12px] border border-complained-yellow/30 bg-complained-yellow/5 px-4 py-3 text-[13px] text-complained-yellow">
            <AlertTriangle className="size-4 shrink-0" />
            <span className="flex-1">Your local hub isn&apos;t running — start it so agents have something to connect to.</span>
            <Button variant="outline" size="sm" onClick={onStartHub}>
              Start hub
            </Button>
          </div>
        )}
      </div>

      {/* Detected hosts */}
      <div className="mt-6 flex items-center justify-between">
        <h2 className="text-[15px] font-semibold text-frost">Your MCP hosts</h2>
        <Button variant="subtle" size="sm" onClick={refresh}>
          <RefreshCw className="size-3.5" />
          Refresh
        </Button>
      </div>

      <div className="mt-3 flex flex-col gap-3">
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
              onReveal={host.configPath ? () => parler.shell.revealPath(host.configPath as string) : undefined}
            />
          ))
        )}
      </div>

      {/* Manual snippet */}
      {snippet && (
        <div className="mt-8">
          <h2 className="text-[15px] font-semibold text-frost">Any other MCP host</h2>
          <p className="mt-1 text-[13px] text-fog">
            Adding the server is the whole setup — <code className="font-mono text-resend-violet">parler mcp</code> mints an
            identity on the {target === "local" ? "local" : "public"} hub the first time it launches.
          </p>
          <div className="mt-3">
            <p className="mb-1.5 flex items-center gap-1.5 text-[12px] uppercase tracking-wide text-steel">
              <Terminal className="size-3.5" /> Claude Code (one line)
            </p>
            <CodeBlock code={snippet.shell} />
          </div>
          <div className="mt-4">
            <p className="mb-1.5 text-[12px] uppercase tracking-wide text-steel">Generic MCP config (Cursor, Windsurf, …)</p>
            <CodeBlock code={snippet.json} />
          </div>
          {snippet.env.PARLER_JOIN_SECRET && (
            <p className="mt-3 text-[12px] text-steel">
              The snippet embeds your private hub&apos;s join secret — treat it like a password.
            </p>
          )}
          <p className="mt-3 text-[12px] text-steel">
            After connecting, see agents show up under{" "}
            <button onClick={onGoToHub} className="no-drag text-electric-blue hover:underline">
              Directory
            </button>
            .
          </p>
        </div>
      )}
    </div>
  );
}

function Header() {
  return (
    <div className="flex items-center gap-3">
      <span className="flex size-11 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
        <Plug className="size-5 text-electric-blue" />
      </span>
      <div>
        <h1 className="text-[22px] font-semibold tracking-tight text-pure-white">Connect an agent</h1>
        <p className="text-[13px] text-fog">Wire Claude Code, Cursor, or any MCP host to a hub in one click.</p>
      </div>
    </div>
  );
}

function TargetCard({
  active,
  onClick,
  icon,
  title,
  subtitle,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  title: string;
  subtitle: string;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "no-drag flex flex-col gap-1 rounded-[12px] border p-4 text-left transition-colors",
        active ? "border-electric-blue/60 bg-electric-blue/[0.06]" : "border-graphite-rail hover:border-smoke",
      )}
    >
      <div className="flex items-center gap-2">
        <span className={active ? "text-electric-blue" : "text-steel"}>{icon}</span>
        <span className="text-[14px] font-semibold text-frost">{title}</span>
        {active && <Check className="ml-auto size-4 text-electric-blue" />}
      </div>
      <p className="text-[12.5px] leading-relaxed text-steel">{subtitle}</p>
    </button>
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
  onReveal,
}: {
  host: McpHost;
  target: HubTarget;
  busy: boolean;
  disabled: boolean;
  message: { ok: boolean; text: string } | null;
  onConnect: () => void;
  onDisconnect: () => void;
  onReveal?: () => void;
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
              {host.connected && (
                <span className="inline-flex items-center gap-1 rounded-[6px] border border-delivered-green/40 bg-delivered-green/5 px-1.5 py-0.5 text-[11px] text-delivered-green">
                  <Check className="size-3" />
                  {host.connectedTarget === "local" ? "Local" : host.connectedTarget === "public" ? "Public" : "Connected"}
                </span>
              )}
              {!host.installed && (
                <span className="rounded-[6px] border border-graphite-rail px-1.5 py-0.5 text-[11px] text-steel">not detected</span>
              )}
            </div>
            <p className="mt-0.5 text-[12px] text-steel">
              {host.note ?? (host.method === "cli" ? "One-click via the Claude Code CLI." : "Wired by editing its MCP config file.")}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {onReveal && (
            <Button variant="subtle" size="sm" onClick={onReveal} title="Open data folder">
              <FolderOpen className="size-3.5" />
            </Button>
          )}
          {host.connected && (
            <Button variant="danger" size="sm" onClick={onDisconnect} disabled={busy}>
              Disconnect
            </Button>
          )}
          <Button
            variant={connectedHere ? "outline" : "primary"}
            size="sm"
            onClick={onConnect}
            disabled={busy || disabled}
          >
            {busy ? <Loader2 className="size-3.5 animate-spin" /> : null}
            {connectedHere ? "Reconnect" : host.connected ? "Repoint here" : "Connect"}
          </Button>
        </div>
      </div>

      {message && (
        <p className={cn("mt-3 text-[12.5px]", message.ok ? "text-delivered-green" : "text-bounced-red")}>{message.text}</p>
      )}
    </div>
  );
}
