import { useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Check, ChevronRight, GitFork, Sparkles, Undo2, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { ApprovedTool, FrontendRequest, MessageRecord, ToolCall } from "../../types/api";
import type { StreamMessage } from "../../types/chat";
import { CopyButton } from "../ui/CopyButton";
import { InstructionMessage, SystemMessageBlock } from "../renderers/SystemMessageBlock";
import { ThinkingBlock } from "../renderers/ThinkingBlock";
import { ToolCallRow } from "../renderers/ToolCallRow";
import { MarkdownRenderer } from "../renderers/MarkdownRenderer";
import {
  buildRounds,
  getRoundPreviewIndex,
  isRoundCollapsible,
  type Round,
  type RoundSegment,
  type SystemMessageBlock as SystemMessageBlockData,
  type ToolCallEntry,
} from "../../utils/round";
import { formatTime, getDuration, stripSystemReminderTags } from "../../utils/format";

export interface MessageListProps {
  messages: MessageRecord[];
  streams: StreamMessage[];
  workspaceRoot?: string;
  onRevert?: (messageId: string) => void;
  onFork?: (messageId: string) => void;
}

export function MessageList({
  messages,
  streams,
  workspaceRoot = "",
  onRevert,
  onFork,
}: MessageListProps) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [expandedRounds, setExpandedRounds] = useState<Record<string, boolean>>({});
  const [expandedStreams, setExpandedStreams] = useState<Record<string, boolean>>({});
  const rounds = useMemo(() => buildRounds(messages), [messages]);
  type Row =
    | { type: "round"; key: string; round: Round }
    | { type: "system"; key: string; block: SystemMessageBlockData }
    | { type: "stream"; key: string; stream: StreamMessage };

  const rows = useMemo<Row[]>(() => {
    const base: Row[] = rounds.map((item) => {
      if ((item as SystemMessageBlockData).kind === "system") {
        const block = item as SystemMessageBlockData;
        return { type: "system", key: block.id, block };
      }
      const round = item as Round;
      return { type: "round", key: round.id, round };
    });
    return base.concat(streams.map((stream) => ({ type: "stream", key: stream.key, stream })));
  }, [rounds, streams]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 160,
    overscan: 8,
    getItemKey: (index) => rows[index]?.key ?? index,
  });

  function toggleRound(roundId: string) {
    setExpandedRounds((current) => ({
      ...current,
      [roundId]: !(current[roundId] ?? false),
    }));
  }

  function toggleStream(streamKey: string) {
    setExpandedStreams((current) => ({
      ...current,
      [streamKey]: !(current[streamKey] ?? false),
    }));
  }

  if (rows.length === 0) {
    return (
      <div className="welcome-state">
        <div className="welcome-icon">
          <Sparkles size={21} />
        </div>
        <h2>{t("What are we building?")}</h2>
        <p>
          {t(
            "Start a conversation with the local tidev runtime. Your messages and streamed responses are persisted in SQLite.",
          )}
        </p>
      </div>
    );
  }

  return (
    <div className="message-scroll" ref={scrollRef}>
      <div className="message-virtual-canvas" style={{ height: `${virtualizer.getTotalSize()}px` }}>
        {virtualizer.getVirtualItems().map((item) => {
          const row = rows[item.index];
          return (
            <div
              className="message-row"
              data-index={item.index}
              key={item.key}
              ref={virtualizer.measureElement}
              style={{ transform: `translateY(${item.start}px)` }}
            >
              {row.type === "round" ? (
                <RoundView
                  round={row.round}
                  workspaceRoot={workspaceRoot}
                  expanded={expandedRounds[row.round.id] ?? !isRoundCollapsible(row.round)}
                  onToggle={() => toggleRound(row.round.id)}
                  onRevert={onRevert}
                  onFork={onFork}
                />
              ) : row.type === "system" ? (
                <SystemMessageBlock message={row.block.message} />
              ) : (
                <StreamBubble
                  stream={row.stream}
                  workspaceRoot={workspaceRoot}
                  expanded={expandedStreams[row.stream.key] ?? row.stream.status === "streaming"}
                  onToggle={() => toggleStream(row.stream.key)}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function RoundView({
  round,
  workspaceRoot,
  expanded,
  onToggle,
  onRevert,
  onFork,
}: {
  round: Round;
  workspaceRoot: string;
  expanded: boolean;
  onToggle: () => void;
  onRevert?: (messageId: string) => void;
  onFork?: (messageId: string) => void;
}) {
  const { t } = useTranslation();
  const userTime = round.userMessage.created_at ? formatTime(round.userMessage.created_at) : "";
  const duration = round.completedAt
    ? getDuration(round.userMessage.created_at ?? "", round.completedAt)
    : null;
  const footerParts: string[] = [];
  if (round.modelName) footerParts.push(round.modelName);
  if (round.completedAt) footerParts.push(formatTime(round.completedAt));
  else if (round.status === "streaming" && userTime) footerParts.push(userTime);

  const hasAssistant =
    round.segments.length > 0 || round.status !== "user_only" || round.interrupted;
  const previewIndex = getRoundPreviewIndex(round);
  const isCollapsible = isRoundCollapsible(round);
  const showAllSegments = !isCollapsible || expanded;
  const interruptionLabel = round.interrupted
    ? round.interruptionKind === "cancelled"
      ? t("Stopped")
      : round.interruptionKind === "failed"
        ? t("Response failed")
        : t("Response interrupted")
    : null;
  const showTurnMeta =
    Boolean(duration) ||
    round.status === "streaming" ||
    isCollapsible ||
    interruptionLabel !== null;
  const turnMetaLabel = interruptionLabel
    ? interruptionLabel
    : duration
      ? t("Elapsed {{duration}}", { duration })
      : round.status === "streaming"
        ? t("streaming")
        : t("Assistant");
  const turnMetaContent = (
    <>
      <span className={interruptionLabel ? "assistant-turn-status interrupted" : undefined}>
        {turnMetaLabel}
      </span>
      {isCollapsible ? (
        <ChevronRight
          className={`assistant-turn-chevron${expanded ? " expanded" : ""}`}
          size={18}
          aria-hidden="true"
        />
      ) : null}
    </>
  );
  return (
    <div className="round-group">
      <article className="chat-message user-message">
        <div className="user-message-inner">
          <div className="user-message-bubble">
            {stripSystemReminderTags(round.userMessage.content)}
          </div>
          <div className="user-message-meta">
            {userTime ? <time>{userTime}</time> : null}
            <CopyButton content={stripSystemReminderTags(round.userMessage.content)} />
            <span className="user-message-actions">
              {onRevert ? (
                <button
                  className="message-action"
                  onClick={() => onRevert(round.userMessage.id)}
                  title={t("Revert to this message (undo later messages)")}
                  aria-label={t("Revert to this message (undo later messages)")}
                >
                  <Undo2 size={16} />
                </button>
              ) : null}
              {onFork ? (
                <button
                  className="message-action"
                  onClick={() => onFork(round.userMessage.id)}
                  title={t("Fork conversation from this message")}
                  aria-label={t("Fork conversation from this message")}
                >
                  <GitFork size={16} />
                </button>
              ) : null}
            </span>
          </div>
        </div>
      </article>
      {round.leadingInstructions.map((message) => (
        <div className="round-instruction" key={message.id}>
          <InstructionMessage message={message} />
        </div>
      ))}
      {hasAssistant ? (
        <article className="chat-message assistant-message">
          <div className="assistant-message-inner">
            {showTurnMeta ? (
              isCollapsible ? (
                <button
                  type="button"
                  className="assistant-turn-meta is-collapsible"
                  onClick={onToggle}
                  aria-expanded={expanded}
                  aria-controls={`assistant-content-${round.id}`}
                  aria-label={
                    expanded ? t("Collapse previous messages") : t("Expand previous messages")
                  }
                >
                  {turnMetaContent}
                </button>
              ) : (
                <div className="assistant-turn-meta">{turnMetaContent}</div>
              )
            ) : null}
            <div
              className="assistant-message-content message-content"
              id={`assistant-content-${round.id}`}
            >
              <AssistantSegments
                segments={round.segments}
                toolCallMap={round.toolCallMap}
                workspaceRoot={workspaceRoot}
                showAllSegments={showAllSegments}
                previewIndex={previewIndex}
                active={round.status === "streaming"}
                reasoningStartedAt={round.reasoningStartedAt}
                reasoningCompletedAt={round.reasoningCompletedAt ?? round.completedAt}
              />
              {round.status === "streaming" && round.segments.length === 0 ? (
                <span className="cursor-block" />
              ) : null}
              {round.status === "complete" && !round.interrupted && footerParts.length ? (
                <div className="round-footer">{footerParts.join(" · ")}</div>
              ) : null}
            </div>
          </div>
        </article>
      ) : null}
    </div>
  );
}

function StreamBubble({
  stream,
  workspaceRoot,
  expanded,
  onToggle,
}: {
  stream: StreamMessage;
  workspaceRoot: string;
  expanded: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const isStreaming = stream.status === "streaming";
  const previewIndex = stream.segments.findLastIndex(
    (segment) => segment.type === "text" && segment.content.trim(),
  );
  const isCollapsible = !isStreaming && stream.segments.length > 1;
  const showAllSegments = isStreaming || expanded;
  const statusLabel = isStreaming
    ? t("streaming")
    : stream.status === "failed"
      ? t("Response failed")
      : t("Response interrupted");
  const contentId = `stream-content-${stream.key.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  return (
    <article className="chat-message assistant-message streaming-message">
      <div className="assistant-message-inner">
        {isCollapsible ? (
          <button
            type="button"
            className="assistant-turn-meta is-collapsible"
            onClick={onToggle}
            aria-expanded={expanded}
            aria-controls={contentId}
            aria-label={expanded ? t("Collapse previous messages") : t("Expand previous messages")}
          >
            <span className="assistant-turn-status interrupted">{statusLabel}</span>
            <ChevronRight
              className={`assistant-turn-chevron${expanded ? " expanded" : ""}`}
              size={18}
              aria-hidden="true"
            />
          </button>
        ) : (
          <div className="assistant-turn-meta">
            <span className={isStreaming ? undefined : "assistant-turn-status interrupted"}>
              {statusLabel}
            </span>
          </div>
        )}
        <div className="assistant-message-content message-content" id={contentId}>
          <AssistantSegments
            segments={stream.segments}
            toolCallMap={stream.toolCallMap}
            workspaceRoot={workspaceRoot}
            showAllSegments={showAllSegments}
            previewIndex={previewIndex >= 0 ? previewIndex : null}
            active={isStreaming}
            reasoningStartedAt={stream.reasoningStartedAt}
            reasoningCompletedAt={stream.reasoningCompletedAt}
          />
          {isStreaming && stream.segments.length === 0 ? <span className="cursor-block" /> : null}
          {!isStreaming ? (
            <div className="stream-error">{stream.error ?? t("Response failed")}</div>
          ) : null}
        </div>
      </div>
    </article>
  );
}

function AssistantSegments({
  segments,
  toolCallMap,
  workspaceRoot,
  showAllSegments,
  previewIndex,
  active,
  reasoningStartedAt,
  reasoningCompletedAt,
}: {
  segments: RoundSegment[];
  toolCallMap: Record<string, ToolCallEntry>;
  workspaceRoot: string;
  showAllSegments: boolean;
  previewIndex: number | null;
  active: boolean;
  reasoningStartedAt?: string | null;
  reasoningCompletedAt?: string | null;
}) {
  return (
    <div className="tool-list">
      {segments.map((segment, index) => {
        if (segment.type === "instruction") {
          return (
            <InstructionMessage
              key={`instruction-${segment.message.id}`}
              message={segment.message}
            />
          );
        }
        if (!showAllSegments && index !== previewIndex) return null;
        if (segment.type === "reasoning" && segment.content) {
          return (
            <ThinkingBlock
              key={`thinking-${index}`}
              content={segment.content}
              active={active}
              startedAt={segment.startedAt ?? reasoningStartedAt ?? undefined}
              completedAt={segment.completedAt ?? reasoningCompletedAt ?? undefined}
            />
          );
        }
        if (segment.type === "text" && segment.content) {
          return (
            <MarkdownRenderer
              key={`text-${index}`}
              content={stripSystemReminderTags(segment.content)}
            />
          );
        }
        if (segment.type === "tool_call") {
          const entry = toolCallMap[segment.toolCallId];
          if (!entry) return null;
          return (
            <ToolCallRow key={segment.toolCallId} entry={entry} workspaceRoot={workspaceRoot} />
          );
        }
        return null;
      })}
    </div>
  );
}

function makeRejectedTool(tool: ToolCall): ApprovedTool {
  return {
    tool_call: tool,
    rejection: {
      output: "The user rejected this tool call.",
      attachments: [],
      metadata: {},
    },
    child_session_id: null,
    allow_outside: false,
    sensitive_file_approved: false,
    user_reason: "Rejected in Web UI",
  };
}

function makeApprovedTool(tool: ToolCall): ApprovedTool {
  return {
    tool_call: tool,
    rejection: null,
    child_session_id: null,
    allow_outside: true,
    sensitive_file_approved: true,
    user_reason: null,
  };
}

export function ApprovalCard({
  request,
  onRespond,
}: {
  request: FrontendRequest;
  onRespond: (tools: ApprovedTool[]) => void;
}) {
  const { t } = useTranslation();
  const tools = request.kind.ToolApproval ?? [];
  return (
    <div className="approval-card">
      <div className="approval-heading">
        <span>
          <Sparkles size={16} /> {t("Approval required")}
        </span>
        <span className="approval-session">{request.session_id.slice(0, 8)}</span>
      </div>
      <p>
        {t("tidev is waiting for permission to run {{countLabel}}.", {
          countLabel:
            tools.length === 1 ? t("a tool") : t("{{count}} tools", { count: tools.length }),
        })}
      </p>
      <div className="approval-tools">
        {tools.map((item) => (
          <div className="approval-tool" key={item.tool_call.id}>
            <strong>{item.tool_call.name}</strong>
            <code>{item.tool_call.arguments}</code>
          </div>
        ))}
      </div>
      <div className="approval-actions">
        <button
          className="secondary-button"
          onClick={() => onRespond(tools.map((item) => makeRejectedTool(item.tool_call)))}
        >
          <X size={15} />
          {t("Reject")}
        </button>
        <button
          className="primary-button"
          onClick={() => onRespond(tools.map((item) => makeApprovedTool(item.tool_call)))}
        >
          <Check size={15} />
          {t("Allow")}
        </button>
      </div>
    </div>
  );
}
