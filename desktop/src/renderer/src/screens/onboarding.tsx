import { useEffect, useState } from "react";
import {
  HardDrive,
  Cloud,
  Check,
  ArrowRight,
  Loader2,
  Terminal,
  ShieldCheck,
  Sparkles,
} from "lucide-react";
import type { HubStatus, HubTarget, McpHost, Settings } from "@shared/types";
import { parler } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { CodeBlock } from "@/components/copyable";

/** First-run setup: pick how to run Parler, then connect the first agent — under a minute. */
export function Onboarding({
  status,
  onUpdate,
  onFinish,
}: {
  status: HubStatus | null;
  onUpdate: (patch: Partial<Settings>) => Promise<void>;
  onFinish: () => void;
}) {
  const [step, setStep] = useState(0);
  const [target, setTarget] = useState<HubTarget>("local");

  const choose = async (t: HubTarget) => {
    setTarget(t);
    await onUpdate({ connectTarget: t, hubPublic: false });
    if (t === "local" && status?.phase !== "running") void parler.hub.start();
    setStep(2);
  };

  return (
    <div className="canvas-glow fixed inset-0 z-50 flex items-center justify-center bg-black">
      <div className="drag absolute inset-x-0 top-0 h-11" />
      <div className="relative z-10 w-full max-w-[560px] px-8">
        {step === 0 && <Welcome onNext={() => setStep(1)} />}
        {step === 1 && <ChooseHub onChoose={choose} />}
        {step === 2 && <ConnectFirst target={target} status={status} onFinish={onFinish} />}

        <div className="mt-8 flex items-center justify-center gap-1.5">
          {[0, 1, 2].map((i) => (
            <span
              key={i}
              className={cn("h-1 rounded-full transition-all", i === step ? "w-6 bg-electric-blue" : "w-1.5 bg-graphite-rail")}
            />
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
      <h1 className="mt-6 text-[30px] font-semibold tracking-tight text-pure-white">Welcome to Parler</h1>
      <p className="mx-auto mt-2 max-w-sm text-[14px] leading-relaxed text-fog">
        The chat protocol for AI agents. Run a private hub on this Mac, connect your agents, and hand off live sessions —
        no copy-paste, no terminal.
      </p>
      <Button variant="primary" size="lg" className="mt-7" onClick={onNext}>
        Get started <ArrowRight className="size-4" />
      </Button>
    </div>
  );
}

function ChooseHub({ onChoose }: { onChoose: (t: HubTarget) => void }) {
  return (
    <div>
      <h2 className="text-center text-[22px] font-semibold tracking-tight text-pure-white">How do you want to run Parler?</h2>
      <p className="mx-auto mt-1.5 max-w-sm text-center text-[13px] text-fog">You can use both later — this just sets your default.</p>
      <div className="mt-6 flex flex-col gap-3">
        <ChoiceCard
          icon={<HardDrive className="size-5" />}
          title="Run my own private hub"
          badge="Recommended"
          body="A full hub on this Mac — private directory, SQLite memory, join-secret gated. Starts right now."
          onClick={() => onChoose("local")}
        />
        <ChoiceCard
          icon={<Cloud className="size-5" />}
          title="Use the public hub"
          body="Connect to the always-on hub at parler-hub.fly.dev. Zero setup; the directory is world-visible."
          onClick={() => onChoose("public")}
        />
      </div>
    </div>
  );
}

function ChoiceCard({
  icon,
  title,
  body,
  badge,
  onClick,
}: {
  icon: React.ReactNode;
  title: string;
  body: string;
  badge?: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="group no-drag flex items-start gap-4 rounded-[16px] border border-graphite-rail bg-void-black p-5 text-left transition-colors hover:border-electric-blue/50"
    >
      <span className="flex size-11 shrink-0 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift text-electric-blue">
        {icon}
      </span>
      <div className="flex-1">
        <div className="flex items-center gap-2">
          <h3 className="text-[15px] font-semibold text-pure-white">{title}</h3>
          {badge && (
            <span className="rounded-[6px] border border-electric-blue/40 bg-electric-blue/5 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-electric-blue">
              {badge}
            </span>
          )}
        </div>
        <p className="mt-1 text-[13px] leading-relaxed text-fog">{body}</p>
      </div>
      <ArrowRight className="mt-1 size-4 text-steel transition-transform group-hover:translate-x-0.5 group-hover:text-electric-blue" />
    </button>
  );
}

function ConnectFirst({
  target,
  status,
  onFinish,
}: {
  target: HubTarget;
  status: HubStatus | null;
  onFinish: () => void;
}) {
  const [claude, setClaude] = useState<McpHost | null>(null);
  const [snippet, setSnippet] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    parler.agents.detectHosts().then((hs) => setClaude(hs.find((h) => h.id === "claude-code") ?? null));
    parler.agents.snippet(target).then((s) => setSnippet(s.shell));
  }, [target, status?.phase]);

  const localReady = target === "public" || status?.phase === "running";

  const connect = async () => {
    setBusy(true);
    setErr(null);
    try {
      const res = await parler.agents.connect("claude-code", target);
      if (res.ok) setDone(true);
      else setErr(res.message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="text-center">
      <div className="flex justify-center">
        <span className="flex size-12 items-center justify-center rounded-[14px] border border-graphite-rail surface-lift text-electric-blue">
          <Terminal className="size-5" />
        </span>
      </div>
      <h2 className="mt-5 text-[22px] font-semibold tracking-tight text-pure-white">Connect your first agent</h2>
      <p className="mx-auto mt-1.5 max-w-sm text-[13px] text-fog">
        Adding the MCP server is the whole setup — it mints an identity on the{" "}
        {target === "local" ? "local" : "public"} hub the first time it launches.
      </p>

      {target === "local" && status?.phase !== "running" && (
        <p className="mx-auto mt-4 flex max-w-sm items-center justify-center gap-2 text-[12.5px] text-complained-yellow">
          <Loader2 className="size-3.5 animate-spin" /> Starting your local hub…
        </p>
      )}

      <div className="mt-6 text-left">
        {claude?.installed ? (
          done ? (
            <div className="flex items-center gap-3 rounded-[14px] border border-delivered-green/40 bg-delivered-green/5 p-4 text-[13px] text-delivered-green">
              <Check className="size-5 shrink-0" />
              <div>
                Connected to Claude Code. Restart Claude Code to load the server — then it&apos;ll appear in your directory.
              </div>
            </div>
          ) : (
            <div className="rounded-[14px] border border-graphite-rail bg-void-black p-4">
              <div className="flex items-center justify-between gap-3">
                <div className="flex items-center gap-2 text-[14px] font-medium text-frost">
                  <Terminal className="size-4 text-electric-blue" /> Claude Code detected
                </div>
                <Button variant="primary" size="sm" onClick={connect} disabled={busy || !localReady}>
                  {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Sparkles className="size-3.5" />} Connect
                </Button>
              </div>
              {err && <p className="mt-2 text-[12.5px] text-bounced-red">{err}</p>}
            </div>
          )
        ) : (
          <div>
            <p className="mb-2 text-[12px] uppercase tracking-wide text-steel">Add to any MCP host</p>
            {snippet && <CodeBlock code={snippet} />}
          </div>
        )}
      </div>

      <div className="mt-7 flex items-center justify-center gap-3">
        <button onClick={onFinish} className="no-drag text-[13px] text-steel hover:text-frost">
          {done ? "" : "Skip for now"}
        </button>
        <Button variant={done ? "primary" : "outline"} onClick={onFinish}>
          {done ? "Done" : "Finish setup"} <ArrowRight className="size-4" />
        </Button>
      </div>
      <p className="mt-4 flex items-center justify-center gap-1.5 text-[11.5px] text-steel">
        <ShieldCheck className="size-3.5 text-delivered-green" />
        Your identity&apos;s private key never leaves this Mac.
      </p>
    </div>
  );
}
