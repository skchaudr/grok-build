"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useI18n } from "@/i18n/provider";
import { docsNeighbors } from "@/lib/docs-nav";

export default function DocsPager() {
  const pathname = usePathname();
  const { t } = useI18n();
  const s = t.docs.sidebar;
  const { prev, next } = docsNeighbors(pathname);

  if (!prev && !next) return null;

  return (
    <nav
      className="mt-16 pt-8 border-t border-border flex flex-col sm:flex-row gap-4 sm:justify-between"
      aria-label="Page"
    >
      {prev ? (
        <Link
          href={prev.href}
          className="group rounded-md border border-border bg-surface px-4 py-3 sm:max-w-[48%] hover:border-border-accent transition-colors"
        >
          <div className="text-[11px] text-text-tertiary mb-1">← Previous</div>
          <div className="text-sm font-medium text-text-primary group-hover:text-accent-bright">
            {s[prev.labelKey]}
          </div>
        </Link>
      ) : (
        <span />
      )}
      {next ? (
        <Link
          href={next.href}
          className="group rounded-md border border-border bg-surface px-4 py-3 sm:max-w-[48%] sm:text-right hover:border-border-accent transition-colors sm:ml-auto"
        >
          <div className="text-[11px] text-text-tertiary mb-1">Next →</div>
          <div className="text-sm font-medium text-text-primary group-hover:text-accent-bright">
            {s[next.labelKey]}
          </div>
        </Link>
      ) : null}
    </nav>
  );
}
