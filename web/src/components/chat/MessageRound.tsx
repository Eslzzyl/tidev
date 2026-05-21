import { memo } from "react";
import type { Round } from "../../types/round";
import { MarkdownRenderer } from "../renderers/MarkdownRenderer";
import { ThinkingBlock } from "../renderers/ThinkingBlock";
import { ToolCallRow } from "../renderers/ToolCallRow";
import { CopyButton } from "../ui/CopyButton";
import { UndoButton } from "./UndoButton";
import { formatTime, getDuration } from "../../utils/format";

interface Props {
  round: Round;
  onUndoRequest?: (messageId: string) => void;
  canUndo?: boolean;
  /** Used to stagger entrance animations when the message list is remounted
   *  (e.g. switching tabs).  Passed from VirtualMessageList. */
  staggerIndex?: number;
}

export const MessageRound = memo(function MessageRound({ round, onUndoRequest, canUndo = true, staggerIndex }: Props) {
  function getFooterParts(): string[] {
    const parts: string[] = [];
    if (round.modelName) parts.push(round.modelName);
    const duration = round.completedAt
      ? getDuration(round.userMessage.created_at, round.completedAt)
      : null;
    if (duration) parts.push(duration);
    if (round.completedAt) {
      parts.push(formatTime(round.completedAt));
    }
    return parts;
  }

  function getAssistantContent(): string {
    return round.segments
      .filter((s) => s.type === "reasoning" || s.type === "text")
      .map((s) => s.content || "")
      .join("\n\n");
  }

  const footerParts = getFooterParts();
  const assistantContent = getAssistantContent();

  const handleUndo = () => {
    if (onUndoRequest) {
      onUndoRequest(round.userMessage.id);
    }
  };

  // Only show undo button for completed rounds with assistant response
  const showUndoButton =
    canUndo && round.status === "complete" && onUndoRequest;

  // Stagger entrance animation — messages animate in sequentially from oldest
  // (low index) to newest (high index), creating a visible cascade when the
  // chat panel is remounted (e.g. switching tabs).
  const staggerDelay = staggerIndex !== undefined
    ? `${Math.min(staggerIndex * 20, 500)}ms`
    : undefined;

  return (
    <div
      className="motion-safe:animate-slide-up-fade border-b border-neutral-100 dark:border-neutral-900"
      style={staggerDelay ? { animationDelay: staggerDelay } : undefined}
    >
      {/* User message */}
      <div className="group flex gap-3 px-4 py-4">
        <div className="flex-shrink-0">
          <div className="flex h-8 w-8 items-center justify-center rounded-full bg-neutral-900 text-xs font-medium text-white dark:bg-neutral-100 dark:text-neutral-900">
            U
          </div>
        </div>

        <div className="flex max-w-[85%] flex-col items-start">
          <div className="mb-1 flex items-center gap-2">
            <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400">
              You
            </span>
            <span className="text-xs text-neutral-400 dark:text-neutral-600">
              {formatTime(round.userMessage.created_at)}
            </span>
            <CopyButton content={round.userMessage.content} />
            {showUndoButton && <UndoButton onClick={handleUndo} />}
          </div>
          <div className="w-full rounded-2xl rounded-tl-sm bg-neutral-100 px-4 py-2.5 text-sm leading-relaxed text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100">
            <p className="whitespace-pre-wrap">{round.userMessage.content}</p>
          </div>
        </div>
      </div>

      {/* Assistant response */}
      {round.segments.length > 0 && (
        <div className="group flex gap-3 px-4 py-4">
          <div className="flex-shrink-0">
            <div className="flex h-8 w-8 items-center justify-center rounded-full bg-blue-600 text-xs font-medium text-white dark:bg-blue-500">
              A
            </div>
          </div>

          <div className="flex max-w-[85%] flex-1 flex-col items-start">
            <div className="mb-1 flex items-center gap-2">
              <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400">
                Assistant
              </span>
              {round.completedAt && (
                <span className="text-xs text-neutral-400 dark:text-neutral-600">
                  {formatTime(round.completedAt)}
                </span>
              )}
              {!round.completedAt && round.status === "streaming" && (
                <span className="text-xs text-neutral-400 dark:text-neutral-600">
                  {formatTime(round.userMessage.created_at)}
                </span>
              )}
              {round.status === "streaming" && (
                <span className="text-xs text-blue-500 dark:text-blue-400">
                  streaming...
                </span>
              )}
              {round.status === "complete" && assistantContent && (
                <CopyButton content={assistantContent} />
              )}
            </div>

            <div className="w-full rounded-2xl rounded-tl-sm bg-white px-4 py-2.5 text-sm leading-relaxed text-neutral-900 dark:bg-neutral-900 dark:text-neutral-100">
              {/* Ordered segments (reasoning inlined at correct position) */}
              {round.segments.map((segment, idx) => (
                <div key={idx} className="mb-2 last:mb-0">
                  {segment.type === "reasoning" && segment.content && (
                    <ThinkingBlock content={segment.content} />
                  )}
                  {segment.type === "text" && segment.content && (
                    <MarkdownRenderer content={segment.content} />
                  )}
                  {segment.type === "tool_call" &&
                    round.toolCallMap[segment.toolCallId] && (
                      <ToolCallRow
                        entry={round.toolCallMap[segment.toolCallId]}
                      />
                    )}
                </div>
              ))}

              {/* Streaming indicator */}
              {round.status === "streaming" && round.segments.length === 0 && (
                <div className="flex items-center gap-1.5 text-neutral-400">
                  <div className="h-2 w-2 animate-stream-dot rounded-full bg-neutral-400" />
                  <div
                    className="h-2 w-2 animate-stream-dot rounded-full bg-neutral-400"
                    style={{ animationDelay: "0.2s" }}
                  />
                  <div
                    className="h-2 w-2 animate-stream-dot rounded-full bg-neutral-400"
                    style={{ animationDelay: "0.4s" }}
                  />
                </div>
              )}
            </div>

            {/* Footer */}
            {round.status === "complete" && footerParts.length > 0 && (
              <div className="mt-0.5 text-xs text-neutral-400 dark:text-neutral-600">
                {footerParts.join(" · ")}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
});
