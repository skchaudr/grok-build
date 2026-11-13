"use client";

import { useDict } from "@/i18n/provider";
import CodeBlock from "@/components/ui/CodeBlock";

function StepList({
  steps,
}: {
  steps: { step: string; title: string; desc: string; code: string }[];
}) {
  return (
    <div className="mt-6 space-y-6">
      {steps.map((s) => (
        <div key={s.step} className="flex gap-4">
          <div className="shrink-0 w-7 h-7 rounded-md border border-border bg-surface flex items-center justify-center font-mono text-xs font-semibold text-accent">
            {s.step}
          </div>
          <div className="flex-1">
            <h3 className="font-semibold text-text-primary">{s.title}</h3>
            <p className="mt-1 text-sm text-text-secondary">{s.desc}</p>
            <div className="mt-3">
              <CodeBlock code={s.code} />
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

function CardGrid({
  items,
}: {
  items: { title: string; desc: string }[];
}) {
  return (
    <div className="mt-6 grid gap-4 sm:grid-cols-2">
      {items.map((a) => (
        <div
          key={a.title}
          className="rounded-md border border-border bg-surface p-5"
        >
          <h3 className="font-medium text-text-primary text-sm">{a.title}</h3>
          <p className="mt-1.5 text-xs leading-relaxed text-text-secondary">
            {a.desc}
          </p>
        </div>
      ))}
    </div>
  );
}

export default function MigrationPage() {
  const dict = useDict();
  const d = dict.docsPages.migration;
  return (
    <div>
      <h1 className="text-3xl font-bold tracking-tight">
        {dict.docs.sidebar.migration}
      </h1>
      <p className="mt-4 text-lg leading-relaxed text-text-secondary">
        {d.intro}
      </p>

      <h2 id="from-grok" className="mt-10 text-xl font-semibold">
        {d.fromGrokTitle}
      </h2>
      <p className="mt-2 text-sm text-text-secondary">{d.fromGrokLead}</p>
      <StepList steps={[...d.grokSteps]} />

      <h2 id="from-pi" className="mt-12 text-xl font-semibold">
        {d.fromPiTitle}
      </h2>
      <p className="mt-2 text-sm text-text-secondary">{d.fromPiLead}</p>
      <StepList steps={[...d.piSteps]} />

      <h3 className="mt-10 text-base font-semibold text-text-primary">
        {d.keepTitle}
      </h3>
      <CardGrid items={[...d.piKeep]} />

      <h3 className="mt-10 text-base font-semibold text-text-primary">
        {d.changesTitle}
      </h3>
      <CardGrid items={[...d.piDiffs]} />

      <h2 id="migrate-home" className="mt-12 text-xl font-semibold">
        {d.migrateHomeTitle}
      </h2>
      <p className="mt-2 text-sm text-text-secondary">{d.migrateHomeLead}</p>
      <div className="mt-4 space-y-3">
        <CodeBlock
          code={`grok-pi migrate-home --status
grok-pi migrate-home --dry-run
grok-pi migrate-home`}
          label="migrate-home"
        />
      </div>
      <p className="mt-3 text-xs text-text-tertiary">{d.migrateHomeNote}</p>

      <h2 className="mt-12 text-xl font-semibold">{d.whyTitle}</h2>
      <CardGrid items={[...d.advantages]} />

      <h2 className="mt-12 text-xl font-semibold">{d.faqTitle}</h2>
      <div className="mt-6 space-y-4">
        {d.faqs.map((faq) => (
          <div
            key={faq.q}
            className="rounded-md border border-border bg-surface p-5"
          >
            <h3 className="font-medium text-text-primary text-sm">{faq.q}</h3>
            <p className="mt-2 text-sm leading-relaxed text-text-secondary">
              {faq.a}
            </p>
          </div>
        ))}
      </div>
    </div>
  );
}
