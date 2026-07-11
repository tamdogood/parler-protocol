import { useEffect, useState } from "react";
import { ArrowRight, Check, Loader2, Terminal, ShieldCheck, Sparkles, AlertTriangle } from "lucide-react";
import type { ConnectAllResult, HubStatus, McpHost } from "@shared/types";
import { parler } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useHubUrl } from "@/lib/hooks";
import { Button } from "@/components/ui/button";
import { CodeBlock } from "@/components/copyable";
import { DialInList } from "@/components/dial-in";

/** First-run setup: a welcome, then connect the first agent to the auto-started local hub. */
export function Onboarding({
  status,
  autoConnect,
  onFinish,
}: {
  status: HubStatus | null;
  autoConnect: boolean;
  onFinish: () => void;
}) {
  const [step, setStep] = useState(0);

  const start = () => {
    // Kick off the local hub so it's ready by the time they connect.
    if (status?.phase !== "running") void parler.hub.start();
    setStep(1);
  };

  return (
    <div className="canvas-glow fixed inset-0 z-50 flex items-center justify-center bg-black">
      <div className="drag absolute inset-x-0 top-0 h-11" />
      <div className="relative z-10 w-full max-w-[520px] px-8">
        {step === 0 ? (
          <Welcome onNext={start} />
        ) : (
          <ConnectFirst status={status} autoConnect={autoConnect} onFinish={onFinish} />
        )}
        <div className="mt-8 flex items-center justify-center gap-1.5">
          {[0, 1].map((i) => (
            <span key={i} className={cn("h-1 rounded-full transition-all", i === step ? "w-6 bg-electric-blue" : "w-1.5 bg-graphite-rail")} />
          ))}
        </div>
      </div>
    </div>
  );
}

function Logo() {
  return (
    <div className="flex justify-center">
      <div className="relative flex size-16 items-center justify-center rounded-[20px] border border-graphite-rail surface-lift">
        <span className="absolute size-9 rounded-full border-2 border-electric-blue/80" />
        <span className="size-3 rounded-full bg-resend-violet" />
        <span className="absolute right-2.5 top-2.5 size-2 rounded-full bg-electric-blue" />
      </div>
    </div>
  );
}

function Welcome({ onNext }: { onNext: () => void }) {
  return (
    <div className="text-center">
      <Logo />
      <h1 className="mt-6 text-[30px] font-semibold tracking-tight text-pure-white">Welcome to Parler Protocol</h1>
      <p className="mx-auto mt-2 max-w-sm text-[14px] leading-relaxed text-fog">
        A private hub for your AI agents, running on this Mac. Connect your agents and hand off live sessions — no
        copy-paste, no terminal.
      </p>
      <Button variant="primary" size="lg" className="mt-7" onClick={onNext}>
        Get started <ArrowRight className="size-4" />
      </Button>
      <p className="mt-4 flex items-center justify-center gap-1.5 text-[11.5px] text-steel">
        <ShieldCheck className="size-3.5 text-delivered-green" />
        Your identity&apos;s private key never leaves this Mac.
      </p>
    </div>
  );
}

function ConnectFirst({
  status,
  autoConnect,
  onFinish,
}: {
  status: HubStatus | null;
  autoConnect: boolean;
  onFinish: () => void;
}) {
  const [installed, setInstalled] = useState<McpHost[] | null>(null);
  const [snippet, setSnippet] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [autoTried, setAutoTried] = useState(false);
  const [result, setResult] = useState<ConnectAllResult | null>(null);
  const localUrl = useHubUrl("local", status);

  useEffect(() => {
    parler.agents.detectHosts().then((hs) => setInstalled(hs.filter((h) => h.installed)));
    parler.agents.snippet("local").then((s) => setSnippet(s.shell));
  }, [status?.phase]);

  const ready = status?.phase === "running";
  const hasAgents = (installed?.length ?? 0) > 0;
  const done = (result?.connected ?? 0) > 0;

  const connect = async () => {
    setBusy(true);
    try {
      setResult(await parler.agents.connectAll("local"));
    } finally {
      setBusy(false);
    }
  };

  // Zero-click setup: once the hub is up and agents are detected, wire them all automatically the
  // first time through. We only auto-try once — a failed/partial wire drops to the manual button
  // below so the user stays in control and can retry.
  useEffect(() => {
    if (!autoConnect || autoTried || busy || done) return;
    if (!ready || !hasAgents) return;
    setAutoTried(true);
    void connect();
    // `connect` is stable enough for this one-shot trigger; deps track the readiness gates only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoConnect, autoTried, busy, done, ready, hasAgents]);

  return (
    <div className="text-center">
      <div className="flex justify-center">
        <span className="flex size-12 items-center justify-center rounded-[14px] border border-graphite-rail surface-lift text-electric-blue">
          <Terminal className="size-5" />
        </span>
      </div>
      <h2 className="mt-5 text-[22px] font-semibold tracking-tight text-pure-white">Connecting your agents</h2>
      <p className="mx-auto mt-1.5 max-w-sm text-[13px] text-fog">
        {autoConnect
          ? "We're wiring every agent on this Mac to your hub for you — each mints its identity the first time it launches."
          : "Adding the MCP server is the whole setup — each agent mints its identity the first time it launches."}
      </p>

      {!ready && (
        <p className="mx-auto mt-4 flex max-w-sm items-center justify-center gap-2 text-[12.5px] text-complained-yellow">
          <Loader2 className="size-3.5 animate-spin" /> Starting your local hub…
        </p>
      )}

      <div className="mt-6 text-left">
        {installed === null ? (
          <p className="flex items-center gap-2 text-[13px] text-steel">
            <Loader2 className="size-4 animate-spin" /> Looking for agents on this Mac…
          </p>
        ) : done ? (
          <div className="rounded-[14px] border border-delivered-green/40 bg-delivered-green/5 p-4 text-[13px] text-delivered-green">
            <div className="flex items-center gap-2 font-medium">
              <Check className="size-5 shrink-0" /> Connected {result?.connected} agent{(result?.connected ?? 0) > 1 ? "s" : ""}.
            </div>
            <p className="mt-1.5 text-[12.5px] text-delivered-green/90">Restart them to load Parler Protocol — then they appear under Agents.</p>
            {localUrl && result && (
              <DialInList
                base={localUrl}
                hosts={result.results.filter((r) => r.status === "wired").map((r) => ({ id: r.id, name: r.name }))}
              />
            )}
          </div>
        ) : hasAgents ? (
          <div className="rounded-[14px] border border-graphite-rail bg-void-black p-4">
            <div className="flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="flex items-center gap-2 text-[14px] font-medium text-frost">
                  <Terminal className="size-4 text-electric-blue" /> {installed!.length} agent{installed!.length > 1 ? "s" : ""} detected
                </div>
                <p className="mt-0.5 truncate text-[12px] text-steel">{installed!.map((h) => h.name).join(", ")}</p>
              </div>
              <Button variant="primary" size="sm" onClick={connect} disabled={busy || !ready}>
                {busy ? (
                  <>
                    <Loader2 className="size-3.5 animate-spin" /> Connecting…
                  </>
                ) : (
                  <>
                    <Sparkles className="size-3.5" /> Connect all
                  </>
                )}
              </Button>
            </div>
            {result && result.results.some((r) => r.status !== "wired") && (
              <p className="mt-2 flex items-center gap-1.5 text-[12.5px] text-complained-yellow">
                <AlertTriangle className="size-3.5" /> Some agents couldn&apos;t be wired — retry from the Connect tab.
              </p>
            )}
          </div>
        ) : (
          <div>
            <p className="mb-2 text-[12px] uppercase tracking-wide text-steel">No agents detected — add to any MCP host</p>
            {snippet && <CodeBlock code={snippet} />}
          </div>
        )}
      </div>

      <div className="mt-7 flex items-center justify-center gap-3">
        {!done && (
          <button onClick={onFinish} className="no-drag text-[13px] text-steel hover:text-frost">
            Skip for now
          </button>
        )}
        <Button variant={done ? "primary" : "outline"} onClick={onFinish}>
          {done ? "Done" : "Finish"} <ArrowRight className="size-4" />
        </Button>
      </div>
    </div>
  );
}
