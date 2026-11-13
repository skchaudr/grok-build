"use client";

import Link from "next/link";
import { useDict } from "@/i18n/provider";
import CodeBlock from "@/components/ui/CodeBlock";

export default function DocsHome() {
  const dict = useDict();
  const d = dict.docsPages.home;
  return (
    <div>
      <h1 className="text-3xl font-bold tracking-tight">{dict.docs.title}</h1>
      <p className="mt-4 text-lg leading-relaxed text-text-secondary max-w-2xl">
        {d.intro}{" "}
        <strong className="text-text-primary">v0.0.8</strong> {d.versionNote}
      </p>

      <div className="mt-10 grid gap-4 sm:grid-cols-2">
        {d.cards.map((card) => (
          <Link
            key={card.href}
            href={card.href}
            className="group rounded-md border border-border bg-surface p-5 transition-colors duration-150 hover:border-border-accent"
          >
            <h3 className="font-medium text-text-primary text-sm">
              {card.title}
            </h3>
            <p className="mt-2 text-sm text-text-secondary leading-relaxed">
              {card.desc}
            </p>
          </Link>
        ))}
      </div>

      <div className="mt-12">
        <h2 className="text-xl font-semibold mb-4">{d.quickStart}</h2>
        <div className="space-y-3">
          <CodeBlock
            code="curl -fsSL https://github.com/Dwsy/grok-pi/releases/latest/download/install.sh | sh"
            label={d.labels.install}
          />
          <CodeBlock
            code="npm install --global @earendil-works/pi-coding-agent"
            label={d.labels.ensurePi}
          />
          <CodeBlock code="cd your-project && grok-pi" label={d.labels.run} />
        </div>
      </div>
    </div>
  );
}
