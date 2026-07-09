import { Logo } from "@/components/logo";
import { GITHUB_URL } from "@/lib/seo";

// The footer is the "everything else" layer: when the landing page slimmed down to hero + video,
// the secondary destinations (viewer, security model, FAQ, license) moved here so every page can
// still reach them in one click.
const COLUMNS: { heading: string; links: { label: string; href: string; external?: boolean }[] }[] = [
  {
    heading: "Product",
    links: [
      { label: "Hub directory", href: "/hub" },
      { label: "Session viewer", href: "/hub#sessions" },
      { label: "Docs", href: "/docs" },
      { label: "Quickstart", href: "/docs/quickstart" },
    ],
  },
  {
    heading: "Resources",
    links: [
      { label: "Blog", href: "/blog" },
      { label: "FAQ", href: "/faq" },
      { label: "Security model", href: "/docs/security" },
      { label: "RSS", href: "/blog/rss.xml" },
    ],
  },
  {
    heading: "Open source",
    links: [
      { label: "GitHub", href: GITHUB_URL, external: true },
      { label: "Report an issue", href: `${GITHUB_URL}/issues`, external: true },
      { label: "Apache-2.0 license", href: `${GITHUB_URL}/blob/main/LICENSE`, external: true },
    ],
  },
];

export function Footer() {
  return (
    <footer className="border-t border-graphite-rail">
      <div className="mx-auto max-w-[1200px] px-6 py-14">
        <div className="flex flex-col justify-between gap-10 md:flex-row">
          <div className="max-w-xs">
            <div className="flex items-center gap-2.5">
              <Logo className="size-5" />
              <span className="text-[15px] font-medium text-pure-white">Parler Protocol</span>
            </div>
            <p className="mt-3 text-[13px] leading-relaxed text-steel">
              The chat protocol for AI agents — hand off a live session with one key instead of
              copy-pasting the transcript.
            </p>
          </div>

          <div className="grid grid-cols-2 gap-10 sm:grid-cols-3">
            {COLUMNS.map((col) => (
              <div key={col.heading}>
                <h3 className="text-[13px] font-medium text-frost">{col.heading}</h3>
                <ul className="mt-4 space-y-2.5">
                  {col.links.map((l) => (
                    <li key={l.label}>
                      <a
                        href={l.href}
                        {...(l.external ? { target: "_blank", rel: "noreferrer" } : {})}
                        className="text-[13px] text-fog transition-colors hover:text-frost"
                      >
                        {l.label}
                      </a>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </div>

        <p className="mt-12 border-t border-graphite-rail pt-6 text-[12.5px] text-steel">
          Parler Protocol — an open protocol for agent coordination.
        </p>
      </div>
    </footer>
  );
}
