"use client";

import { useI18n } from "@/i18n/provider";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { DOCS_NAV, pathKey } from "@/lib/docs-nav";

export default function DocsSidebar() {
  const { t } = useI18n();
  const pathname = usePathname();
  const s = t.docs.sidebar;
  const current = pathKey(pathname);

  const groups = [
    {
      title: s.gettingStarted,
      links: DOCS_NAV.filter((n) => n.group === "gettingStarted"),
    },
    {
      title: s.coreConcepts,
      links: DOCS_NAV.filter((n) => n.group === "coreConcepts"),
    },
    {
      title: s.guides,
      links: DOCS_NAV.filter((n) => n.group === "guides"),
    },
  ];

  return (
    <>
      {/* Mobile: horizontal chips (Pages has no JS router bar) */}
      <nav
        className="lg:hidden -mx-6 px-6 mb-8 overflow-x-auto"
        aria-label="Docs"
      >
        <ul className="flex gap-2 min-w-max pb-1">
          {DOCS_NAV.map((link) => {
            const active = current === pathKey(link.href);
            return (
              <li key={link.href}>
                <Link
                  href={link.href}
                  className={`block px-3 py-1.5 rounded-md text-xs whitespace-nowrap border transition-colors ${
                    active
                      ? "border-accent/40 bg-accent/10 text-accent-bright font-medium"
                      : "border-border text-text-secondary hover:text-text-primary hover:bg-surface-hover"
                  }`}
                >
                  {s[link.labelKey]}
                </Link>
              </li>
            );
          })}
        </ul>
      </nav>

      {/* Desktop sticky sidebar */}
      <aside className="hidden lg:block w-64 shrink-0">
        <nav className="sticky top-24 space-y-8" aria-label="Docs">
          {groups.map((group) => (
            <div key={group.title}>
              <h3 className="px-3 mb-2 text-xs font-medium text-text-tertiary">
                {group.title}
              </h3>
              <ul className="space-y-0.5">
                {group.links.map((link) => {
                  const active = current === pathKey(link.href);
                  return (
                    <li key={link.href}>
                      <Link
                        href={link.href}
                        className={`block px-3 py-1.5 rounded-md text-sm transition-colors ${
                          active
                            ? "bg-accent/10 text-accent-bright font-medium"
                            : "text-text-secondary hover:text-text-primary hover:bg-surface-hover"
                        }`}
                      >
                        {s[link.labelKey]}
                      </Link>
                    </li>
                  );
                })}
              </ul>
            </div>
          ))}
        </nav>
      </aside>
    </>
  );
}
