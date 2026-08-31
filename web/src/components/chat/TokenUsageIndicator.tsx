import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import type { MessageRecord } from "../../types/api";
import { Popover } from "../ui";

interface TokenUsageIndicatorProps {
  messages: MessageRecord[];
  contextWindow?: number;
}

interface TokenUsageTotals {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
}

function formatTokenCount(count: number): string {
  if (count >= 1_000_000_000_000) return `${(count / 1_000_000_000_000).toFixed(1)}T`;
  if (count >= 1_000_000_000) return `${(count / 1_000_000_000).toFixed(1)}B`;
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K`;
  return String(count);
}

function collectTokenUsage(messages: MessageRecord[]): TokenUsageTotals | null {
  let usage: TokenUsageTotals = {
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
    totalTokens: 0,
  };
  let hasUsage = false;

  for (const record of messages) {
    const message = record.message;
    if (message.role !== "assistant") continue;

    const tokenValues = [
      message.input_tokens,
      message.output_tokens,
      message.total_tokens,
      message.cache_read_tokens,
      message.cache_write_tokens,
    ];
    if (!tokenValues.some((value) => value !== null)) continue;

    hasUsage = true;
    const inputTokens = message.input_tokens ?? 0;
    const outputTokens = message.output_tokens ?? 0;
    usage = {
      inputTokens: usage.inputTokens + inputTokens,
      outputTokens: usage.outputTokens + outputTokens,
      cacheReadTokens: usage.cacheReadTokens + (message.cache_read_tokens ?? 0),
      cacheWriteTokens: usage.cacheWriteTokens + (message.cache_write_tokens ?? 0),
      totalTokens: usage.totalTokens + (message.total_tokens ?? inputTokens + outputTokens),
    };
  }

  return hasUsage ? usage : null;
}

export function TokenUsageIndicator({ messages, contextWindow }: TokenUsageIndicatorProps) {
  const { t } = useTranslation();
  const usage = useMemo(() => collectTokenUsage(messages), [messages]);
  const contextLimit = contextWindow && contextWindow > 0 ? contextWindow : null;
  if (!usage || !contextLimit) return null;

  const usedPercent = Math.min((usage.totalTokens / contextLimit) * 100, 100);
  const cachedPercent =
    usage.inputTokens > 0 ? Math.min((usage.cacheReadTokens / usage.inputTokens) * 100, 100) : 0;

  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <button
          type="button"
          className="token-usage-trigger"
          aria-label={t("Token Usage")}
          title={t("Token Usage")}
        >
          <span className="token-usage-ring" aria-hidden="true">
            <svg viewBox="0 0 36 36">
              <circle className="token-usage-ring-track" cx="18" cy="18" r="15.5" />
              <circle
                className="token-usage-ring-value"
                cx="18"
                cy="18"
                r="15.5"
                pathLength="100"
                strokeDasharray="100 100"
                strokeDashoffset={100 - usedPercent}
              />
            </svg>
          </span>
        </button>
      </Popover.Trigger>
      <Popover.Content className="token-usage-popover" side="top" align="end">
        <div className="token-usage-popover-heading">
          <strong>{t("Token Usage")}</strong>
          <span>
            {formatTokenCount(usage.totalTokens)} / {formatTokenCount(contextLimit)} (
            {Math.round(usedPercent)}% {t("Used")})
          </span>
        </div>
        <dl className="token-usage-details">
          <div>
            <dt>{t("Total Tokens")}</dt>
            <dd>{formatTokenCount(usage.totalTokens)}</dd>
          </div>
          <div>
            <dt>{t("Total input")}</dt>
            <dd>
              {formatTokenCount(usage.inputTokens)} ({Math.round(cachedPercent)}% {t("Cached")})
            </dd>
          </div>
          <div>
            <dt>{t("Total output")}</dt>
            <dd>{formatTokenCount(usage.outputTokens)}</dd>
          </div>
        </dl>
      </Popover.Content>
    </Popover.Root>
  );
}
