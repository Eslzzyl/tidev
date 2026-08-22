import { memo, useEffect, useState } from "react";
import { ChevronDown, Lightbulb, Timer } from "lucide-react";

import { CopyButton } from "../ui/CopyButton";
import { MarkdownRenderer } from "./MarkdownRenderer";

interface Props {
  content: string;
  tokenCount?: number;
  defaultExpanded?: boolean;
  startedAt?: string;
}

function ElapsedTimer({ startedAt }: { startedAt: string }) {
  const [elapsed, setElapsed] = useState("");

  useEffect(() => {
    const start = new Date(startedAt).getTime();
    if (Number.isNaN(start)) return;

    const update = () => {
      const seconds = Math.floor((Date.now() - start) / 1000);
      const minutes = Math.floor(seconds / 60);
      setElapsed(minutes > 0 ? `${minutes}m ${seconds % 60}s` : `${seconds}s`);
    };

    update();
    const timer = setInterval(update, 1000);
    return () => clearInterval(timer);
  }, [startedAt]);

  if (!elapsed) return null;

  return (
    <span className="thinking-elapsed">
      <Timer size={12} />
      {elapsed}
    </span>
  );
}

export const ThinkingBlock = memo(function ThinkingBlock({
  content,
  tokenCount,
  defaultExpanded = true,
  startedAt,
}: Props) {
  const [expanded, setExpanded] = useState(defaultExpanded);

  return (
    <div className="thinking-block">
      <div className="thinking-header">
        <button className="thinking-toggle" onClick={() => setExpanded((value) => !value)}>
          <Lightbulb size={14} />
          <span>
            Thinking
            {tokenCount ? ` (${tokenCount.toLocaleString()} tokens)` : ""}
          </span>
          <ChevronDown
            className={expanded ? "thinking-chevron expanded" : "thinking-chevron"}
            size={13}
          />
        </button>
        <CopyButton content={content} />
        {startedAt ? <ElapsedTimer startedAt={startedAt} /> : null}
      </div>
      <div className={expanded ? "thinking-body expanded" : "thinking-body"}>
        <div className="thinking-markdown">
          <MarkdownRenderer content={content} />
        </div>
      </div>
    </div>
  );
});
