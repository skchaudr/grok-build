"use client";

import { useDict } from "@/i18n/provider";

function Table({
  rows,
  commandLabel,
  descriptionLabel,
}: {
  rows: { cmd: string; desc: string }[];
  commandLabel: string;
  descriptionLabel: string;
}) {
  return (
    <div className="mt-4 overflow-x-auto rounded-md border border-border">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border bg-surface/80">
            <th className="px-4 py-3 text-left font-semibold text-text-secondary">
              {commandLabel}
            </th>
            <th className="px-4 py-3 text-left font-semibold text-text-secondary">
              {descriptionLabel}
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((c, i) => (
            <tr
              key={c.cmd}
              className={`border-b border-border/50 ${i % 2 === 1 ? "bg-surface/20" : ""}`}
            >
              <td className="px-4 py-2.5 font-mono text-xs text-accent whitespace-nowrap">
                {c.cmd.startsWith("/") || c.cmd.startsWith("Ctrl")
                  ? c.cmd
                  : `/${c.cmd}`}
              </td>
              <td className="px-4 py-2.5 text-text-secondary">{c.desc}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export default function CommandsPage() {
  const dict = useDict();
  const d = dict.docsPages.commands;
  return (
    <div>
      <h1 className="text-3xl font-bold tracking-tight">
        {dict.docs.sidebar.commands}
      </h1>
      <p className="mt-4 text-lg leading-relaxed text-text-secondary">
        {d.intro}
      </p>

      <h2 className="mt-10 text-xl font-semibold">{d.retainedTitle}</h2>
      <Table
        rows={[...d.nativeCommands]}
        commandLabel={d.cols.command}
        descriptionLabel={d.cols.description}
      />

      <h2 className="mt-10 text-xl font-semibold">{d.sessionTitle}</h2>
      <Table
        rows={[...d.sessionCommands]}
        commandLabel={d.cols.command}
        descriptionLabel={d.cols.description}
      />
      <ul className="mt-4 space-y-1.5 text-sm text-text-secondary list-disc pl-5">
        {d.treeNotes.map((n) => (
          <li key={n}>{n}</li>
        ))}
      </ul>

      <h2 className="mt-10 text-xl font-semibold">{d.modeTitle}</h2>
      <p className="mt-2 text-sm text-text-secondary">{d.modeLead}</p>
      <Table
        rows={[...d.modeCommands]}
        commandLabel={d.cols.command}
        descriptionLabel={d.cols.description}
      />

      <h2 className="mt-10 text-xl font-semibold">{d.resourceTitle}</h2>
      <Table
        rows={[...d.resourceCommands]}
        commandLabel={d.cols.command}
        descriptionLabel={d.cols.description}
      />

      <h2 className="mt-10 text-xl font-semibold">{d.dynamicTitle}</h2>
      <p className="mt-2 text-text-secondary">{d.dynamicBody}</p>

      <h2 className="mt-10 text-xl font-semibold">{d.excludedTitle}</h2>
      <p className="mt-2 text-text-secondary">{d.excludedLead}</p>
      <div className="mt-3 flex flex-wrap gap-2">
        {d.excludedCommands.map((cmd) => (
          <code
            key={cmd}
            className="px-2.5 py-1 rounded-md bg-surface border border-border text-xs font-mono text-text-tertiary line-through"
          >
            /{cmd}
          </code>
        ))}
      </div>
      <p className="mt-4 text-sm text-text-tertiary">{d.loginNote}</p>
    </div>
  );
}
