"use client";

import { useEffect } from "react";

/**
 * Back-compat shim. The session viewer now lives in the Hub's Sessions tab (`/hub#sessions`), so this
 * old route just forwards there — preserving any `#k=<watch-token>` so a previously-minted/shared
 * `/session#k=…` deep link still opens the viewer pre-connected. Runs client-side because the URL hash
 * (where the token lives) is never sent to the server.
 */
export default function SessionRedirect() {
  useEffect(() => {
    const m = window.location.hash.match(/[#&]k=([^&]+)/);
    window.location.replace(m ? `/hub#sessions&k=${m[1]}` : "/hub#sessions");
  }, []);

  return (
    <main className="flex min-h-screen items-center justify-center px-6">
      <p className="text-[14px] text-fog">
        Redirecting to the session hub…{" "}
        <a href="/hub#sessions" className="text-electric-blue hover:underline">
          continue
        </a>
      </p>
    </main>
  );
}
