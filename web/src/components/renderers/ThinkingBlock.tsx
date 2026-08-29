import { memo, useEffect, useState } from "react";
import type { TFunction } from "i18next";
import { Lightbulb } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ExpandableBody } from "../ui/ExpandableBody";
import { ActivityRipple } from "./ActivityRipple";
import { MarkdownRenderer } from "./MarkdownRenderer";
import { formatDurationHuman } from "../../utils/format";

interface Props {
  content: string;
  tokenCount?: number;
  active?: boolean;
  startedAt?: string;
  completedAt?: string;
  expanded?: boolean;
  onExpandedChange?: (expanded: boolean) => void;
}

function ElapsedTimer({
  startedAt,
  completedAt,
  active,
}: {
  startedAt: string;
  completedAt?: string;
  active: boolean;
}) {
  const { t } = useTranslation();
  const start = Date.parse(startedAt);
  const completed = completedAt ? Date.parse(completedAt) : Number.NaN;
  const fixedElapsedMs = Number.isNaN(start)
    ? null
    : !Number.isNaN(completed)
      ? Math.max(0, completed - start)
      : !active
        ? Math.max(0, Date.now() - start)
        : null;
  const [liveElapsedMs, setLiveElapsedMs] = useState<number | null>(() =>
    Number.isNaN(start) ? null : Math.max(0, Date.now() - start),
  );

  useEffect(() => {
    if (Number.isNaN(start) || !Number.isNaN(completed) || !active) return;
    const update = () =>
      setLiveElapsedMs((current) => {
        const next = Math.max(0, Date.now() - start);
        return current === next ? current : next;
      });
    const timer = setInterval(update, 100);
    return () => clearInterval(timer);
  }, [active, completedAt, startedAt]);

  const elapsedMs = fixedElapsedMs ?? liveElapsedMs;
  if (elapsedMs === null) {
    return <span className="thinking-elapsed">{t("Thinking")}</span>;
  }

  return (
    <span className="thinking-elapsed">
      {t("Thought for {{duration}}", {
        duration: formatDurationHuman(elapsedMs, t, !active),
      })}
    </span>
  );
}

export const ThinkingBlock = memo(function ThinkingBlock({
  content,
  tokenCount,
  active = false,
  startedAt,
  completedAt,
  expanded: controlledExpanded,
  onExpandedChange,
}: Props) {
  const { t } = useTranslation();
  const [localExpanded, setLocalExpanded] = useState(false);
  const expanded = controlledExpanded ?? localExpanded;

  function toggleExpanded() {
    const next = !expanded;
    if (controlledExpanded === undefined) setLocalExpanded(next);
    onExpandedChange?.(next);
  }

  return (
    <div className="thinking-block">
      <div className="thinking-header">
        <button
          className="thinking-toggle"
          onClick={toggleExpanded}
          aria-expanded={expanded}
        >
          <ActivityRipple active={active} row label={t("Thinking")}>
            <Lightbulb size={14} />
            <span className="thinking-label">
              {startedAt ? (
                <ElapsedTimer startedAt={startedAt} completedAt={completedAt} active={active} />
              ) : (
                t("Thinking")
              )}
              {tokenCount
                ? ` (${t("{{count}} tokens", { count: tokenCount.toLocaleString() })})`
                : ""}
            </span>
          </ActivityRipple>
        </button>
      </div>
      <ExpandableBody expanded={expanded} className="thinking-body">
        <div className="thinking-markdown">
          <MarkdownRenderer content={content} />
        </div>
      </ExpandableBody>
    </div>
  );
});
