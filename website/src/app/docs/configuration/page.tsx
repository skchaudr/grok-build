"use client";

import { useDict } from "@/i18n/provider";
import CodeBlock from "@/components/ui/CodeBlock";

export default function ConfigurationPage() {
  const dict = useDict();
  const d = dict.docsPages.configuration;
  return (
    <div>
      <h1 className="text-3xl font-bold tracking-tight">
        {dict.docs.sidebar.configuration}
      </h1>
      <p className="mt-4 text-lg leading-relaxed text-text-secondary">
        {d.intro}
      </p>

      <h2 className="mt-10 text-xl font-semibold">{d.isolationTitle}</h2>
      <p className="mt-2 text-text-secondary">{d.isolationLead}</p>
      <div className="mt-4 overflow-x-auto rounded-md border border-border">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border bg-surface/80">
              <th className="px-4 py-3 text-left font-semibold text-text-secondary">
                {d.table.layer}
              </th>
              <th className="px-4 py-3 text-left font-semibold text-text-secondary">
                {d.table.stock}
              </th>
              <th className="px-4 py-3 text-left font-semibold text-text-secondary">
                {d.table.grokPi}
              </th>
            </tr>
          </thead>
          <tbody className="text-text-secondary">
            <tr className="border-b border-border/50">
              <td className="px-4 py-3">{d.table.userHome}</td>
              <td className="px-4 py-3 font-mono text-xs">~/.grok</td>
              <td className="px-4 py-3 font-mono text-xs text-accent">
                ~/.grok-pi
              </td>
            </tr>
            <tr>
              <td className="px-4 py-3">{d.table.projectTree}</td>
              <td className="px-4 py-3 font-mono text-xs">&lt;repo&gt;/.grok</td>
              <td className="px-4 py-3 font-mono text-xs text-accent">
                &lt;repo&gt;/.grok-pi
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <p className="mt-3 text-sm text-text-secondary">{d.isolationPi}</p>
      <div className="mt-3 space-y-3">
        <CodeBlock
          code={`grok-pi migrate-home --status
grok-pi migrate-home --dry-run
grok-pi migrate-home          # copy allowlisted files
grok-pi migrate-home --include-auth   # optional`}
          label="migrate-home"
        />
      </div>
      <p className="mt-3 text-xs text-text-tertiary">{d.migrateNote}</p>

      <h2 className="mt-10 text-xl font-semibold">{d.envTitle}</h2>
      <div className="mt-4 overflow-x-auto rounded-md border border-border">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border bg-surface/80">
              <th className="px-4 py-3 text-left font-semibold text-text-secondary">
                {d.envCols.variable}
              </th>
              <th className="px-4 py-3 text-left font-semibold text-text-secondary">
                {d.envCols.default}
              </th>
              <th className="px-4 py-3 text-left font-semibold text-text-secondary">
                {d.envCols.purpose}
              </th>
            </tr>
          </thead>
          <tbody>
            {d.envVars.map((v, i) => (
              <tr
                key={v.name}
                className={`border-b border-border/50 ${i % 2 === 1 ? "bg-surface/20" : ""}`}
              >
                <td className="px-4 py-3 font-mono text-xs text-accent whitespace-nowrap">
                  {v.name}
                </td>
                <td className="px-4 py-3 font-mono text-xs text-text-tertiary">
                  {v.default}
                </td>
                <td className="px-4 py-3 text-text-secondary">{v.desc}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <h2 className="mt-10 text-xl font-semibold">{d.flagsTitle}</h2>
      <p className="mt-2 text-text-secondary">{d.flagsLead}</p>
      <div className="mt-4">
        <CodeBlock
          code="grok-pi -- --model openai/gpt-4o"
          label={d.passFlagsLabel}
        />
      </div>
      <div className="mt-4 overflow-x-auto rounded-md border border-border">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border bg-surface/80">
              <th className="px-4 py-3 text-left font-semibold text-text-secondary">
                {d.flagCols.flag}
              </th>
              <th className="px-4 py-3 text-left font-semibold text-text-secondary">
                {d.flagCols.desc}
              </th>
            </tr>
          </thead>
          <tbody>
            {d.flags.map((f, i) => (
              <tr
                key={f.flag}
                className={`border-b border-border/50 ${i % 2 === 1 ? "bg-surface/20" : ""}`}
              >
                <td className="px-4 py-3 font-mono text-xs text-accent whitespace-nowrap">
                  {f.flag}
                </td>
                <td className="px-4 py-3 text-text-secondary">{f.desc}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <h2 className="mt-10 text-xl font-semibold">{d.f2Title}</h2>
      <p className="mt-2 text-sm text-text-secondary">{d.f2Lead}</p>
      <div className="mt-4 overflow-x-auto rounded-md border border-border">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border bg-surface/80">
              <th className="px-4 py-3 text-left font-semibold text-text-secondary">
                {d.f2Cols.setting}
              </th>
              <th className="px-4 py-3 text-left font-semibold text-text-secondary">
                {d.f2Cols.default}
              </th>
              <th className="px-4 py-3 text-left font-semibold text-text-secondary">
                {d.f2Cols.notes}
              </th>
            </tr>
          </thead>
          <tbody>
            {d.f2Gates.map((g, i) => (
              <tr
                key={g.key}
                className={`border-b border-border/50 ${i % 2 === 1 ? "bg-surface/20" : ""}`}
              >
                <td className="px-4 py-3 font-mono text-xs text-accent whitespace-nowrap">
                  {g.key}
                </td>
                <td className="px-4 py-3 font-mono text-xs text-text-tertiary">
                  {g.def}
                </td>
                <td className="px-4 py-3 text-text-secondary">{g.note}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <h2 className="mt-10 text-xl font-semibold">{d.resourcesTitle}</h2>
      <p className="mt-2 text-text-secondary">{d.resourcesBody}</p>
      <ul className="mt-3 space-y-1.5 text-sm text-text-secondary list-disc pl-5">
        {d.resourcesBullets.map((b) => (
          <li key={b}>{b}</li>
        ))}
      </ul>

      <h2 className="mt-10 text-xl font-semibold">{d.selfHealTitle}</h2>
      <p className="mt-2 text-text-secondary">{d.selfHealBody}</p>

      <h2 className="mt-10 text-xl font-semibold">{d.themesTitle}</h2>
      <p className="mt-2 text-text-secondary">{d.themesBody}</p>
      <ul className="mt-3 space-y-1.5 text-sm text-text-secondary">
        <li>
          <code className="font-mono text-xs text-accent">
            pi:transparent
          </code>{" "}
          — {d.themeDark.replace("pi:transparent — ", "")}
        </li>
        <li>
          <code className="font-mono text-xs text-accent">
            pi:transparent-light
          </code>{" "}
          — {d.themeLight.replace("pi:transparent-light — ", "")}
        </li>
      </ul>
    </div>
  );
}
