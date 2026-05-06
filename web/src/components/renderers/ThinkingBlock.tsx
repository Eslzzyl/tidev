import { useState, useEffect } from "react";
import { Lightbulb, ChevronDown, Timer } from "lucide-react";
import { MarkdownRenderer } from "./MarkdownRenderer";
import { CopyButton } from "../ui/CopyButton";

interface Props {
  content: string;
  tokenCount?: number;
  defaultExpanded?: boolean;
  /** Timestamp when reasoning started, used to show elapsed time */
  startedAt?: string;
}

function ElapsedTimer({ startedAt }: { startedAt: string }) {
  const [elapsed, setElapsed] = useState("");

  useEffect(() => {
    const start = new Date(startedAt).getTime();
    if (isNaN(start)) return;

    const update = () => {
      const now = Date.now();
      const diff = now - start;
      const seconds = Math.floor(diff / 1000);
      const minutes = Math.floor(seconds / 60);
      if (minutes > 0) {
        setElapsed(`${minutes}m ${seconds % 60}s`);
      } else {
        setElapsed(`${seconds}s`);
      }
    };

    update();
    const interval = setInterval(update, 1000);
    return () => clearInterval(interval);
  }, [startedAt]);

  if (!elapsed) return null;

  return (
    <span className="flex items-center gap-1 text-[10px] text-neutral-400 dark:text-neutral-500">
      <Timer className="h-3 w-3" />
      {elapsed}
    </span>
  );
}

export function ThinkingBlock({
  content,
  tokenCount,
  defaultExpanded = true,
  startedAt,
}: Props) {
  const [expanded, setExpanded] = useState(defaultExpanded);

  return (
    <div className="mb-2">
      {/* Header row: icon + label + collapse toggle + copy button + timer */}
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
        {startedAt && <ElapsedTimer startedAt={startedAt} />}
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
