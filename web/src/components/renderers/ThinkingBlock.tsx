import { useState } from 'react';
import { Lightbulb, ChevronDown } from 'lucide-react';
import { MarkdownRenderer } from './MarkdownRenderer';
import { CopyButton } from '../ui/CopyButton';

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
          <Lightbulb className="h-4 w-4 text-amber-600 dark:text-amber-400" />
          <span className="text-sm font-medium text-amber-800 dark:text-amber-200">
            Thinking{tokenCount ? ` (${tokenCount.toLocaleString()} tokens)` : ''}
          </span>
        </div>
        <ChevronDown
            className={`h-4 w-4 text-amber-600 transition-transform dark:text-amber-400 ${expanded ? 'rotate-180' : ''}`}
          />
      </button>

      {expanded ? (
        <div className="border-t border-amber-200 px-4 py-3 dark:border-amber-800">
          <div className="mb-2 flex items-center gap-2">
            <span className="text-xs font-medium text-amber-800 dark:text-amber-200">Thinking</span>
            <CopyButton content={content} />
          </div>
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
