"use client";

import { useDict } from "@/i18n/provider";
import { CodeBlock } from "@/components/ui/CodeBlock";
import { withBase } from "@/lib/paths";

export function Migration() {
  const dict = useDict();
  const { migration } = dict;

  return (
    <section id="migration" className="py-section px-6">
      <div className="mx-auto max-w-4xl">
        <h2 className="text-2xl sm:text-3xl font-bold tracking-tight text-text-primary text-center">
          {migration.title}
        </h2>
        <p className="mt-3 text-text-secondary text-center max-w-xl mx-auto">
          {migration.subtitle}{" "}
          <a
            href={withBase("/docs/migration/")}
            className="text-accent hover:underline"
          >
            Docs → Migration
          </a>
        </p>

        {/* Steps — numbered sequence, no decorative markers */}
        <div className="mt-12 space-y-6">
          {migration.steps.map((step, i) => (
            <div key={step.step} className="flex gap-4">
              <div className="shrink-0 w-7 h-7 rounded-md border border-border bg-surface flex items-center justify-center">
                <span className="text-xs font-mono font-semibold text-accent">{i + 1}</span>
              </div>
              <div className="flex-1 min-w-0">
                <h3 className="text-sm font-semibold text-text-primary mb-1">{step.title}</h3>
                <p className="text-[13px] text-text-secondary mb-2.5">{step.desc}</p>
                <CodeBlock code={step.code} />
              </div>
            </div>
          ))}
        </div>

        {/* Advantages — plain list */}
        <div className="mt-12 grid grid-cols-1 sm:grid-cols-2 gap-4">
          {migration.advantages.map((a) => (
            <div key={a.title} className="rounded-md border border-border bg-surface p-5">
              <h3 className="text-sm font-semibold text-text-primary mb-1.5">{a.title}</h3>
              <p className="text-[13px] text-text-secondary leading-relaxed">{a.desc}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

export default Migration;
