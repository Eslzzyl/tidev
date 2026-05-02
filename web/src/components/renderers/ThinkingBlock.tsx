import { useState } from 'react';
import { MarkdownRenderer } from './MarkdownRenderer';

interface Props {
  content: string;
  tokenCount?: number;
  defaultExpanded?: boolean;
}

function getPreview(text: string): string {
  const maxLen = 100;
  if (text.length <= maxLen) return text;
  return text.slice(0, maxLen) + '...';
}

export function ThinkingBlock({ content, tokenCount, defaultExpanded = false }: Props) {
  const [expanded, setExpanded] = useState(defaultExpanded);

  return (
    <div className="mb-3 overflow-hidden rounded-lg border border-amber-200 bg-amber-50 dark:border-amber-800 dark:bg-amber-950/50">
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center justify-between px-4 py-2 text-left transition-colors hover:bg-amber-100 dark:hover:bg-amber-900/50"
      >
        <div className="flex items-center gap-2">
          <svg
            className="h-4 w-4 text-amber-600 dark:text-amber-400"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
            />
          </svg>
          <span className="text-sm font-medium text-amber-800 dark:text-amber-200">
            Thinking{tokenCount ? ` (${tokenCount.toLocaleString()} tokens)` : ''}
          </span>
        </div>
        <svg
          className={`h-4 w-4 text-amber-600 transition-transform dark:text-amber-400 ${expanded ? 'rotate-180' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>

      {expanded ? (
        <div className="border-t border-amber-200 px-4 py-3 dark:border-amber-800">
          <MarkdownRenderer content={content} />
        </div>
      ) : (
        <div className="border-t border-amber-200 px-4 py-2 dark:border-amber-800">
          <p className="truncate text-sm text-amber-700/70 dark:text-amber-300/70">
            {getPreview(content)}
          </p>
        </div>
      )}
    </div>
  );
}
