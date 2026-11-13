"use client";

import { useCopyToClipboard } from "@/hooks/useCopyToClipboard";
import { cn } from "@/lib/utils";

interface CodeBlockProps {
  code: string;
  language?: string;
  title?: string;
  label?: string;
  className?: string;
}

export function CodeBlock({ code, language = "bash", title, label, className }: CodeBlockProps) {
  const displayTitle = title || label;
  const { copied, copy } = useCopyToClipboard();

  return (
    <div className={cn("group relative rounded-lg border border-border bg-abyss overflow-hidden", className)}>
      {/* Header bar */}
      <div className="flex items-center justify-between px-4 py-2 border-b border-border-subtle bg-surface">
        <div className="flex items-center gap-2">
          {displayTitle && <span className="text-xs text-text-tertiary font-mono">{displayTitle}</span>}
        </div>
        <button
          onClick={() => copy(code)}
          className="flex items-center gap-1.5 px-2 py-1 rounded text-xs text-text-tertiary hover:text-text-primary hover:bg-surface-hover transition-colors"
        >
          {copied ? (
            <>
              <svg className="w-3.5 h-3.5 text-success" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
              </svg>
              <span className="text-success">Copied</span>
            </>
          ) : (
            <>
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
              </svg>
              Copy
            </>
          )}
        </button>
      </div>
      {/* Code content */}
      <pre className="p-4 overflow-x-auto">
        <code className="text-sm font-mono text-text-secondary leading-relaxed whitespace-pre">{code}</code>
      </pre>
    </div>
  );
}


export default CodeBlock;
