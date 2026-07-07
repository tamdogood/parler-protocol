"use client";

import { usePathname } from "next/navigation";
import { DOC_GROUPS, docsInGroup } from "@/lib/docs";

/**
 * Docs sidebar: grouped list of every page, with the current page highlighted.
 * Sticky on desktop; collapses to a horizontal scroller above the content on mobile.
 */
export function DocsSidebar() {
  const pathname = usePathname();

  return (
    <nav
      aria-label="Documentation"
      className="shrink-0 border-graphite-rail md:sticky md:top-[59px] md:h-[calc(100vh-59px)] md:w-[236px] md:overflow-y-auto md:border-r md:py-12 md:pr-6"
    >
      <a
        href="/docs"
        className={`hidden text-[13px] font-medium md:block ${
          pathname === "/docs" ? "text-electric-blue" : "text-frost/70 hover:text-frost"
        }`}
      >
        Overview
      </a>

      <div className="flex gap-6 overflow-x-auto py-4 md:mt-6 md:flex-col md:gap-7 md:overflow-visible md:py-0">
        {DOC_GROUPS.map((group) => (
          <div key={group} className="shrink-0">
            <div className="mb-2 font-mono text-[11px] uppercase tracking-[0.08em] text-steel">
              {group}
            </div>
            <ul className="flex gap-4 md:flex-col md:gap-1.5">
              {docsInGroup(group).map((doc) => {
                const active = pathname === `/docs/${doc.slug}`;
                return (
                  <li key={doc.slug}>
                    <a
                      href={`/docs/${doc.slug}`}
                      className={`block whitespace-nowrap text-[14px] transition-colors md:whitespace-normal ${
                        active
                          ? "font-medium text-electric-blue"
                          : "text-frost/70 hover:text-frost"
                      }`}
                    >
                      {doc.navLabel ?? doc.title}
                    </a>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>
    </nav>
  );
}
