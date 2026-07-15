import type { Settings } from "../shared/types";

/** The default local-hub port. 7071 (not 7070) so it never collides with a dev/seed hub. */
export const DEFAULT_HUB_PORT = 7071;

/** Defaults used only when the app has no saved settings yet. */
export function defaultSettings(who: string): Settings {
  return {
    // A fresh install joins the shared hub. Local hosting remains an explicit, privacy-first choice.
    autoStartHub: false,
    hubPublic: false,
    hubReachable: false,
    hubName: `${who}'s Hub`,
    hubPort: DEFAULT_HUB_PORT,
    connectTarget: "public",
    autoConnectAgents: true,
    startAtLogin: false,
    onboarded: false,
  };
}
