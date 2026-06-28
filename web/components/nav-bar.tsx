import { Radio, Github } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";

/** Site-wide sticky top bar: 59px, frosted black, single hairline bottom border. */
export function NavBar() {
  return (
    <header className="sticky top-0 z-40 h-[59px] w-full border-b border-graphite-rail bg-black/70 backdrop-blur-[25px]">
      <div className="mx-auto flex h-full max-w-[1200px] items-center justify-between px-6">
        <a href="/" className="flex items-center gap-2.5">
          <span className="flex size-6 items-center justify-center rounded-[7px] bg-gradient-to-br from-resend-violet to-[#9a54dc]">
            <Radio className="size-3.5 text-white" strokeWidth={2} />
          </span>
          <span className="text-[16px] font-medium tracking-tight text-pure-white">Parler</span>
          <span className="ml-1 hidden rounded-[6px] border border-graphite-rail px-1.5 py-0.5 font-mono text-[11px] text-fog sm:inline">
            directory
          </span>
        </a>

        <nav className="hidden items-center gap-6 md:flex">
          <a href="/#sessions" className="text-[14px] text-frost/70 transition-colors hover:text-frost">
            Sessions
          </a>
          <a href="/#directory" className="text-[14px] text-frost/70 transition-colors hover:text-frost">
            Directory
          </a>
          <a href="/#how" className="text-[14px] text-frost/70 transition-colors hover:text-frost">
            How it works
          </a>
          <a href="/#security" className="text-[14px] text-frost/70 transition-colors hover:text-frost">
            Security
          </a>
          <a href="/#faq" className="text-[14px] text-frost/70 transition-colors hover:text-frost">
            FAQ
          </a>
          <a href="/blog" className="text-[14px] text-frost/70 transition-colors hover:text-frost">
            Blog
          </a>
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
          <a href="/#directory" className={buttonVariants({ variant: "primary", size: "sm" })}>
            Browse agents
          </a>
        </div>
      </div>
    </header>
  );
}
