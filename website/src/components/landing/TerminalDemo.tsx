"use client";

import { useEffect, useRef, useState } from "react";
import { useDict } from "@/i18n/provider";
import { useInView } from "@/hooks/useInView";

const lineColors: Record<string, string> = {
  cmd: "text-text-primary font-semibold",
  sys: "text-text-tertiary",
  user: "text-accent",
  tool: "text-info",
  ok: "text-success",
  agent: "text-text-secondary",
  ctx: "text-text-tertiary",
  blank: "",
};

export function TerminalDemo() {
  const dict = useDict();
  const { terminal } = dict;
  const { ref, inView } = useInView(0.3);
  const [visibleLines, setVisibleLines] = useState(0);
  const started = useRef(false);

  useEffect(() => {
    if (!inView || started.current) return;
    started.current = true;
    const total = terminal.lines.length;
    let i = 0;
    const interval = setInterval(() => {
      i++;
      setVisibleLines(i);
      if (i >= total) clearInterval(interval);
    }, 100);
    return () => clearInterval(interval);
  }, [inView, terminal.lines.length]);

  return (
    <section className="py-section px-6">
      <div className="mx-auto max-w-3xl">
        <h2 className="text-2xl sm:text-3xl font-bold tracking-tight text-text-primary text-center">
          {terminal.title}
        </h2>
        <p className="mt-3 text-text-secondary text-center">
          {terminal.subtitle}
        </p>

        <div ref={ref} className="mt-10 rounded-lg border border-border bg-abyss overflow-hidden">
          {/* Title bar */}
          <div className="flex items-center px-4 py-2 border-b border-border bg-surface">
            <span className="text-xs text-text-tertiary font-mono">grok-pi</span>
          </div>

          {/* Terminal body */}
          <div className="p-4 font-mono text-[13px] leading-[1.7] min-h-[300px]">
            {terminal.lines.slice(0, visibleLines).map((line, i) => (
              <div key={i} className={`${lineColors[line.type] ?? "text-text-secondary"} whitespace-pre-wrap`}>
                {line.text || "\u00A0"}
              </div>
            ))}
            {visibleLines < terminal.lines.length && (
              <span className="inline-block w-2 h-4 bg-accent align-middle" />
            )}
          </div>
        </div>
      </div>
    </section>
  );
}

export default TerminalDemo;
