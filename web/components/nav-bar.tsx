import { Github } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import { Logo } from "@/components/logo";

/** Site-wide sticky top bar: 59px, frosted black, single hairline bottom border. */
export function NavBar() {
  return (
    <header className="sticky top-0 z-40 h-[59px] w-full border-b border-graphite-rail bg-black/70 backdrop-blur-[25px]">
      <div className="mx-auto flex h-full max-w-[1200px] items-center justify-between px-6">
        <a href="/" className="flex items-center gap-2.5">
          <Logo className="size-6" />
          <span className="text-[16px] font-medium tracking-tight text-pure-white">Parler Protocol</span>
          <span className="ml-1 hidden rounded-[6px] border border-graphite-rail px-1.5 py-0.5 font-mono text-[11px] text-fog sm:inline">
            beta
          </span>
        </a>

        {/* Deliberately slim — the depth (security model, FAQ, viewer) is in the footer. */}
        <nav className="hidden items-center gap-7 md:flex">
          <a href="/hub" className="text-[14px] text-frost/70 transition-colors hover:text-frost">
            Hub
          </a>
          <a href="/docs" className="text-[14px] text-frost/70 transition-colors hover:text-frost">
            Docs
          </a>
          <a href="/blog" className="text-[14px] text-frost/70 transition-colors hover:text-frost">
            Blog
          </a>
          {/* Download link temporarily hidden while the desktop app is stabilized. */}
        </nav>

        <div className="flex items-center gap-3">
          <a
            href="https://github.com/tamdogood/parler-ai"
            target="_blank"
            rel="noreferrer"
            className="hidden items-center gap-1.5 text-[14px] text-frost/90 transition-colors hover:text-pure-white sm:inline-flex"
          >
            <Github className="size-4" />
            GitHub
          </a>
          <a href="/docs/quickstart" className={buttonVariants({ variant: "primary", size: "sm" })}>
            Get started
          </a>
        </div>
      </div>
    </header>
  );
}
