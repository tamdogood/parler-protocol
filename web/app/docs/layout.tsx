import { NavBar } from "@/components/nav-bar";
import { Footer } from "@/components/footer";
import { DocsSidebar } from "@/components/docs/sidebar";

/**
 * Shell for every docs route: sticky top bar, a left sidebar with the full page
 * list, and the page body. Individual pages render only their content — the chrome
 * lives here so it can't drift between pages.
 */
export default function DocsLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen">
      <NavBar />
      <div className="mx-auto flex max-w-[1200px] flex-col px-6 md:flex-row md:gap-10">
        <DocsSidebar />
        <main className="min-w-0 flex-1 py-10 md:py-14">{children}</main>
      </div>
      <Footer />
    </div>
  );
}
