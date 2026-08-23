import { memo, useEffect, useState } from "react";
import type { TFunction } from "i18next";
import { Lightbulb } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ActivityRipple } from "./ActivityRipple";
import { MarkdownRenderer } from "./MarkdownRenderer";

interface Props {
  content: string;
  tokenCount?: number;
  active?: boolean;
  startedAt?: string;
  completedAt?: string;
}

function formatThoughtDuration(milliseconds: number, t: TFunction) {
  if (milliseconds < 1000) {
    return t("{{count}} milliseconds", { count: milliseconds });
  }

  const totalSeconds = Math.floor(milliseconds / 1000);
  if (totalSeconds < 60) {
    return t("{{count}} seconds", { count: (milliseconds / 1000).toFixed(1) });
  }

  const seconds = totalSeconds % 60;
  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) {
    return t("{{minutes}} minutes {{seconds}} seconds", {
      minutes: totalMinutes,
      seconds,
    });
  }

  return t("{{hours}} hours {{minutes}} minutes {{seconds}} seconds", {
    hours: Math.floor(totalMinutes / 60),
    minutes: totalMinutes % 60,
    seconds,
  });
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
  const [elapsedMs, setElapsedMs] = useState<number | null>(null);

  useEffect(() => {
    const start = Date.parse(startedAt);
    const completed = completedAt ? Date.parse(completedAt) : Number.NaN;
    if (Number.isNaN(start)) {
      setElapsedMs(null);
      return;
    }

    const update = () => {
      const end = Number.isNaN(completed) ? Date.now() : completed;
      setElapsedMs(Math.max(0, end - start));
    };

    update();
    if (!Number.isNaN(completed) || !active) return;
    const timer = setInterval(update, 100);
    return () => clearInterval(timer);
  }, [active, completedAt, startedAt]);

  if (elapsedMs === null) {
    return <span className="thinking-elapsed">{t("Thinking")}</span>;
  }

  return (
    <span className="thinking-elapsed">
      {t("Thought for {{duration}}", {
        duration: formatThoughtDuration(elapsedMs, t),
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
}: Props) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="thinking-block">
      <div className="thinking-header">
        <button
          className="thinking-toggle"
          onClick={() => setExpanded((value) => !value)}
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
      <div className={expanded ? "thinking-body expanded" : "thinking-body"}>
        <div className="thinking-markdown">
          <MarkdownRenderer content={content} />
        </div>
      </div>
    </div>
  );
});
