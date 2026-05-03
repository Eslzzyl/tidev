import { useState } from "react";
import { Lightbulb, ChevronDown } from "lucide-react";
import { MarkdownRenderer } from "./MarkdownRenderer";
import { CopyButton } from "../ui/CopyButton";

interface Props {
  content: string;
  tokenCount?: number;
  defaultExpanded?: boolean;
}

export function ThinkingBlock({
  content,
  tokenCount,
  defaultExpanded = true,
}: Props) {
  const [expanded, setExpanded] = useState(defaultExpanded);

  return (
    <div className="mb-2">
      {/* Header row: icon + label + collapse toggle + copy button */}
      <div className="flex items-center gap-1.5">
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex items-center gap-1.5 text-left transition-colors hover:opacity-80"
        >
          <Lightbulb className="h-3.5 w-3.5 text-amber-500 dark:text-amber-400" />
          <span className="text-xs font-medium text-neutral-400 dark:text-neutral-500">
            Thinking
            {tokenCount ? ` (${tokenCount.toLocaleString()} tokens)` : ""}
          </span>
          <ChevronDown
            className={`h-3 w-3 text-neutral-400 transition-transform duration-200 dark:text-neutral-500 ${expanded ? "rotate-180" : ""}`}
          />
        </button>
        <CopyButton content={content} />
      </div>

      {/* Thinking content — only rendered when expanded */}
      {expanded && (
        <div className="text-neutral-500 dark:text-neutral-400">
          <MarkdownRenderer content={content} />
        </div>
      )}
    </div>
  );
}
