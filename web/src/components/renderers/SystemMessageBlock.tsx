import type { Message } from "../../types/api";
import { MarkdownRenderer } from "./MarkdownRenderer";

interface Props {
  message: Message;
}

const COMPACTION_LABEL = "Compaction";

export function SystemMessageBlock({ message }: Props) {
  const isCompaction = message.content.startsWith(COMPACTION_LABEL);

  if (isCompaction) {
    const summary = message.content
      .split("\n\n")
      .slice(1)
      .join("\n\n")
      .trim();

    return (
      <div className="px-4 py-2">
        {/* Divider line */}
        <div className="flex items-center gap-2 py-1">
          <div className="h-px flex-1 bg-neutral-200 dark:bg-neutral-700" />
          <span className="text-xs font-medium text-neutral-400 dark:text-neutral-500">
            {COMPACTION_LABEL}
          </span>
          <div className="h-px flex-1 bg-neutral-200 dark:bg-neutral-700" />
        </div>
        {summary && (
          <div className="mt-2 text-sm leading-relaxed text-neutral-600 dark:text-neutral-400">
            <MarkdownRenderer content={summary} />
          </div>
        )}
      </div>
    );
  }

  // Generic system message — render content as-is
  return (
    <div className="px-4 py-2">
      <div className="text-sm leading-relaxed text-neutral-500 dark:text-neutral-400">
        <MarkdownRenderer content={message.content} />
      </div>
    </div>
  );
}
