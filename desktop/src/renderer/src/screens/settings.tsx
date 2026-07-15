import { Settings as Cog, FolderOpen, Github, Globe2, RotateCcw, Server } from "lucide-react";
import type { Settings } from "@shared/types";
import { PUBLIC_HUB } from "@shared/types";
import { parler } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import type { Screen } from "@/components/sidebar";

const REPO = "https://github.com/tamdogood/parler-protocol";

export function SettingsScreen({
  settings,
  version,
  onUpdate,
  onNavigate,
  onReplayOnboarding,
}: {
  settings: Settings | null;
  version: string;
  onUpdate: (patch: Partial<Settings>) => Promise<void>;
  onNavigate: (s: Screen) => void;
  onReplayOnboarding: () => void;
}) {
  if (!settings) return null;
  return (
    <div className="mx-auto max-w-[680px] px-8 py-8">
      <div className="flex items-center gap-3">
        <span className="flex size-11 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
          <Cog className="size-5 text-electric-blue" />
        </span>
        <div>
          <h1 className="text-[22px] font-semibold tracking-tight text-pure-white">Settings</h1>
          <p className="text-[13px] text-fog">Preferences for the app and your local hub.</p>
        </div>
      </div>

      <Group title="Local hub">
        <Row title="Start Parler Protocol at login" subtitle="Launch in the background at login so the hub is up before your agents dial in.">
          <Switch on={settings.startAtLogin} onChange={(v) => onUpdate({ startAtLogin: v })} />
        </Row>
        <Row title="Start hub on launch" subtitle="Boot your private hub automatically when Parler Protocol opens.">
          <Switch on={settings.autoStartHub} onChange={(v) => onUpdate({ autoStartHub: v })} />
        </Row>
        <Row
          title="Keep agents connected automatically"
          subtitle="Detect new agent apps and wire them to your selected hub in the background."
        >
          <Switch on={settings.autoConnectAgents} onChange={(v) => onUpdate({ autoConnectAgents: v })} />
        </Row>
        <Row
          title={settings.hubName}
          subtitle={`Port ${settings.hubPort} · ${settings.hubPublic ? "public" : settings.hubReachable ? "team (LAN)" : "private"}`}
        >
          <Button variant="outline" size="sm" onClick={() => onNavigate("hub")}>
            <Server className="size-3.5" /> Manage
          </Button>
        </Row>
        <Row title="Data folder" subtitle="Where the hub's database, blobs, and identity live.">
          <Button variant="outline" size="sm" onClick={() => parler.hub.openDataFolder()}>
            <FolderOpen className="size-3.5" /> Open
          </Button>
        </Row>
      </Group>

      <Group title="About">
        <Row title="Parler Protocol Desktop" subtitle={`Version ${version} · unsigned build`}>
          <div className="flex gap-2">
            <Button variant="subtle" size="sm" onClick={() => parler.shell.openExternal(REPO)}>
              <Github className="size-3.5" /> GitHub
            </Button>
            <Button variant="subtle" size="sm" onClick={() => parler.shell.openExternal(PUBLIC_HUB)}>
              <Globe2 className="size-3.5" /> Website
            </Button>
          </div>
        </Row>
        <Row title="Replay onboarding" subtitle="See the first-run setup again.">
          <Button variant="outline" size="sm" onClick={onReplayOnboarding}>
            <RotateCcw className="size-3.5" /> Replay
          </Button>
        </Row>
      </Group>
    </div>
  );
}

function Group({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="mt-7">
      <p className="mb-2 text-[11px] uppercase tracking-wide text-steel">{title}</p>
      <div className="divide-y divide-graphite-rail overflow-hidden rounded-[14px] border border-graphite-rail bg-void-black">{children}</div>
    </div>
  );
}

function Row({ title, subtitle, children }: { title: string; subtitle: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-3 p-4">
      <div className="min-w-0">
        <p className="text-[14px] font-medium text-frost">{title}</p>
        <p className="mt-0.5 text-[12.5px] text-steel">{subtitle}</p>
      </div>
      {children}
    </div>
  );
}

function Switch({ on, onChange }: { on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      onClick={() => onChange(!on)}
      className={cn(
        "no-drag relative h-6 w-11 shrink-0 rounded-full border transition-colors",
        on ? "border-electric-blue bg-electric-blue/25" : "border-graphite-rail bg-black",
      )}
    >
      <span className={cn("absolute top-1/2 size-4 -translate-y-1/2 rounded-full bg-frost transition-all", on ? "left-[22px]" : "left-1")} />
    </button>
  );
}
