import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type UIEvent,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  Check,
  ChevronRight,
  CircleAlert,
  CircleStop,
  GitFork,
  LoaderCircle,
  Sparkles,
  Undo2,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import type { MessageRecord, Model, Session } from "../../types/api";
import type { CompactionNotice, InstructionNotice, StreamMessage } from "../../types/chat";
import { ProviderErrorCard } from "./ApprovalCards";
import {
  buildChatItems,
  estimateChatItemSize,
  type AssistantStatus,
  type ChatItem,
  type SegmentItem,
  turnContentId,
} from "./chatItems";
export { ApprovalCard } from "./ApprovalCards";
export { buildChatItems } from "./chatItems";
import { ChatScrollContext } from "./ChatScrollContext";
import { MessageImageGallery } from "./ImageAttachments";
import { CopyButton } from "../ui/CopyButton";
import { ExpandableBody } from "../ui/ExpandableBody";
import { InstructionMessage, SystemMessageBlock } from "../renderers/SystemMessageBlock";
import { ThinkingBlock } from "../renderers/ThinkingBlock";
import { ToolCallRow } from "../renderers/ToolCallRow";
import { MarkdownRenderer } from "../renderers/MarkdownRenderer";
import { ActivityRipple } from "../renderers/ActivityRipple";
import { Button, IconButton } from "../ui";
import { buildRounds, type Round } from "../../utils/round";
import { formatDurationHuman, formatTime, stripSystemReminderTags } from "../../utils/format";

export interface MessageListProps {
  messages: MessageRecord[];
  streams: StreamMessage[];
  instructionNotices?: InstructionNotice[];
  compactionNotice?: CompactionNotice | null;
  sessionId?: string;
  session?: Session;
  models?: Model[];
  workspaceRoot?: string;
  onRevert?: (messageId: string) => void;
  onFork?: (messageId: string) => void;
  onRetryProviderError?: (messageId: string) => void;
  scrollToBottomRequest?: number;
}

const CHAT_SCROLL_BOTTOM_THRESHOLD = 32;

function getChatItemKey(item: ChatItem | undefined, index: number) {
  if (!item) return index;
  return item.kind === "assistant-segment" ? item.item.key : item.key;
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
        {content ? <div className="user-message-bubble">{content}</div> : null}
        <MessageImageGallery attachments={round.userMessage.attachments} />
        <div className="user-message-meta">
          {userTime ? <time>{userTime}</time> : null}
          <CopyButton content={content} />
          <span className="user-message-actions">
            {onRevert ? (
              <IconButton
                label={t("Revert to this message (undo later messages)")}
                size="sm"
                className="message-action"
                onClick={() => onRevert(round.userMessage.id)}
                title={t("Revert to this message (undo later messages)")}
              >
                <Undo2 size={16} />
              </IconButton>
            ) : null}
            {onFork ? (
              <IconButton
                label={t("Fork conversation from this message")}
                size="sm"
                className="message-action"
                onClick={() => onFork(round.userMessage.id)}
                title={t("Fork conversation from this message")}
              >
                <GitFork size={16} />
              </IconButton>
            ) : null}
          </span>
        </div>
      </div>
    </article>
  );
}

function WorkDuration({
  startedAt,
  completedAt,
  active,
}: {
  startedAt?: string;
  completedAt?: string;
  active: boolean;
}) {
  const { t } = useTranslation();
  const start = startedAt ? Date.parse(startedAt) : Number.NaN;
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
    const update = () => setLiveElapsedMs(Math.max(0, Date.now() - start));
    update();
    const timer = setInterval(update, 500);
    return () => clearInterval(timer);
  }, [active, completedAt, start]);

  const elapsedMs = fixedElapsedMs ?? liveElapsedMs;
  if (elapsedMs === null || Number.isNaN(elapsedMs)) {
    return <span>{active ? t("Working…") : t("Assistant")}</span>;
  }

  const duration = formatDurationHuman(elapsedMs, t, !active);
  return <span>{t("Worked for {{duration}}", { duration })}</span>;
}

function InterruptionDuration({
  startedAt,
  completedAt,
}: {
  startedAt?: string;
  completedAt?: string;
}) {
  const { t } = useTranslation();
  const start = startedAt ? Date.parse(startedAt) : Number.NaN;
  const completed = completedAt ? Date.parse(completedAt) : Number.NaN;
  if (Number.isNaN(start) || Number.isNaN(completed)) {
    return <span>{t("You interrupted the response.")}</span>;
  }
  const duration = formatDurationHuman(Math.max(0, completed - start), t, true);
  return <span>{t("You interrupted after {{duration}}", { duration })}</span>;
}

function AssistantMetaItem({
  turnId,
  status,
  active,
  startedAt,
  completedAt,
  expanded,
  collapsible,
  onToggle,
}: {
  turnId: string;
  status: AssistantStatus;
  active: boolean;
  startedAt?: string;
  completedAt?: string;
  expanded: boolean;
  collapsible: boolean;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const interruptionContent = status === "failed" ? t("Response failed") : null;
  const hasFailure = status === "failed";

  const content = (
    <>
      <span className={hasFailure ? "assistant-turn-status failed" : undefined}>
        {interruptionContent ? (
          interruptionContent
        ) : (
          <ActivityRipple active={active}>
            <WorkDuration startedAt={startedAt} completedAt={completedAt} active={active} />
          </ActivityRipple>
        )}
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
          <Button
            type="button"
            className="assistant-turn-meta is-collapsible"
            onClick={onToggle}
            aria-expanded={expanded}
            aria-controls={turnContentId(turnId)}
            aria-label={expanded ? t("Collapse previous messages") : t("Expand previous messages")}
            variant="ghost"
            size="sm"
          >
            {content}
          </Button>
        ) : (
          <div className="assistant-turn-meta">{content}</div>
        )}
      </div>
    </article>
  );
}

function InterruptionNotice({
  status,
  startedAt,
  completedAt,
}: {
  status: "cancelled" | "interrupted";
  startedAt?: string;
  completedAt?: string;
}) {
  const { t } = useTranslation();
  const content =
    status === "cancelled" ? (
      <InterruptionDuration startedAt={startedAt} completedAt={completedAt} />
    ) : (
      t("Response interrupted")
    );

  return (
    <article className="chat-message assistant-message assistant-segment-row assistant-interruption-row">
      <div className="assistant-message-inner">
        <div className="assistant-interruption-notice">
          <CircleStop className="assistant-interruption-icon" size={14} aria-hidden="true" />
          <span>{content}</span>
        </div>
      </div>
    </article>
  );
}

function CompactionMessage({ notice }: { notice: CompactionNotice }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const hasSummary = notice.status === "complete" && Boolean(notice.summary);
  const title =
    notice.status === "running"
      ? notice.manual
        ? t("Compacting context")
        : t("Automatically compacting context")
      : notice.status === "complete"
        ? notice.manual
          ? t("Context compacted")
          : t("Context automatically compacted")
        : t("Context compaction failed");
  const bodyId = "context-compaction-summary";

  return (
    <article className="chat-message assistant-message assistant-segment-row compaction-message">
      <Button
        type="button"
        className="compaction-message-header"
        onClick={() => hasSummary && setExpanded((current) => !current)}
        aria-expanded={hasSummary ? expanded : undefined}
        aria-controls={hasSummary ? bodyId : undefined}
        variant="ghost"
        size="sm"
        disabled={!hasSummary}
      >
        {notice.status === "running" ? (
          <LoaderCircle className="spin" size={14} />
        ) : notice.status === "complete" ? (
          <Check size={14} />
        ) : (
          <CircleAlert size={14} />
        )}
        <strong>{title}</strong>
        {hasSummary ? (
          <ChevronRight
            className={`compaction-message-chevron${expanded ? " expanded" : ""}`}
            size={16}
            aria-hidden="true"
          />
        ) : null}
      </Button>
      {notice.status === "failed" && notice.error ? (
        <div className="compaction-message-error">{notice.error}</div>
      ) : null}
      <ExpandableBody expanded={expanded} className="compaction-message-body-shell">
        <div id={bodyId} className="compaction-message-body">
          {notice.summary ? <MarkdownRenderer content={notice.summary} /> : null}
        </div>
      </ExpandableBody>
    </article>
  );
}

function renderSegment(
  item: SegmentItem,
  workspaceRoot: string,
  sessionId: string | undefined,
  expanded: boolean,
  onExpandedChange: (expanded: boolean) => void,
): ReactNode {
  const { segment, entry } = item;
  if (segment.type === "instruction") {
    return (
      <InstructionMessage
        message={segment.message}
        content={item.instructionContent}
        sessionId={sessionId}
        expanded={expanded}
        onExpandedChange={onExpandedChange}
      />
    );
  }
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
  sessionId,
  expanded,
  collapsed,
  onExpandedChange,
}: {
  item: SegmentItem;
  workspaceRoot: string;
  sessionId?: string;
  expanded: boolean;
  collapsed: boolean;
  onExpandedChange: (detailKey: string, expanded: boolean) => void;
}) {
  const content = renderSegment(item, workspaceRoot, sessionId, expanded, (next) => {
    onExpandedChange(item.key, next);
  });
  if (!content) return null;
  const className = [
    "chat-message",
    "assistant-message",
    "assistant-segment-row",
    collapsed ? "is-collapsed" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <article className={className} id={item.contentId}>
      <ExpandableBody expanded={!collapsed} className="assistant-segment-expandable">
        <div className="assistant-message-inner">
          <div className="assistant-message-content message-content">
            <div className="chat-segment-content">{content}</div>
          </div>
        </div>
      </ExpandableBody>
    </article>
  );
});

export const MessageList = memo(function MessageList({
  messages,
  streams,
  instructionNotices = [],
  compactionNotice = null,
  sessionId,
  session,
  models = [],
  workspaceRoot = "",
  onRevert,
  onFork,
  onRetryProviderError,
  scrollToBottomRequest = 0,
}: MessageListProps) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [expandedTurns, setExpandedTurns] = useState<Record<string, boolean>>({});
  const [expandedDetails, setExpandedDetails] = useState<Record<string, boolean>>({});

  const rounds = useMemo(() => buildRounds(messages), [messages]);

  const items = useMemo(
    () =>
      buildChatItems(
        rounds,
        streams,
        expandedTurns,
        instructionNotices,
        workspaceRoot,
        models,
        session,
        t,
        compactionNotice,
      ),
    [
      rounds,
      streams,
      expandedTurns,
      instructionNotices,
      workspaceRoot,
      models,
      session,
      t,
      compactionNotice,
    ],
  );

  const followTailRef = useRef(true);
  const scrollFrameRef = useRef<number | null>(null);
  const previousSessionIdRef = useRef<string | null>(null);
  const previousScrollToBottomRequestRef = useRef(scrollToBottomRequest);

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => estimateChatItemSize(items[index]),
    overscan: 6,
    getItemKey: (index) => getChatItemKey(items[index], index),
    useFlushSync: false,
    anchorTo: "end",
    followOnAppend: true,
    scrollEndThreshold: CHAT_SCROLL_BOTTOM_THRESHOLD,
  });
  virtualizer.shouldAdjustScrollPositionOnItemSizeChange = (item, _delta, instance) => {
    // The end anchor handles size changes while following the tail.
    if (followTailRef.current) return false;
    // When the user is reading history above, only compensate for size changes
    // of elements strictly above the fold to keep the visible viewport steady.
    const offset = instance.scrollOffset ?? 0;
    return item.start + item.size <= offset;
  };

  const handleScroll = useCallback((event: UIEvent<HTMLDivElement>) => {
    // Virtualizer corrections during measurement also dispatch scroll events.
    // They must not turn off tail-following while a round is collapsing.
    if (!event.nativeEvent.isTrusted) return;
    const element = event.currentTarget;
    followTailRef.current =
      element.scrollHeight - element.scrollTop - element.clientHeight <=
      CHAT_SCROLL_BOTTOM_THRESHOLD;
  }, []);

  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    const sessionChanged = previousSessionIdRef.current !== sessionId;
    const scrollWasRequested = previousScrollToBottomRequestRef.current !== scrollToBottomRequest;
    previousSessionIdRef.current = sessionId ?? null;
    previousScrollToBottomRequestRef.current = scrollToBottomRequest;
    if (!followTailRef.current && !scrollWasRequested && !sessionChanged) return;

    if (scrollWasRequested || sessionChanged) followTailRef.current = true;
    if (!followTailRef.current) return;

    if (scrollFrameRef.current !== null) {
      window.cancelAnimationFrame(scrollFrameRef.current);
    }
    scrollFrameRef.current = window.requestAnimationFrame(() => {
      scrollFrameRef.current = null;
      const current = scrollRef.current;
      if (!current || !followTailRef.current) return;
      current.scrollTop = Math.max(0, current.scrollHeight - current.clientHeight);
    });

    return () => {
      if (scrollFrameRef.current !== null) {
        window.cancelAnimationFrame(scrollFrameRef.current);
        scrollFrameRef.current = null;
      }
    };
  }, [items, scrollToBottomRequest, sessionId]);

  function toggleTurn(turnId: string, expanded: boolean) {
    setExpandedTurns((current) => ({ ...current, [turnId]: !expanded }));
  }

  const toggleDetail = useCallback((detailKey: string, expanded: boolean) => {
    setExpandedDetails((current) => ({ ...current, [detailKey]: expanded }));
  }, []);

  function renderItem(item: ChatItem): ReactNode {
    switch (item.kind) {
      case "user":
        return <UserMessageItem round={item.round} onRevert={onRevert} onFork={onFork} />;
      case "assistant-meta":
        return (
          <AssistantMetaItem
            turnId={item.turnId}
            status={item.status}
            active={item.active}
            startedAt={item.startedAt}
            completedAt={item.completedAt}
            expanded={item.expanded}
            collapsible={item.collapsible}
            onToggle={() => toggleTurn(item.turnId, item.expanded)}
          />
        );
      case "assistant-segment":
        return (
          <SegmentItemView
            item={item.item}
            workspaceRoot={workspaceRoot}
            sessionId={sessionId}
            expanded={expandedDetails[item.item.key] ?? false}
            collapsed={item.item.collapsed}
            onExpandedChange={toggleDetail}
          />
        );
      case "round-footer":
        return (
          <article className="chat-message assistant-message assistant-footer-row">
            <div className="assistant-message-inner">
              <div className="assistant-message-content message-content">
                <div className="round-footer">
                  <span>{item.footerParts.join(" · ")}</span>
                  {item.content ? <CopyButton content={item.content} /> : null}
                </div>
              </div>
            </div>
          </article>
        );
      case "interruption-notice":
        return (
          <InterruptionNotice
            status={item.status}
            startedAt={item.startedAt}
            completedAt={item.completedAt}
          />
        );
      case "system":
        return <SystemMessageBlock message={item.block.message} sessionId={sessionId} />;
      case "compaction":
        return <CompactionMessage notice={item.notice} />;
      case "stream-empty":
        return (
          <article className="chat-message assistant-message assistant-segment-row">
            <div className="assistant-message-inner">
              <div className="assistant-message-content message-content">
                <div className="stream-waiting">
                  <ActivityRipple active row label={t("Waiting for response…")}>
                    <span className="stream-waiting-text">{t("Waiting for response…")}</span>
                  </ActivityRipple>
                </div>
              </div>
            </div>
          </article>
        );
      case "stream-error":
        return <div className="stream-error">{item.message}</div>;
      case "provider-error":
        return (
          <ProviderErrorCard
            error={item.error}
            messageId={item.messageId}
            retrying={item.retrying}
            onRetry={onRetryProviderError}
          />
        );
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
    <div className="message-scroll" ref={scrollRef} onScroll={handleScroll}>
      <ChatScrollContext.Provider value={scrollRef}>
        <div
          className="message-virtual-canvas"
          style={{ height: `${virtualizer.getTotalSize()}px` }}
        >
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
