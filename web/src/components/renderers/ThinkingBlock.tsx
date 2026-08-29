import { memo, useEffect, useState } from "react";
import { Lightbulb } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ExpandableBody } from "../ui/ExpandableBody";
import { ActivityRipple } from "./ActivityRipple";
import { MarkdownRenderer } from "./MarkdownRenderer";
import { formatThinkingDuration } from "../../utils/format";

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
  const [liveNow, setLiveNow] = useState(() => Date.now());

  useEffect(() => {
    if (Number.isNaN(start) || !Number.isNaN(completed) || !active) return;
    let timer: number | undefined;

    const update = () => {
      const now = Date.now();
      const elapsedMs = Math.max(0, now - start);
      setLiveNow(now);

      const cadenceMs = elapsedMs < 60_000 ? 100 : 1000;
      const remainderMs = elapsedMs % cadenceMs;
      const delayMs = Math.max(1, cadenceMs - remainderMs);
      timer = window.setTimeout(update, delayMs);
    };

    update();
    return () => {
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [active, completedAt, start]);

  const liveElapsedMs = Number.isNaN(start) ? null : Math.max(0, liveNow - start);
  const elapsedMs = fixedElapsedMs ?? liveElapsedMs;
  if (elapsedMs === null) {
    return <span className="thinking-elapsed">{t("Thinking")}</span>;
  }

  return (
    <span className="thinking-elapsed">
      {t("Thought for {{duration}}", {
        duration: formatThinkingDuration(elapsedMs, t),
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
        <button className="thinking-toggle" onClick={toggleExpanded} aria-expanded={expanded}>
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
