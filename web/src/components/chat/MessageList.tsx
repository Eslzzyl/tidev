import {
  memo,
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Check, ChevronRight, GitFork, Sparkles, Undo2, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import type {
  ApprovedTool,
  FrontendRequest,
  Message,
  MessageRecord,
  ToolCall,
} from "../../types/api";
import type { StreamMessage } from "../../types/chat";
import { ChatScrollContext } from "./ChatScrollContext";
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
import {
  beginPerformance,
  endPerformance,
  schedulePerformanceFrame,
  type PerformanceSpan,
} from "../../utils/performance";

export interface MessageListProps {
  messages: MessageRecord[];
  streams: StreamMessage[];
  workspaceRoot?: string;
  onRevert?: (messageId: string) => void;
  onFork?: (messageId: string) => void;
}

interface ExpansionTrace {
  roundId: string;
  details: {
    segmentCount: number;
    toolCallCount: number;
  };
  commit: PerformanceSpan | null;
  firstFrame: PerformanceSpan | null;
  contentCommit: PerformanceSpan | null;
  contentFirstFrame: PerformanceSpan | null;
}

interface SegmentItem {
  key: string;
  contentId?: string;
  segment: RoundSegment;
  entry?: ToolCallEntry;
  active: boolean;
  reasoningStartedAt?: string | null;
  reasoningCompletedAt?: string | null;
}

type ChatItem =
  | { kind: "user"; key: string; round: Round }
  | { kind: "instruction"; key: string; message: Message }
  | {
      kind: "round-meta";
      key: string;
      round: Round;
      expanded: boolean;
      collapsible: boolean;
    }
  | { kind: "round-segment"; item: SegmentItem }
  | { kind: "round-footer"; key: string; footerParts: string[] }
  | { kind: "system"; key: string; block: SystemMessageBlockData }
  | {
      kind: "stream-meta";
      key: string;
      stream: StreamMessage;
      expanded: boolean;
      collapsible: boolean;
    }
  | { kind: "stream-segment"; item: SegmentItem }
  | { kind: "stream-empty"; key: string }
  | { kind: "stream-error"; key: string; message: string };

function isSystemBlock(item: Round | SystemMessageBlockData): item is SystemMessageBlockData {
  return "kind" in item && item.kind === "system";
}

function hasAssistant(round: Round) {
  return round.segments.length > 0 || round.status !== "user_only" || round.interrupted;
}

function roundIsExpanded(round: Round, expandedRounds: Record<string, boolean>) {
  return expandedRounds[round.id] ?? !isRoundCollapsible(round);
}

function makeSegmentItems(
  segments: RoundSegment[],
  toolCallMap: Record<string, ToolCallEntry>,
  showAll: boolean,
  previewIndex: number | null,
  active: boolean,
  reasoningStartedAt?: string | null,
  reasoningCompletedAt?: string | null,
  keyPrefix = "segment",
  contentId?: string,
): SegmentItem[] {
  const visible: SegmentItem[] = [];

  segments.forEach((segment, index) => {
    if (segment.type !== "instruction" && !showAll && index !== previewIndex) return;
    const entry = segment.type === "tool_call" ? toolCallMap[segment.toolCallId] : undefined;
    if (segment.type === "tool_call" && !entry) return;

    visible.push({
      key: `${keyPrefix}-${index}-${segment.type === "tool_call" ? segment.toolCallId : "content"}`,
      contentId: visible.length === 0 ? contentId : undefined,
      segment,
      entry,
      active,
      reasoningStartedAt,
      reasoningCompletedAt,
    });
  });

  return visible;
}

function buildChatItems(
  rounds: (Round | SystemMessageBlockData)[],
  streams: StreamMessage[],
  expandedRounds: Record<string, boolean>,
  expandedStreams: Record<string, boolean>,
): ChatItem[] {
  const items: ChatItem[] = [];

  for (const value of rounds) {
    if (isSystemBlock(value)) {
      items.push({ kind: "system", key: value.id, block: value });
      continue;
    }

    const round = value;
    const expanded = roundIsExpanded(round, expandedRounds);
    const collapsible = isRoundCollapsible(round);
    const previewIndex = getRoundPreviewIndex(round);
    const assistant = hasAssistant(round);

    items.push({ kind: "user", key: `${round.id}:user`, round });
    for (const message of round.leadingInstructions) {
      items.push({ kind: "instruction", key: `${round.id}:instruction:${message.id}`, message });
    }

    if (!assistant) continue;

    const interruptionLabel = round.interrupted
      ? round.interruptionKind === "cancelled"
        ? "Stopped"
        : round.interruptionKind === "failed"
          ? "Response failed"
          : "Response interrupted"
      : null;
    const duration = round.completedAt
      ? getDuration(round.userMessage.created_at ?? "", round.completedAt)
      : null;
    const showTurnMeta =
      Boolean(duration) ||
      round.status === "streaming" ||
      collapsible ||
      interruptionLabel !== null;

    if (showTurnMeta) {
      items.push({
        kind: "round-meta",
        key: `${round.id}:meta`,
        round,
        expanded,
        collapsible,
      });
    }

    const segments = makeSegmentItems(
      round.segments,
      round.toolCallMap,
      !collapsible || expanded,
      previewIndex,
      round.status === "streaming",
      round.reasoningStartedAt,
      round.reasoningCompletedAt ?? round.completedAt,
      `${round.id}:segment`,
      `assistant-content-${round.id}`,
    );
    for (const segment of segments) {
      items.push({ kind: "round-segment", item: segment });
    }

    const footerParts: string[] = [];
    if (round.modelName) footerParts.push(round.modelName);
    if (round.completedAt) footerParts.push(formatTime(round.completedAt));
    else if (round.status === "streaming" && round.userMessage.created_at) {
      footerParts.push(formatTime(round.userMessage.created_at));
    }
    if (round.status === "complete" && !round.interrupted && footerParts.length) {
      items.push({ kind: "round-footer", key: `${round.id}:footer`, footerParts });
    }
  }

  for (const stream of streams) {
    const isStreaming = stream.status === "streaming";
    const collapsible = !isStreaming && stream.segments.length > 1;
    const expanded = expandedStreams[stream.key] ?? isStreaming;
    const previewIndex = stream.segments.findLastIndex(
      (segment) => segment.type === "text" && segment.content.trim(),
    );

    items.push({
      kind: "stream-meta",
      key: `${stream.key}:meta`,
      stream,
      expanded,
      collapsible,
    });

    const segments = makeSegmentItems(
      stream.segments,
      stream.toolCallMap,
      isStreaming || expanded,
      previewIndex >= 0 ? previewIndex : null,
      isStreaming,
      stream.reasoningStartedAt,
      stream.reasoningCompletedAt,
      `${stream.key}:segment`,
      `stream-content-${stream.key.replace(/[^a-zA-Z0-9_-]/g, "-")}`,
    );
    for (const segment of segments) {
      items.push({ kind: "stream-segment", item: segment });
    }

    if (isStreaming && stream.segments.length === 0) {
      items.push({ kind: "stream-empty", key: `${stream.key}:empty` });
    } else if (!isStreaming) {
      items.push({
        kind: "stream-error",
        key: `${stream.key}:error`,
        message: stream.error ?? "Response failed",
      });
    }
  }

  return items;
}

function estimateChatItemSize(item: ChatItem | undefined) {
  if (!item) return 48;
  switch (item.kind) {
    case "user":
      return 118;
    case "instruction":
      return 28;
    case "round-meta":
    case "stream-meta":
      return 42;
    case "round-segment":
    case "stream-segment":
      switch (item.item.segment.type) {
        case "tool_call":
          return 34;
        case "reasoning":
          return 30;
        case "instruction":
          return 28;
        case "text":
          return 72;
      }
      break;
    case "round-footer":
      return 28;
    case "system":
      return 80;
    case "stream-empty":
      return 24;
    case "stream-error":
      return 36;
  }
}

function getChatItemKey(item: ChatItem | undefined, index: number) {
  if (!item) return index;
  return item.kind === "round-segment" || item.kind === "stream-segment"
    ? item.item.key
    : item.key;
}

function UserMessageItem({
  round,
  onRevert,
  onFork,
}: {
  round: Round;
  onRevert?: (messageId: string) => void;
  onFork?: (messageId: string) => void;
}) {
  const { t } = useTranslation();
  const content = stripSystemReminderTags(round.userMessage.content);
  const userTime = round.userMessage.created_at ? formatTime(round.userMessage.created_at) : "";

  return (
    <article className="chat-message user-message">
      <div className="user-message-inner">
        <div className="user-message-bubble">{content}</div>
        <div className="user-message-meta">
          {userTime ? <time>{userTime}</time> : null}
          <CopyButton content={content} />
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
  );
}

function RoundMetaItem({
  round,
  expanded,
  collapsible,
  onToggle,
}: {
  round: Round;
  expanded: boolean;
  collapsible: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const duration = round.completedAt
    ? getDuration(round.userMessage.created_at ?? "", round.completedAt)
    : null;
  const interruptionLabel = round.interrupted
    ? round.interruptionKind === "cancelled"
      ? t("Stopped")
      : round.interruptionKind === "failed"
        ? t("Response failed")
        : t("Response interrupted")
    : null;
  const label = interruptionLabel
    ? interruptionLabel
    : duration
      ? t("Elapsed {{duration}}", { duration })
      : round.status === "streaming"
        ? t("streaming")
        : t("Assistant");

  const content = (
    <>
      <span className={interruptionLabel ? "assistant-turn-status interrupted" : undefined}>
        {label}
      </span>
      {collapsible ? (
        <ChevronRight
          className={`assistant-turn-chevron${expanded ? " expanded" : ""}`}
          size={18}
          aria-hidden="true"
        />
      ) : null}
    </>
  );

  return (
    <article className="chat-message assistant-message assistant-meta-row">
      <div className="assistant-message-inner">
        {collapsible ? (
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
            {content}
          </button>
        ) : (
          <div className="assistant-turn-meta">{content}</div>
        )}
      </div>
    </article>
  );
}

function StreamMetaItem({
  stream,
  expanded,
  collapsible,
  onToggle,
}: {
  stream: StreamMessage;
  expanded: boolean;
  collapsible: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const isStreaming = stream.status === "streaming";
  const statusLabel = isStreaming
    ? t("streaming")
    : stream.status === "failed"
      ? t("Response failed")
      : t("Response interrupted");
  const contentId = `stream-content-${stream.key.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  const content = (
    <>
      <span className={isStreaming ? undefined : "assistant-turn-status interrupted"}>
        {statusLabel}
      </span>
      {collapsible ? (
        <ChevronRight
          className={`assistant-turn-chevron${expanded ? " expanded" : ""}`}
          size={18}
          aria-hidden="true"
        />
      ) : null}
    </>
  );

  return (
    <article className="chat-message assistant-message assistant-meta-row streaming-meta-row">
      <div className="assistant-message-inner">
        {collapsible ? (
          <button
            type="button"
            className="assistant-turn-meta is-collapsible"
            onClick={onToggle}
            aria-expanded={expanded}
            aria-controls={contentId}
            aria-label={expanded ? t("Collapse previous messages") : t("Expand previous messages")}
          >
            {content}
          </button>
        ) : (
          <div className="assistant-turn-meta">{content}</div>
        )}
      </div>
    </article>
  );
}

function renderSegment(
  item: SegmentItem,
  workspaceRoot: string,
  expanded: boolean,
  onExpandedChange: (expanded: boolean) => void,
): ReactNode {
  const { segment, entry } = item;
  if (segment.type === "instruction") return <InstructionMessage message={segment.message} />;
  if (segment.type === "reasoning" && segment.content) {
    return (
      <ThinkingBlock
        content={segment.content}
        active={item.active}
        expanded={expanded}
        onExpandedChange={onExpandedChange}
        startedAt={segment.startedAt ?? item.reasoningStartedAt ?? undefined}
        completedAt={segment.completedAt ?? item.reasoningCompletedAt ?? undefined}
      />
    );
  }
  if (segment.type === "text" && segment.content) {
    return <MarkdownRenderer content={stripSystemReminderTags(segment.content)} />;
  }
  if (segment.type === "tool_call" && entry) {
    return (
      <ToolCallRow
        entry={entry}
        workspaceRoot={workspaceRoot}
        expanded={expanded}
        onExpandedChange={onExpandedChange}
      />
    );
  }
  return null;
}

const SegmentItemView = memo(function SegmentItemView({
  item,
  workspaceRoot,
  stream,
  expanded,
  onExpandedChange,
}: {
  item: SegmentItem;
  workspaceRoot: string;
  stream?: boolean;
  expanded: boolean;
  onExpandedChange: (detailKey: string, expanded: boolean) => void;
}) {
  const content = renderSegment(item, workspaceRoot, expanded, (next) => {
    onExpandedChange(item.key, next);
  });
  if (!content) return null;
  const className = [
    "chat-message",
    "assistant-message",
    "assistant-segment-row",
    stream ? "streaming-segment-row" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <article className={className} id={item.contentId}>
      <div className="assistant-message-inner">
        <div className="assistant-message-content message-content">
          <div className="chat-segment-content">{content}</div>
        </div>
      </div>
    </article>
  );
});

export const MessageList = memo(function MessageList({
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
  const [expandedDetails, setExpandedDetails] = useState<Record<string, boolean>>({});
  const pendingExpansions = useRef(new Map<string, ExpansionTrace>());

  const rounds = useMemo(() => {
    const performanceSpan = beginPerformance("session.message-list.build-rounds", {
      messageCount: messages.length,
    });
    const result = buildRounds(messages);
    endPerformance(performanceSpan, { roundCount: result.length });
    return result;
  }, [messages]);

  const items = useMemo(
    () => buildChatItems(rounds, streams, expandedRounds, expandedStreams),
    [rounds, streams, expandedRounds, expandedStreams],
  );

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => estimateChatItemSize(items[index]),
    overscan: 6,
    getItemKey: (index) => getChatItemKey(items[index], index),
    useFlushSync: false,
  });

  useLayoutEffect(() => {
    const pending = [...pendingExpansions.current.values()].find((trace) => trace.commit);
    if (!pending) return;

    const details = pending.details;
    endPerformance(pending.commit, details);
    endPerformance(pending.contentCommit, details);
    pending.commit = null;
    pending.contentCommit = null;
    return schedulePerformanceFrame(() => {
      endPerformance(pending.firstFrame, details);
      endPerformance(pending.contentFirstFrame, details);
      pending.firstFrame = null;
      pending.contentFirstFrame = null;
      pendingExpansions.current.delete(pending.roundId);
    });
  }, [items]);

  function toggleRound(roundId: string) {
    const round = rounds.find((item): item is Round => !isSystemBlock(item) && item.id === roundId);
    const currentExpanded = expandedRounds[roundId] ?? (round ? !isRoundCollapsible(round) : false);
    if (!currentExpanded && round) {
      const details = {
        segmentCount: round.segments.length,
        toolCallCount: Object.keys(round.toolCallMap).length,
      };
      pendingExpansions.current.set(roundId, {
        roundId,
        details,
        commit: beginPerformance("chat.round-expand.commit", details),
        firstFrame: beginPerformance("chat.round-expand.first-frame", details),
        contentCommit: beginPerformance("chat.round-expand.content-commit", details),
        contentFirstFrame: beginPerformance("chat.round-expand.content-first-frame", details),
      });
    }
    setExpandedRounds((current) => ({
      ...current,
      [roundId]: !(
        current[roundId] ?? (round ? !isRoundCollapsible(round) : false)
      ),
    }));
  }

  function toggleStream(streamKey: string) {
    setExpandedStreams((current) => ({
      ...current,
      [streamKey]: !(current[streamKey] ?? false),
    }));
  }

  const toggleDetail = useCallback((detailKey: string, expanded: boolean) => {
    setExpandedDetails((current) => ({ ...current, [detailKey]: expanded }));
  }, []);

  function renderItem(item: ChatItem): ReactNode {
    switch (item.kind) {
      case "user":
        return <UserMessageItem round={item.round} onRevert={onRevert} onFork={onFork} />;
      case "instruction":
        return (
          <div className="round-instruction">
            <InstructionMessage message={item.message} />
          </div>
        );
      case "round-meta":
        return (
          <RoundMetaItem
            round={item.round}
            expanded={item.expanded}
            collapsible={item.collapsible}
            onToggle={() => toggleRound(item.round.id)}
          />
        );
      case "round-segment":
        return (
          <SegmentItemView
            item={item.item}
            workspaceRoot={workspaceRoot}
            expanded={expandedDetails[item.item.key] ?? false}
            onExpandedChange={toggleDetail}
          />
        );
      case "round-footer":
        return (
          <article className="chat-message assistant-message assistant-footer-row">
            <div className="assistant-message-inner">
              <div className="assistant-message-content message-content">
                <div className="round-footer">{item.footerParts.join(" · ")}</div>
              </div>
            </div>
          </article>
        );
      case "system":
        return <SystemMessageBlock message={item.block.message} />;
      case "stream-meta":
        return (
          <StreamMetaItem
            stream={item.stream}
            expanded={item.expanded}
            collapsible={item.collapsible}
            onToggle={() => toggleStream(item.stream.key)}
          />
        );
      case "stream-segment":
        return (
          <SegmentItemView
            item={item.item}
            workspaceRoot={workspaceRoot}
            stream
            expanded={expandedDetails[item.item.key] ?? false}
            onExpandedChange={toggleDetail}
          />
        );
      case "stream-empty":
        return (
          <article className="chat-message assistant-message assistant-segment-row">
            <div className="assistant-message-inner">
              <div className="assistant-message-content message-content">
                <span className="cursor-block" />
              </div>
            </div>
          </article>
        );
      case "stream-error":
        return <div className="stream-error">{item.message}</div>;
    }
  }

  if (items.length === 0) {
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
      <ChatScrollContext.Provider value={scrollRef}>
        <div className="message-virtual-canvas" style={{ height: `${virtualizer.getTotalSize()}px` }}>
          {virtualizer.getVirtualItems().map((virtualItem) => {
            const item = items[virtualItem.index];
            if (!item) return null;
            return (
              <div
                className="message-row"
                data-index={virtualItem.index}
                key={virtualItem.key}
                ref={virtualizer.measureElement}
                style={{ transform: `translateY(${virtualItem.start}px)` }}
              >
                {renderItem(item)}
              </div>
            );
          })}
        </div>
      </ChatScrollContext.Provider>
    </div>
  );
});

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
