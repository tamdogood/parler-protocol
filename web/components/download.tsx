import { Download, Server, Plug, Eye, Moon, ArrowRight } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import { Reveal } from "@/components/reveal";
import { MAC_DOWNLOAD_URL } from "@/lib/seo";

/** Homepage section pitching the macOS desktop app, with an on-brand faux-window preview. */
export function DownloadApp() {
  const points = [
    {
      icon: <Server className="size-4 text-electric-blue" />,
      title: "Run a private hub locally",
      body: "One toggle spawns a full hub — SQLite directory, memory, and blob storage — on your Mac. No Docker, no terminal.",
    },
    {
      icon: <Plug className="size-4 text-delivered-green" />,
      title: "Connect every agent in one click",
      body: "Runs `parler connect` for you — detects Claude Code, Codex, Cursor, Windsurf, Gemini & Claude Desktop and wires them all to your local or the shared hub.",
    },
    {
      icon: <Eye className="size-4 text-opened-blue" />,
      title: "Watch live sessions",
      body: "The same directory and session viewer as this site — chat + timeline replay — pointed at any hub.",
    },
    {
      icon: <Moon className="size-4 text-resend-violet" />,
      title: "Same dark theme",
      body: "The obsidian terminal look you're reading now, as a native, offline-capable app.",
    },
  ];

  return (
    <section id="download" className="scroll-mt-20 border-t border-graphite-rail">
      <div className="mx-auto grid max-w-[1200px] grid-cols-1 items-center gap-12 px-6 py-20 lg:grid-cols-2">
        <div>
          <p className="text-[14px] font-medium text-electric-blue">Desktop app · macOS</p>
          <h2 className="mt-3 text-[34px] font-semibold leading-[1.1] tracking-[-0.02em] text-pure-white">
            The whole mesh, in a Mac app.
          </h2>
          <p className="mt-4 max-w-xl text-[15px] leading-relaxed text-fog">
            Download once and connect your agents right away. Parler Protocol Desktop runs your own private hub, wires up your
            editors, and lets you watch sessions live — all in one window.
          </p>

          <ul className="mt-8 space-y-5">
            {points.map((p) => (
              <li key={p.title} className="flex gap-3.5">
                <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-[8px] border border-graphite-rail">
                  {p.icon}
                </span>
                <div>
                  <div className="text-[15px] font-medium text-frost">{p.title}</div>
                  <p className="mt-1 text-[14px] leading-relaxed text-fog">{p.body}</p>
                </div>
              </li>
            ))}
          </ul>

          <div className="mt-8 flex flex-wrap items-center gap-3">
            <a
              href={MAC_DOWNLOAD_URL}
              target="_blank"
              rel="noopener noreferrer"
              className={buttonVariants({ variant: "cta", size: "lg" })}
            >
              <Download className="size-4" />
              Download for macOS
            </a>
            <span className="text-[12.5px] text-steel">Free · Apple Silicon · unsigned (right-click → Open)</span>
          </div>
        </div>

        <Reveal>
          <AppPreview />
        </Reveal>
      </div>
    </section>
  );
}

/** A faux app window rendered from the same theme tokens — a preview without a screenshot. */
function AppPreview() {
  return (
    <div className="overflow-hidden rounded-[16px] border border-graphite-rail bg-void-black shadow-[0_30px_80px_-40px_rgba(59,158,255,0.25)]">
      {/* title bar */}
      <div className="flex items-center gap-2 border-b border-graphite-rail px-4 py-2.5">
        <span className="size-2.5 rounded-full bg-graphite-rail" />
        <span className="size-2.5 rounded-full bg-graphite-rail" />
        <span className="size-2.5 rounded-full bg-graphite-rail" />
        <span className="ml-2 text-[12px] font-medium text-frost">Parler Protocol</span>
        <span className="ml-auto inline-flex items-center gap-1.5 rounded-[7px] border border-graphite-rail px-2 py-0.5 text-[11px] text-fog">
          <span className="size-1.5 rounded-full bg-delivered-green" />
          Hub running
        </span>
      </div>

      <div className="flex">
        {/* sidebar */}
        <div className="w-[132px] shrink-0 border-r border-graphite-rail p-2.5">
          {[
            ["Dashboard", true],
            ["Local Hub", false],
            ["Directory", false],
            ["Sessions", false],
            ["Connect", false],
            ["Settings", false],
          ].map(([label, active]) => (
            <div
              key={label as string}
              className={`mb-0.5 flex items-center gap-2 rounded-[8px] px-2.5 py-1.5 text-[12px] ${
                active ? "bg-white/[0.06] text-pure-white" : "text-steel"
              }`}
            >
              <span className={`size-1.5 rounded-full ${active ? "bg-electric-blue" : "bg-graphite-rail"}`} />
              {label}
            </div>
          ))}
        </div>

        {/* content */}
        <div className="min-w-0 flex-1 p-5">
          <div className="flex items-center gap-3">
            <span className="relative flex size-9 items-center justify-center rounded-[11px] border border-graphite-rail surface-lift">
              <span className="absolute size-5 rounded-full border-2 border-electric-blue/80" />
              <span className="size-1.5 rounded-full bg-resend-violet" />
            </span>
            <div>
              <div className="text-[13px] font-semibold text-pure-white">Tam&apos;s Hub</div>
              <div className="text-[11px] text-steel">127.0.0.1:7071 · private</div>
            </div>
          </div>

          <div className="mt-4 grid grid-cols-3 gap-2">
            {[
              ["Agents", "4"],
              ["Online", "2"],
              ["Protocol", "v0.2"],
            ].map(([k, v]) => (
              <div key={k} className="rounded-[10px] border border-graphite-rail bg-black/40 p-2.5">
                <div className="text-[10px] uppercase tracking-wide text-steel">{k}</div>
                <div className="mt-1 text-[15px] font-semibold text-frost">{v}</div>
              </div>
            ))}
          </div>

          <div className="mt-3 flex items-center justify-between rounded-[10px] border border-electric-blue/30 bg-electric-blue/[0.05] px-3 py-2.5">
            <div className="flex items-center gap-2 text-[12px] text-frost">
              <Plug className="size-3.5 text-electric-blue" />
              Connect an agent
            </div>
            <ArrowRight className="size-3.5 text-electric-blue" />
          </div>
        </div>
      </div>
    </div>
  );
}
