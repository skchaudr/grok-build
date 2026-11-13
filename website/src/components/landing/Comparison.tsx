"use client";

import { useI18n } from "@/i18n/provider";

export default function Comparison() {
  const { t } = useI18n();
  const { table } = t.comparison;

  return (
    <section id="comparison" className="py-section px-6">
      <div className="mx-auto max-w-5xl">
        <h2 className="text-2xl sm:text-3xl font-bold tracking-tight text-text-primary text-center">
          {t.comparison.title}
        </h2>
        <p className="mt-3 text-text-secondary text-center max-w-xl mx-auto">
          {t.comparison.subtitle}
        </p>

        <div className="mt-12 overflow-hidden rounded-lg border border-border">
          <div className="overflow-x-auto">
            <table className="w-full text-sm min-w-[600px]">
              <thead>
                <tr className="border-b border-border bg-surface">
                  <th className="px-5 py-3 text-left text-xs font-medium text-text-tertiary">
                    {table.capability}
                  </th>
                  <th className="px-5 py-3 text-left text-xs font-medium text-text-tertiary">
                    {table.grokBuild}
                  </th>
                  <th className="px-5 py-3 text-left text-xs font-medium text-accent">
                    {table.grokPi}
                  </th>
                </tr>
              </thead>
              <tbody>
                {table.rows.map((row, i) => (
                  <tr
                    key={row.capability}
                    className={`border-b border-border-subtle ${i % 2 === 1 ? "bg-surface/40" : ""}`}
                  >
                    <td className="px-5 py-3 font-medium text-text-primary text-[13px] whitespace-nowrap">
                      {row.capability}
                    </td>
                    <td className="px-5 py-3 text-text-tertiary text-[13px]">
                      {row.grokBuild}
                    </td>
                    <td className="px-5 py-3 text-text-secondary text-[13px]">
                      {row.grokPi}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      </div>
    </section>
  );
}
