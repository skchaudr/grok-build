"use client";

import { useDict } from "@/i18n/provider";
import CodeBlock from "@/components/ui/CodeBlock";

export default function ExtensionsPage() {
  const dict = useDict();
  const d = dict.docsPages.extensions;
  return (
    <div>
      <h1 className="text-3xl font-bold tracking-tight">
        {dict.docs.sidebar.extensions}
      </h1>
      <p className="mt-4 text-lg leading-relaxed text-text-secondary max-w-2xl">
        {d.intro}
      </p>

      <h2 id="recommended" className="mt-10 text-xl font-semibold">
        {d.recommendedTitle}
      </h2>
      <p className="mt-2 text-sm text-text-secondary">{d.recommendedLead}</p>

      <div className="mt-6 space-y-8">
        {d.recommended.map((r) => (
          <section
            key={r.pkg}
            className="rounded-md border border-border bg-surface p-5"
          >
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="font-semibold text-text-primary">{r.title}</h3>
              <span className="text-[10px] uppercase tracking-wide px-2 py-0.5 rounded border border-border text-text-tertiary">
                {r.status}
              </span>
            </div>
            <p className="mt-1 font-mono text-xs text-accent">{r.pkg}</p>
            <p className="mt-3 text-sm leading-relaxed text-text-secondary">
              {r.body}
            </p>
            <div className="mt-4">
              <CodeBlock code={r.install} label={d.installLabel} />
            </div>
            <ul className="mt-4 space-y-1.5 text-sm text-text-secondary list-disc pl-5">
              {r.notes.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          </section>
        ))}
      </div>

      <div className="mt-6">
        <CodeBlock
          code={`# Recommended pair for grok-pi workflows
pi install npm:@juicesharp/rpiv-todo
pi install npm:@juicesharp/rpiv-ask-user-question

# Verify in resource manager
grok-pi
# then F2 or /pi-config → Extensions`}
          label={d.quickInstallLabel}
        />
      </div>

      <h2 id="bash" className="mt-12 text-xl font-semibold">
        {d.bashTitle}
      </h2>
      <p className="mt-2 text-sm text-text-secondary">{d.bashLead}</p>
      <ul className="mt-4 space-y-1.5 text-sm text-text-secondary list-disc pl-5">
        {d.bashPoints.map((p) => (
          <li key={p}>{p}</li>
        ))}
      </ul>
      <p className="mt-3 text-xs text-text-tertiary font-mono">{d.bashGate}</p>
      <div className="mt-4 rounded-md border border-border bg-surface p-4 text-sm text-text-secondary">
        <p className="font-medium text-text-primary text-sm mb-2">
          {d.bashTuiTitle}
        </p>
        <ol className="list-decimal pl-5 space-y-1">
          {d.bashTuiSteps.map((s) => (
            <li key={s}>{s}</li>
          ))}
        </ol>
      </div>

      <h2 id="subagents" className="mt-12 text-xl font-semibold">
        {d.subagentsTitle}
      </h2>
      <p className="mt-2 text-sm text-text-secondary">{d.subagentsLead}</p>
      <ul className="mt-4 space-y-1.5 text-sm text-text-secondary list-disc pl-5">
        {d.subagentsPoints.map((p) => (
          <li key={p}>{p}</li>
        ))}
      </ul>
      <p className="mt-3 text-xs text-text-tertiary font-mono">
        {d.subagentsGate}
      </p>
      <div className="mt-4 overflow-x-auto">
        <table className="w-full text-sm border-collapse">
          <thead>
            <tr className="border-b border-border text-left text-text-tertiary">
              <th className="py-2 pr-4 font-medium">{d.profileCols.profile}</th>
              <th className="py-2 pr-4 font-medium">{d.profileCols.tools}</th>
              <th className="py-2 font-medium">{d.profileCols.useWhen}</th>
            </tr>
          </thead>
          <tbody className="text-text-secondary">
            {d.profiles.map((row) => (
              <tr key={row.profile} className="border-b border-border/60">
                <td className="py-2 pr-4 font-mono text-xs text-accent">
                  {row.profile}
                </td>
                <td className="py-2 pr-4">{row.tools}</td>
                <td className="py-2">{row.useWhen}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <h2 id="catalog" className="mt-12 text-xl font-semibold">
        {d.catalogTitle}
      </h2>
      <p className="mt-2 text-sm text-text-secondary">{d.catalogLead}</p>
      <div className="mt-4 overflow-x-auto">
        <table className="w-full text-sm border-collapse">
          <thead>
            <tr className="border-b border-border text-left text-text-tertiary">
              <th className="py-2 pr-3 font-medium">
                {d.catalogCols.extension}
              </th>
              <th className="py-2 pr-3 font-medium">{d.catalogCols.role}</th>
              <th className="py-2 font-medium">{d.catalogCols.default}</th>
            </tr>
          </thead>
          <tbody className="text-text-secondary">
            {d.catalog.map((row) => (
              <tr key={row.name} className="border-b border-border/60">
                <td className="py-2 pr-3 font-mono text-xs text-accent whitespace-nowrap">
                  {row.name}
                </td>
                <td className="py-2 pr-3">{row.role}</td>
                <td className="py-2 whitespace-nowrap">{row.default}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="mt-3 text-xs text-text-tertiary">{d.catalogFoot}</p>

      <h2 id="advanced" className="mt-12 text-xl font-semibold">
        {d.advancedTitle}
      </h2>
      <div className="mt-6 space-y-4">
        {d.advanced.map((s) => (
          <div
            key={s.title}
            className="rounded-md border border-border bg-surface p-5"
          >
            <h3 className="font-medium text-text-primary text-sm">{s.title}</h3>
            <p className="mt-2 text-sm leading-relaxed text-text-secondary">
              {s.body}
            </p>
          </div>
        ))}
      </div>

      <h2 id="control" className="mt-12 text-xl font-semibold">
        {d.controlTitle}
      </h2>
      <div className="mt-4 space-y-3">
        <CodeBlock
          code={`# Disable all injected bridge extensions
grok-pi -ne
# or
grok-pi --no-extensions

# Bash only
PI_GROK_BASH=0 grok-pi

# Remote TUI (custom UI host for packages like rpiv-ask)
PI_GROK_REMOTE_TUI=0 grok-pi   # off
PI_GROK_REMOTE_TUI=1 grok-pi   # on (default)`}
          label={d.controlGatesLabel}
        />
      </div>
      <p className="mt-3 text-sm text-text-secondary">{d.controlNote}</p>

      <h2 id="faq" className="mt-12 text-xl font-semibold">
        {d.faqTitle}
      </h2>
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
