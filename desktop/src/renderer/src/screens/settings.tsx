import { Settings as Cog, FolderOpen, Github, Globe2, RotateCcw, HardDrive, Cloud } from "lucide-react";
import type { HubTarget, Settings } from "@shared/types";
import { PUBLIC_HUB } from "@shared/types";
import { parler } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import type { Screen } from "@/components/sidebar";

const REPO = "https://github.com/tamdogood/parler-ai";
const SITE = PUBLIC_HUB;

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
    <div className="mx-auto max-w-[720px] px-8 py-8">
      <div className="flex items-center gap-3">
        <span className="flex size-11 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
          <Cog className="size-5 text-electric-blue" />
        </span>
        <div>
          <h1 className="text-[22px] font-semibold tracking-tight text-pure-white">Settings</h1>
          <p className="text-[13px] text-fog">Preferences for the app and your local hub.</p>
        </div>
      </div>

      <Group title="Startup">
        <Row title="Start the local hub on launch" subtitle="Boot your private hub automatically when Parler opens.">
          <Switch on={settings.autoStartHub} onChange={(v) => onUpdate({ autoStartHub: v })} />
        </Row>
      </Group>

      <Group title="Defaults">
        <Row title="Default connect target" subtitle="Which hub the Connect tab targets first.">
          <Segmented
            value={settings.connectTarget}
            onChange={(t) => onUpdate({ connectTarget: t })}
            options={[
              { value: "local", label: "Local", icon: <HardDrive className="size-3.5" /> },
              { value: "public", label: "Public", icon: <Cloud className="size-3.5" /> },
            ]}
          />
        </Row>
      </Group>

      <Group title="Local hub">
        <Row title={settings.hubName} subtitle={`Port ${settings.hubPort} · ${settings.hubPublic ? "public" : "private"} · SQLite + blobs on this Mac`}>
          <Button variant="outline" size="sm" onClick={() => onNavigate("hub")}>
            Manage
          </Button>
        </Row>
        <Row title="Data folder" subtitle="Where the hub's database, blobs, and identity live.">
          <Button variant="outline" size="sm" onClick={() => parler.hub.openDataFolder()}>
            <FolderOpen className="size-3.5" /> Open
          </Button>
        </Row>
      </Group>

      <Group title="About">
        <Row title="Parler Desktop" subtitle={`Version ${version} · unsigned build`}>
          <div className="flex gap-2">
            <Button variant="subtle" size="sm" onClick={() => parler.shell.openExternal(REPO)}>
              <Github className="size-3.5" /> GitHub
            </Button>
            <Button variant="subtle" size="sm" onClick={() => parler.shell.openExternal(SITE)}>
              <Globe2 className="size-3.5" /> Website
            </Button>
          </div>
        </Row>
        <Row title="Replay onboarding" subtitle="See the first-run setup flow again.">
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
      <div className="divide-y divide-graphite-rail overflow-hidden rounded-[14px] border border-graphite-rail bg-void-black">
        {children}
      </div>
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
      <span
        className={cn(
          "absolute top-1/2 size-4 -translate-y-1/2 rounded-full bg-frost transition-all",
          on ? "left-[22px]" : "left-1",
        )}
      />
    </button>
  );
}

function Segmented({
  value,
  onChange,
  options,
}: {
  value: HubTarget;
  onChange: (v: HubTarget) => void;
  options: { value: HubTarget; label: string; icon: React.ReactNode }[];
}) {
  return (
    <div className="no-drag flex rounded-[9px] border border-graphite-rail p-0.5">
      {options.map((o) => (
        <button
          key={o.value}
          onClick={() => onChange(o.value)}
          className={cn(
            "flex items-center gap-1.5 rounded-[7px] px-2.5 py-1 text-[12.5px] font-medium transition-colors",
            value === o.value ? "bg-electric-blue/15 text-pure-white" : "text-steel hover:text-frost",
          )}
        >
          {o.icon}
          {o.label}
        </button>
      ))}
    </div>
  );
}
