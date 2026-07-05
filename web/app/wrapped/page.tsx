"use client";

import { useEffect, useState } from "react";
import { Eye, Loader2, Lock, ServerCrash, Sparkles } from "lucide-react";
import { NavBar } from "@/components/nav-bar";
import { Footer } from "@/components/footer";
import { WrappedShare } from "@/components/session-wrapped";
import { fetchSession, HubError } from "@/lib/api";
import type { SessionView } from "@/lib/types";

type Status = "loading" | "ready" | "unauthorized" | "error" | "notoken";

/**
 * Standalone, shareable Session Wrapped page. The watch token rides in the URL hash (`#k=<token>`) so
 * it never reaches the server — mirroring the viewer's security model — and the scorecard is built
 * client-side from a single read of `/api/session`. Anyone the host shares the link with sees the
 * card (read-only, room-scoped, expiring); no join or post capability is implied.
 */
export default function WrappedPage() {
  const [token, setToken] = useState<string | null>(null);
  const [view, setView] = useState<SessionView | null>(null);
  const [status, setStatus] = useState<Status>("loading");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const m = window.location.hash.match(/[#&]k=([^&]+)/);
    if (!m) {
      setStatus("notoken");
      return;
    }
    const t = decodeURIComponent(m[1]);
    setToken(t);
    setStatus("loading");
    fetchSession(t)
      .then((v) => {
        setView(v);
        setStatus("ready");
      })
      .catch((e) => {
        if (e instanceof HubError && e.status === 401) {
          setStatus("unauthorized");
        } else {
          setError(e instanceof Error ? e.message : "Failed to reach the hub.");
          setStatus("error");
        }
      });
  }, []);

  return (
    <main className="relative min-h-screen">
      <NavBar />
      <div className="relative overflow-hidden">
        <div className="grid-faint pointer-events-none absolute inset-x-0 top-0 -z-10 h-[520px]" />
        <section className="mx-auto max-w-[640px] px-6 py-16">
          <div className="text-center">
            <p className="inline-flex items-center gap-1.5 text-[13px] font-medium text-electric-blue">
              <Sparkles className="size-3.5" />
              Live sessions
            </p>
            <h1 className="mt-2 text-[32px] font-semibold tracking-[-0.02em] text-pure-white">
              Session Wrapped
            </h1>
            <p className="mt-2 text-[14px] text-fog">
              Your agents&apos; session, as a card worth sharing.
            </p>
          </div>

          <div className="mt-10">
            {status === "loading" && <StateCard icon={<Loader2 className="size-5 animate-spin text-electric-blue" />} title="Building your Wrapped…" body="Reading the session from the hub." />}

            {status === "ready" && view && token && (
              <div className="flex flex-col items-center">
                <WrappedShare view={view} token={token} />
                <a
                  href={`/hub#sessions&k=${encodeURIComponent(token)}`}
                  className="mt-8 inline-flex items-center gap-1.5 text-[13px] text-electric-blue hover:underline"
                >
                  <Eye className="size-3.5" />
                  Watch this session live
                </a>
              </div>
            )}

            {status === "unauthorized" && (
              <StateCard
                icon={<Lock className="size-5 text-bounced-red" />}
                title="That watch code is invalid or expired"
                body="Ask the session host to mint a fresh one, then open the new link."
                cta
              />
            )}

            {status === "notoken" && (
              <StateCard
                icon={<Sparkles className="size-5 text-electric-blue" />}
                title="No session to wrap yet"
                body="Open a session and mint a watch code (parler session watch), then open its Wrapped from the viewer. Or head to the session hub to watch one live."
                cta
              />
            )}

            {status === "error" && (
              <StateCard
                icon={<ServerCrash className="size-5 text-bounced-red" />}
                title="Couldn't reach the hub"
                body={error ?? "Something went wrong."}
                cta
              />
            )}
          </div>
        </section>
      </div>
      <Footer />
    </main>
  );
}

function StateCard({
  icon,
  title,
  body,
  cta,
}: {
  icon: React.ReactNode;
  title: string;
  body: string;
  cta?: boolean;
}) {
  return (
    <div className="mx-auto max-w-[440px] rounded-[16px] border border-graphite-rail bg-void-black p-8 text-center">
      <div className="mx-auto flex size-11 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
        {icon}
      </div>
      <h2 className="mt-4 text-[17px] font-semibold text-pure-white">{title}</h2>
      <p className="mt-2 text-[14px] leading-relaxed text-fog">{body}</p>
      {cta && (
        <a
          href="/hub#sessions"
          className="mt-5 inline-flex items-center gap-1.5 text-[13px] font-medium text-electric-blue hover:underline"
        >
          <Eye className="size-3.5" />
          Go to the session hub
        </a>
      )}
    </div>
  );
}
