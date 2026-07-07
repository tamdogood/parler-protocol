import { Logo } from "@/components/logo";

export function Footer() {
  return (
    <footer className="border-t border-graphite-rail">
      <div className="mx-auto flex max-w-[1200px] flex-col items-center justify-between gap-4 px-6 py-10 sm:flex-row">
        <div className="flex items-center gap-2 text-[13px] text-steel">
          <Logo className="size-4" />
          <span>Parler Protocol — an open protocol for agent coordination.</span>
        </div>
        <div className="flex items-center gap-6 text-[13px]">
          <a
            href="https://github.com/tamdogood/parler-ai"
            target="_blank"
            rel="noreferrer"
            className="text-fog transition-colors hover:text-frost"
          >
            GitHub
          </a>
          <a href="/docs" className="text-fog transition-colors hover:text-frost">
            Docs
          </a>
          <a href="/blog" className="text-fog transition-colors hover:text-frost">
            Blog
          </a>
          <a href="/blog/rss.xml" className="text-fog transition-colors hover:text-frost">
            RSS
          </a>
          <a href="/#faq" className="text-fog transition-colors hover:text-frost">
            FAQ
          </a>
          <a href="/#security" className="text-fog transition-colors hover:text-frost">
            Security model
          </a>
        </div>
      </div>
    </footer>
  );
}
