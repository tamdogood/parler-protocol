import { useEffect, useState } from "react";
import { PUBLIC_HUB } from "@shared/types";
import { parler } from "@/lib/ipc";
import { useHubStatus, useHubUrl, useSettings } from "@/lib/hooks";
import { TitleBar } from "@/components/titlebar";
import { Sidebar, type Screen } from "@/components/sidebar";
import { AgentsScreen } from "@/screens/agents-screen";
import { ConnectScreen } from "@/screens/connect";
import { SessionsScreen } from "@/screens/sessions";
import { SettingsScreen } from "@/screens/settings";
import { LocalHubScreen } from "@/screens/local-hub";
import { Onboarding } from "@/screens/onboarding";

export function App() {
  const status = useHubStatus();
  const { settings, update } = useSettings();
  const localUrl = useHubUrl("local", status);
  const publicUrl = useHubUrl("public", status) ?? PUBLIC_HUB;

  const [screen, setScreen] = useState<Screen>("agents");
  const [version, setVersion] = useState("0.0.0");
  const [replayOnboarding, setReplayOnboarding] = useState(false);

  useEffect(() => {
    parler.app.version().then(setVersion);
  }, []);

  if (!settings) return <div className="h-screen w-screen bg-black" />;

  const showOnboarding = replayOnboarding || !settings.onboarded;

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-black text-frost">
      <TitleBar status={status} onOpenHub={() => setScreen("hub")} />
      <div className="flex min-h-0 flex-1">
        <Sidebar active={screen} onNavigate={setScreen} status={status} />
        <main className="min-w-0 flex-1 overflow-y-auto">
          {screen === "agents" && <AgentsScreen localUrl={localUrl} status={status} onConnect={() => setScreen("connect")} />}
          {screen === "connect" && (
            <ConnectScreen status={status} onStartHub={() => parler.hub.start()} onGoToAgents={() => setScreen("agents")} />
          )}
          {screen === "sessions" && <SessionsScreen localUrl={localUrl} publicUrl={publicUrl} status={status} />}
          {screen === "settings" && (
            <SettingsScreen
              settings={settings}
              version={version}
              onUpdate={update}
              onNavigate={setScreen}
              onReplayOnboarding={() => setReplayOnboarding(true)}
            />
          )}
          {screen === "hub" && <LocalHubScreen status={status} settings={settings} onUpdateSettings={update} onBack={() => setScreen("settings")} />}
        </main>
      </div>

      {showOnboarding && (
        <Onboarding
          status={status}
          onFinish={async () => {
            await update({ onboarded: true });
            setReplayOnboarding(false);
            setScreen("agents");
          }}
        />
      )}
    </div>
  );
}
