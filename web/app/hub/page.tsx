"use client";

import { useEffect, useState } from "react";
import { MessagesSquare, Users } from "lucide-react";
import { NavBar } from "@/components/nav-bar";
import { Footer } from "@/components/footer";
import { AgentsConsole } from "@/components/agents-console";
import { SessionHub } from "@/components/session-hub";
import { cn } from "@/lib/utils";

type Tab = "agents" | "sessions";

/**
 * The standalone Hub — the agent control center. Two tabs: Agents (the full-screen directory console)
 * and Sessions (the session hub: explainer + live watch viewer). The tab is synced to the URL hash so
 * `/hub` lands on Agents, `/hub#sessions` on Sessions, and a minted `/hub#sessions&k=<token>` link
 * opens the viewer pre-connected (the redirect from the old `/session` route carries the token here).
 */
export default function HubPage() {
  const [tab, setTab] = useState<Tab>("agents");

  // Read-only on mount: never overwrite the hash here, so a `&k=<token>` survives for the viewer.
  useEffect(() => {
    const h = typeof window !== "undefined" ? window.location.hash : "";
    if (/sessions|k=/.test(h)) setTab("sessions");
  }, []);

  const select = (t: Tab) => {
    setTab(t);
    if (typeof window !== "undefined") {
      // replaceState (not location.hash) so switching tabs never scroll-jumps to an anchor.
      window.history.replaceState(null, "", t === "sessions" ? "#sessions" : "#agents");
    }
  };

  return (
    <main className="min-h-screen">
      <NavBar />

      {/* Tab bar — sticks just below the 59px nav. */}
      <div className="sticky top-[59px] z-30 border-b border-graphite-rail bg-black/70 backdrop-blur-[25px]">
        <div className="mx-auto flex max-w-[1600px] items-center gap-1 px-4 sm:px-6">
          <TabButton active={tab === "agents"} onClick={() => select("agents")} icon={<Users className="size-4" />}>
            Agents
          </TabButton>
          <TabButton active={tab === "sessions"} onClick={() => select("sessions")} icon={<MessagesSquare className="size-4" />}>
            Sessions
          </TabButton>
        </div>
      </div>

      {tab === "agents" ? <AgentsConsole /> : <SessionHub />}

      <Footer />
    </main>
  );
}

function TabButton({
  active,
  onClick,
  icon,
  children,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "relative inline-flex items-center gap-2 px-3 py-3 text-[14px] font-medium transition-colors",
        active ? "text-pure-white" : "text-fog hover:text-frost",
      )}
    >
      {icon}
      {children}
      <span
        className={cn(
          "absolute inset-x-2 -bottom-px h-0.5 rounded-full transition-colors",
          active ? "bg-electric-blue" : "bg-transparent",
        )}
      />
    </button>
  );
}
