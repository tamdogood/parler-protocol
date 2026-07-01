import { useEffect, useRef, useState } from "react";
import type { HubTarget } from "@shared/types";
import { PUBLIC_HUB } from "@shared/types";
import { parler } from "@/lib/ipc";
import { useHubStatus, useHubUrl, useSettings } from "@/lib/hooks";
import { TitleBar } from "@/components/titlebar";
import { Sidebar, type Screen } from "@/components/sidebar";
import { Dashboard } from "@/screens/dashboard";
import { LocalHubScreen } from "@/screens/local-hub";
import { DirectoryScreen } from "@/screens/directory-screen";
import { SessionsScreen } from "@/screens/sessions";
import { ConnectScreen } from "@/screens/connect";
import { SettingsScreen } from "@/screens/settings";
import { Onboarding } from "@/screens/onboarding";

export function App() {
  const status = useHubStatus();
  const { settings, update } = useSettings();
  const localUrl = useHubUrl("local", status);
  const publicUrl = useHubUrl("public", status) ?? PUBLIC_HUB;

  const [screen, setScreen] = useState<Screen>("dashboard");
  const [target, setTarget] = useState<HubTarget>("local");
  const [version, setVersion] = useState("0.0.0");
  const [replayOnboarding, setReplayOnboarding] = useState(false);
  const targetInit = useRef(false);

  useEffect(() => {
    parler.app.version().then(setVersion);
  }, []);

  // Seed the view target from the saved default once (user toggles win afterward).
  useEffect(() => {
    if (settings && !targetInit.current) {
      setTarget(settings.connectTarget);
      targetInit.current = true;
    }
  }, [settings]);

  if (!settings) {
    return <div className="h-screen w-screen bg-black" />;
  }

  const showOnboarding = replayOnboarding || !settings.onboarded;
  const viewBase = target === "local" ? localUrl : publicUrl;

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-black text-frost">
      <TitleBar status={status} target={target} onTarget={setTarget} onOpenHub={() => setScreen("hub")} />
      <div className="flex min-h-0 flex-1">
        <Sidebar active={screen} onNavigate={setScreen} status={status} version={version} />
        <main className="min-w-0 flex-1 overflow-y-auto">
          {screen === "dashboard" && <Dashboard status={status} localUrl={localUrl} onNavigate={setScreen} />}
          {screen === "hub" && <LocalHubScreen status={status} settings={settings} onUpdateSettings={update} />}
          {screen === "directory" && <DirectoryScreen base={viewBase} target={target} />}
          {screen === "sessions" && (
            <SessionsScreen base={viewBase ?? publicUrl} localUrl={localUrl} publicUrl={publicUrl} status={status} />
          )}
          {screen === "connect" && (
            <ConnectScreen
              status={status}
              defaultTarget={settings.connectTarget}
              onStartHub={() => parler.hub.start()}
              onGoToHub={() => setScreen("directory")}
            />
          )}
          {screen === "settings" && (
            <SettingsScreen
              settings={settings}
              version={version}
              onUpdate={update}
              onNavigate={setScreen}
              onReplayOnboarding={() => setReplayOnboarding(true)}
            />
          )}
        </main>
      </div>

      {showOnboarding && (
        <Onboarding
          status={status}
          onUpdate={update}
          onFinish={async () => {
            await update({ onboarded: true });
            setReplayOnboarding(false);
            setScreen("dashboard");
          }}
        />
      )}
    </div>
  );
}
