"use client";

import { useDict } from "@/i18n/provider";

export function Architecture() {
  const dict = useDict();
  const { architecture } = dict;

  return (
    <section id="architecture" className="py-section px-6">
      <div className="mx-auto max-w-3xl">
        <h2 className="text-2xl sm:text-3xl font-bold tracking-tight text-text-primary text-center">
          {architecture.title}
        </h2>
        <p className="mt-3 text-text-secondary text-center max-w-xl mx-auto">
          {architecture.subtitle}
        </p>

        {/* Layer diagram — flat boxes, no gradient, no hover scale */}
        <div className="mt-12 flex flex-col items-center gap-1">
          {architecture.layers.map((layer, i) => (
            <div key={layer.name} className="w-full max-w-md flex flex-col items-center gap-1">
              <div className="w-full rounded-md border border-border bg-surface px-6 py-5 text-center">
                <p className="font-mono text-sm font-semibold text-text-primary">
                  {layer.name}
                </p>
                <p className="mt-1.5 text-xs text-text-secondary leading-relaxed">
                  {layer.role}
                </p>
              </div>
              {i < architecture.layers.length - 1 && (
                <div className="flex flex-col items-center py-0.5 text-text-tertiary">
                  <svg width="12" height="20" viewBox="0 0 12 20" fill="none">
                    <path d="M6 0v14M2 11l4 5 4-5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                  <span className="text-[10px] font-mono">
                    {i === 0 ? "ACP" : "JSONL RPC"}
                  </span>
                </div>
              )}
            </div>
          ))}
        </div>

        {/* Guarantees — simple list */}
        <div className="mt-12 space-y-2">
          {architecture.guarantees.map((g) => (
            <div key={g} className="flex items-start gap-2.5 text-sm text-text-secondary">
              <span className="text-accent shrink-0 mt-px">✓</span>
              <span>{g}</span>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

export default Architecture;
