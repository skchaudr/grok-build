"use client";

import { useDict } from "@/i18n/provider";
import { CodeBlock } from "@/components/ui/CodeBlock";

export function Download() {
  const dict = useDict();
  const { download } = dict;

  return (
    <section id="download" className="py-section px-6">
      <div className="mx-auto max-w-3xl text-center">
        <h2 className="text-2xl sm:text-3xl font-bold tracking-tight text-text-primary">
          {download.title}
        </h2>
        <p className="mt-3 text-text-secondary">
          {download.subtitle}
        </p>

        <div className="mt-10 space-y-4 text-left">
          {download.platforms.map((p) => (
            <div key={p.os}>
              <p className="text-xs font-medium text-text-tertiary mb-1.5">{p.os}</p>
              <CodeBlock code={p.cmd} />
            </div>
          ))}
        </div>

        <p className="mt-6 text-xs text-text-tertiary">{download.note}</p>

        <a
          href="https://github.com/Dwsy/grok-pi"
          target="_blank"
          rel="noopener noreferrer"
          className="mt-8 inline-flex items-center gap-2 px-5 py-2.5 rounded-md border border-border text-sm text-text-secondary hover:text-text-primary hover:bg-surface-hover transition-colors duration-150"
        >
          {download.cta}
          <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
          </svg>
        </a>
      </div>
    </section>
  );
}

export default Download;
