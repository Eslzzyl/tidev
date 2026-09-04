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
import type { TFunction } from "i18next";
import {
  Check,
  ChevronRight,
  CircleAlert,
  CircleStop,
  GitFork,
  LoaderCircle,
  Pencil,
  RefreshCw,
  Sparkles,
  Undo2,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import type {
  ApprovedTool,
  FrontendRequest,
  Message,
  MessageRecord,
  Model,
  ProviderErrorData,
  Session,
  ToolCall,
} from "../../types/api";
import type { InstructionNotice, StreamMessage } from "../../types/chat";
import {
  approvedPermissionTool,
  formatQuestionAnswers,
  invalidQuestionResult,
  isQuestionTool,
  parseQuestionArgs,
  questionResult,
  rejectedTool,
} from "./approval";
import { ChatScrollContext } from "./ChatScrollContext";
import { MessageImageGallery } from "./ImageAttachments";
import { CopyButton } from "../ui/CopyButton";
import { ExpandableBody } from "../ui/ExpandableBody";
import { InstructionMessage, SystemMessageBlock } from "../renderers/SystemMessageBlock";
import { ThinkingBlock } from "../renderers/ThinkingBlock";
import { ToolCallRow } from "../renderers/ToolCallRow";
import { MarkdownRenderer } from "../renderers/MarkdownRenderer";
import { ActivityRipple } from "../renderers/ActivityRipple";
import { Button, IconButton, Textarea } from "../ui";
import {
  buildRounds,
  isRoundCollapsible,
  parseInstructionMessage,
  type Round,
  type RoundSegment,
  type SystemMessageBlock as SystemMessageBlockData,
  type ToolCallEntry,
} from "../../utils/round";
import {
  formatDurationHuman,
  formatTime,
  getDuration,
  stripSystemReminderTags,
} from "../../utils/format";
import { formatThinkingLevel, isThinkingLevelEnabled } from "../../utils/chat";
import { latestTurnStream, segmentReasoningTiming } from "../../utils/stream";

export interface MessageListProps {
  messages: MessageRecord[];
  streams: StreamMessage[];
  instructionNotices?: InstructionNotice[];
  sessionId?: string;
  session?: Session;
  models?: Model[];
  workspaceRoot?: string;
  onRevert?: (messageId: string) => void;
  onFork?: (messageId: string) => void;
  onRetryProviderError?: (messageId: string) => void;
  scrollToBottomRequest?: number;
}

interface SegmentItem {
  key: string;
  contentId?: string;
  segment: RoundSegment;
  entry?: ToolCallEntry;
  instructionContent?: string;
  active: boolean;
  collapsed: boolean;
  reasoningStartedAt?: string | null;
  reasoningCompletedAt?: string | null;
}

type AssistantStatus = "streaming" | "complete" | "cancelled" | "failed" | "interrupted";

type ChatItem =
  | { kind: "user"; key: string; round: Round }
  | {
      kind: "assistant-meta";
      key: string;
      turnId: string;
      status: AssistantStatus;
      active: boolean;
      startedAt?: string;
      completedAt?: string;
      expanded: boolean;
      collapsible: boolean;
    }
  | { kind: "assistant-segment"; item: SegmentItem }
  | { kind: "round-footer"; key: string; footerParts: string[]; content: string }
  | {
      kind: "interruption-notice";
      key: string;
      status: "cancelled" | "interrupted";
      startedAt?: string;
      completedAt?: string;
    }
  | { kind: "system"; key: string; block: SystemMessageBlockData }
  | { kind: "stream-empty"; key: string }
  | { kind: "stream-error"; key: string; message: string }
  | {
      kind: "provider-error";
      key: string;
      error: ProviderErrorData;
      messageId: string;
      retrying?: StreamMessage["retrying"];
    };

function isSystemBlock(item: Round | SystemMessageBlockData): item is SystemMessageBlockData {
  return "kind" in item && item.kind === "system";
}

function hasAssistant(round: Round) {
  return round.segments.length > 0 || round.status !== "user_only" || round.interrupted;
}

function turnContentId(turnId: string) {
  return `assistant-content-${turnId.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
}

function roundAssistantStatus(round: Round): AssistantStatus {
  if (!round.interrupted) return round.status === "streaming" ? "streaming" : "complete";
  if (round.interruptionKind === "cancelled") return "cancelled";
  return round.interruptionKind === "failed" ? "failed" : "interrupted";
}

function relativeInstructionPath(source: string, workspaceRoot: string) {
  const normalizedSource = source.replaceAll("\\", "/");
  const normalizedRoot = workspaceRoot.replaceAll("\\", "/").replace(/\/+$/, "");
  if (!normalizedRoot) return normalizedSource;
  if (normalizedSource === normalizedRoot) return ".";
  return normalizedSource.startsWith(`${normalizedRoot}/`)
    ? normalizedSource.slice(normalizedRoot.length + 1)
    : normalizedSource;
}

function instructionMessageContent(sources: string[], workspaceRoot: string) {
  const displayPaths = sources.map((source) => relativeInstructionPath(source, workspaceRoot));
  return displayPaths.length === 1
    ? `Loaded instructions from ${displayPaths[0]}`
    : `Loaded ${displayPaths.length} instruction files: ${displayPaths.join(", ")}`;
}

function instructionReminderBlocks(content: string) {
  return content.match(/<system-reminder>[\s\S]*?<\/system-reminder>/g) ?? [];
}

function normalizedInstructionPath(path: string) {
  return path.trim().replaceAll("\\", "/").replace(/^\.\//, "").replace(/\/+$/, "");
}

function instructionSourceMatches(actual: string, expected: string, workspaceRoot: string) {
  const normalizedActual = normalizedInstructionPath(actual);
  const normalizedExpected = normalizedInstructionPath(expected);
  const relativeExpected = normalizedInstructionPath(
    relativeInstructionPath(expected, workspaceRoot),
  );

  return (
    normalizedActual === normalizedExpected ||
    normalizedActual === relativeExpected ||
    normalizedActual.endsWith(`/${relativeExpected}`) ||
    relativeExpected.endsWith(`/${normalizedActual}`)
  );
}

function instructionPayloadForSources(
  contents: string[],
  sources: string[],
  workspaceRoot: string,
): string | undefined {
  const blocks = contents.flatMap(instructionReminderBlocks);
  if (blocks.length === 0) return undefined;
  if (sources.length === 0) return blocks.join("\n\n");

  const matchingBlocks = blocks.filter((block) => {
    const blockSources = block.match(/^Instructions from:\s*(.+)$/gm) ?? [];
    return blockSources.some((line) => {
      const actualSource = line.replace(/^Instructions from:\s*/, "");
      return sources.some((source) =>
        instructionSourceMatches(actualSource, source, workspaceRoot),
      );
    });
  });

  return matchingBlocks.length > 0
    ? matchingBlocks.join("\n\n")
    : blocks.length === 1
      ? blocks[0]
      : undefined;
}

function liveInstructionMessage(
  template: Message,
  sources: string[],
  workspaceRoot: string,
  noticeIndex: number,
): Message {
  return {
    ...template,
    id: `live-instructions-${noticeIndex}`,
    role: "system",
    content: instructionMessageContent(sources, workspaceRoot),
    attachments: [],
    reasoning: "",
    tool_calls: [],
    tool_call_id: null,
    tool_name: null,
    completed_at: null,
    streaming: false,
    reasoning_started_at: null,
    reasoning_completed_at: null,
    input_tokens: null,
    output_tokens: null,
    total_tokens: null,
    cache_read_tokens: null,
    cache_write_tokens: null,
    model_id: null,
    tokens_per_second: null,
    thinking_level: null,
  };
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
  activeFromIndex = 0,
  activeReasoningStartedAt?: string | null,
  activeReasoningCompletedAt?: string | null,
  instructionContentByMessageId?: ReadonlyMap<string, string | undefined>,
): SegmentItem[] {
  const visible: SegmentItem[] = [];

  segments.forEach((segment, index) => {
    const entry = segment.type === "tool_call" ? toolCallMap[segment.toolCallId] : undefined;
    if (segment.type === "tool_call" && !entry) return;

    const isLiveSegment = index >= activeFromIndex;
    const { startedAt: segmentStartedAt, completedAt: segmentCompletedAt } = segmentReasoningTiming(
      {
        isLiveSegment,
        reasoningStartedAt,
        reasoningCompletedAt,
        activeReasoningStartedAt,
        activeReasoningCompletedAt,
      },
    );
    const segmentActive =
      active &&
      isLiveSegment &&
      segment.type === "reasoning" &&
      index === segments.length - 1 &&
      !segment.completedAt &&
      !segmentCompletedAt;
    visible.push({
      key: `${keyPrefix}-${index}-${segment.type === "tool_call" ? segment.toolCallId : "content"}`,
      contentId: visible.length === 0 ? contentId : undefined,
      segment,
      entry,
      instructionContent:
        segment.type === "instruction"
          ? instructionContentByMessageId?.get(segment.message.id)
          : undefined,
      active: segmentActive,
      collapsed: !showAll && index !== previewIndex,
      reasoningStartedAt: segmentStartedAt,
      reasoningCompletedAt: segmentCompletedAt,
    });
  });

  return visible;
}

function getTextPreviewIndex(segments: RoundSegment[]) {
  return segments.findLastIndex((segment) => segment.type === "text" && segment.content.trim());
}

function streamProviderError(stream: StreamMessage): ProviderErrorData | null {
  return (
    stream.providerError ??
    (stream.status === "failed" && stream.error
      ? {
          message: stream.error,
          retryable: false,
          request_id: stream.requestId,
          user_message_id: stream.userMessageId ?? null,
        }
      : null)
  );
}

function resolveModelDisplayName(
  modelId: string | undefined,
  models: Model[],
  session: Session | undefined,
): string | null {
  if (!modelId) return session?.model_display_name || null;

  const separator = modelId.indexOf(":");
  const providerId = separator > 0 ? modelId.slice(0, separator) : undefined;
  const rawModelId = separator > 0 ? modelId.slice(separator + 1) : modelId;

  if (
    session &&
    session.model_id === rawModelId &&
    (!providerId || session.provider_id === providerId)
  ) {
    return session.model_display_name || rawModelId;
  }

  const model = models.find(
    (candidate) =>
      candidate.model_id === rawModelId && (!providerId || candidate.provider_id === providerId),
  );
  return model?.model_display_name || modelId;
}

function buildRoundFooterParts(
  round: Round,
  models: Model[],
  session: Session | undefined,
  t: TFunction,
): string[] {
  const parts: string[] = [];
  const modelName = resolveModelDisplayName(round.modelId, models, session);
  if (modelName) parts.push(modelName);

  if (round.thinkingLevel && isThinkingLevelEnabled(round.thinkingLevel)) {
    parts.push(formatThinkingLevel(round.thinkingLevel));
  }

  if (round.completedAt) {
    const duration = getDuration(round.userMessage.created_at ?? "", round.completedAt);
    if (duration) parts.push(duration);
  }

  if (round.tokensPerSecond !== undefined && Number.isFinite(round.tokensPerSecond)) {
    parts.push(`${round.tokensPerSecond.toFixed(1)} t/s`);
  }

  if (round.completedAt) {
    parts.push(formatTime(round.completedAt, true));
  }

  if (round.mode === "plan" || round.mode === "build") {
    parts.push(t(round.mode === "plan" ? "Plan" : "Build"));
  }

  return parts;
}

export function buildChatItems(
  rounds: (Round | SystemMessageBlockData)[],
  streams: StreamMessage[],
  expandedTurns: Record<string, boolean>,
  instructionNotices: InstructionNotice[],
  workspaceRoot: string,
  models: Model[],
  session: Session | undefined,
  t: TFunction,
): ChatItem[] {
  const items: ChatItem[] = [];
  const mergedStreamKeys = new Set<string>();
  const latestRound = [...rounds].reverse().find((item): item is Round => !isSystemBlock(item));
  const instructionTurnId = latestRound?.userMessage.id;
  const liveProviderErrorUserIds = new Set(
    streams
      .map((stream) => stream.userMessageId)
      .filter((messageId): messageId is string => Boolean(messageId)),
  );
  const userMessageTimestampMap = new Map<string, string>();
  for (const value of rounds) {
    if (!isSystemBlock(value) && value.userMessage.created_at) {
      userMessageTimestampMap.set(value.userMessage.id, value.userMessage.created_at);
    }
  }

  for (const value of rounds) {
    if (isSystemBlock(value)) {
      items.push({ kind: "system", key: value.id, block: value });
      continue;
    }

    const round = value;
    const turnId = round.userMessage.id;
    const persistedSegments: RoundSegment[] = [
      ...round.leadingInstructions.map((message) => ({ type: "instruction" as const, message })),
      ...round.segments,
    ];
    const turnStream = latestTurnStream(streams, turnId);
    for (const stream of streams) {
      if (stream.userMessageId === turnId) {
        mergedStreamKeys.add(stream.key);
      }
    }
    const turnStreamHasToolCall = turnStream?.segments.some(
      (segment) => segment.type === "tool_call",
    );
    const pendingInstructions =
      turnId === instructionTurnId
        ? instructionNotices
            .map((notice, index) => ({
              message: liveInstructionMessage(
                round.userMessage,
                notice.sources,
                workspaceRoot,
                index,
              ),
              deferred: notice.deferred,
            }))
            .filter(
              ({ message, deferred }) =>
                (!deferred || !turnStream || !turnStreamHasToolCall) &&
                !persistedSegments.some(
                  (segment) =>
                    segment.type === "instruction" && segment.message.content === message.content,
                ),
            )
        : [];
    const assistant = hasAssistant(round) || Boolean(turnStream) || pendingInstructions.length > 0;

    items.push({ kind: "user", key: `${turnId}:user`, round });

    if (!assistant) continue;

    const insertInstructionBeforeStream = Boolean(turnStream && pendingInstructions.length > 0);
    let mergedSegments = turnStream
      ? [...persistedSegments, ...turnStream.segments]
      : persistedSegments;
    let activeFromIndex = persistedSegments.length;
    if (pendingInstructions.length > 0) {
      const instructionSegments = pendingInstructions.map(({ message }) => ({
        type: "instruction" as const,
        message,
      }));
      if (insertInstructionBeforeStream) {
        mergedSegments = [
          ...persistedSegments,
          ...instructionSegments,
          ...(turnStream?.segments ?? []),
        ];
        activeFromIndex += instructionSegments.length;
      } else {
        mergedSegments = [...mergedSegments, ...instructionSegments];
      }
    }
    const mergedToolCallMap = turnStream
      ? { ...round.toolCallMap, ...turnStream.toolCallMap }
      : round.toolCallMap;
    const instructionContents = [
      round.userMessage.content,
      ...Object.values(mergedToolCallMap).map((entry) => entry.result?.output ?? ""),
    ];
    const instructionContentByMessageId = new Map<string, string | undefined>();
    for (const segment of mergedSegments) {
      if (segment.type !== "instruction") continue;
      const details = parseInstructionMessage(segment.message.content);
      const sources = details
        ? details.sources
            .split(",")
            .map((source) => source.trim())
            .filter(Boolean)
        : [];
      instructionContentByMessageId.set(
        segment.message.id,
        instructionPayloadForSources(instructionContents, sources, workspaceRoot),
      );
    }
    const hasPendingInstructions = pendingInstructions.length > 0;
    const hasStreamContinuation = Boolean(turnStream);
    const isTurnStreamActive = turnStream?.status === "streaming";
    const terminalInterruption =
      !isTurnStreamActive &&
      (round.interrupted || turnStream?.status === "cancelled" || turnStream?.status === "interrupted");
    const renderableSegmentCount = mergedSegments.filter(
      (segment) => segment.type !== "tool_call" || Boolean(mergedToolCallMap[segment.toolCallId]),
    ).length;
    const collapsible =
      (isRoundCollapsible(round) ||
        (renderableSegmentCount > 1 &&
          (hasStreamContinuation ||
            round.status === "complete" ||
            Boolean(round.completedAt) ||
            terminalInterruption)));
    const expanded =
      expandedTurns[turnId] ?? (isTurnStreamActive || terminalInterruption || !collapsible);
    const previewIndex = getTextPreviewIndex(mergedSegments);
    const hasFinalAnswer = previewIndex >= 0 && previewIndex === mergedSegments.length - 1;
    const active = isTurnStreamActive || pendingInstructions.length > 0;

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
      hasStreamContinuation ||
      hasPendingInstructions ||
      ((hasFinalAnswer || round.interrupted) &&
        (Boolean(duration) ||
          collapsible ||
          (interruptionLabel !== null && round.providerErrors.length === 0)));
    const assistantStatus =
      turnStream?.status ??
      (pendingInstructions.length > 0 ? "streaming" : roundAssistantStatus(round));
    const appendTurnMeta = () => {
      items.push({
        kind: "assistant-meta",
        key: `${turnId}:meta`,
        turnId,
        status: assistantStatus,
        active,
        startedAt: round.userMessage.created_at,
        completedAt: isTurnStreamActive
          ? undefined
          : (turnStream?.completedAt ?? round.completedAt),
        expanded,
        collapsible,
      });
    };

    if (showTurnMeta) appendTurnMeta();

    const segments = makeSegmentItems(
      mergedSegments,
      mergedToolCallMap,
      !collapsible || expanded,
      previewIndex,
      active,
      round.reasoningStartedAt,
      round.reasoningCompletedAt ?? round.completedAt,
      `${turnId}:segment`,
      turnContentId(turnId),
      activeFromIndex,
      turnStream?.reasoningStartedAt,
      turnStream?.reasoningCompletedAt,
      instructionContentByMessageId,
    );
    for (const segment of segments) {
      items.push({ kind: "assistant-segment", item: segment });
    }

    for (const providerError of round.providerErrors) {
      const messageId = providerError.data.user_message_id ?? round.userMessage.id;
      if (liveProviderErrorUserIds.has(messageId)) continue;
      items.push({
        kind: "provider-error",
        key: `${turnId}:provider-error:${providerError.id}`,
        error: providerError.data,
        messageId,
      });
    }

    const turnProviderError = turnStream ? streamProviderError(turnStream) : null;
    if (turnStream && turnProviderError) {
      items.push({
        kind: "provider-error",
        key: `${turnId}:provider-error`,
        error: turnProviderError,
        messageId: turnProviderError.user_message_id ?? turnStream.userMessageId ?? "",
        retrying: turnStream.retrying,
      });
    }

    const footerParts = buildRoundFooterParts(round, models, session, t);
    const finalReplyContent =
      hasFinalAnswer && mergedSegments[previewIndex]?.type === "text"
        ? stripSystemReminderTags(mergedSegments[previewIndex].content)
        : "";
    if (
      !hasStreamContinuation &&
      round.status === "complete" &&
      !round.interrupted &&
      hasFinalAnswer &&
      footerParts.length
    ) {
      items.push({
        kind: "round-footer",
        key: `${turnId}:footer`,
        footerParts,
        content: finalReplyContent,
      });
    }
    if (terminalInterruption && assistantStatus !== "failed") {
      items.push({
        kind: "interruption-notice",
        key: `${turnId}:interruption`,
        status: assistantStatus === "cancelled" ? "cancelled" : "interrupted",
        startedAt: round.userMessage.created_at,
        completedAt: turnStream?.completedAt ?? round.completedAt,
      });
    }
  }

  for (const stream of streams) {
    if (mergedStreamKeys.has(stream.key)) continue;
    const isStreaming = stream.status === "streaming";
    const terminalInterruption = stream.status === "cancelled" || stream.status === "interrupted";
    const fallbackStreamId = `unmatched-stream-${stream.key}`;
    const previewIndex = getTextPreviewIndex(stream.segments);
    const collapsible = stream.segments.length > 1 && previewIndex > 0;
    const expanded =
      expandedTurns[fallbackStreamId] ?? (isStreaming || terminalInterruption || !collapsible);
    const providerError = streamProviderError(stream);
    const appendFallbackMeta = () => {
      const startedAt =
        (stream.userMessageId ? userMessageTimestampMap.get(stream.userMessageId) : undefined) ??
        stream.reasoningStartedAt ??
        undefined;
      items.push({
        kind: "assistant-meta",
        key: `${fallbackStreamId}:meta`,
        turnId: fallbackStreamId,
        status: stream.status,
        active: isStreaming,
        startedAt,
        completedAt: stream.completedAt ?? undefined,
        expanded,
        collapsible,
      });
    };

    if (!providerError) appendFallbackMeta();

    const segments = makeSegmentItems(
      stream.segments,
      stream.toolCallMap,
      isStreaming || expanded,
      previewIndex >= 0 ? previewIndex : null,
      isStreaming,
      stream.reasoningStartedAt,
      stream.reasoningCompletedAt,
      `${fallbackStreamId}:segment`,
      turnContentId(fallbackStreamId),
    );
    for (const segment of segments) {
      items.push({ kind: "assistant-segment", item: segment });
    }

    if (providerError) {
      items.push({
        kind: "provider-error",
        key: `${fallbackStreamId}:provider-error`,
        error: providerError,
        messageId: providerError.user_message_id ?? stream.userMessageId ?? "",
        retrying: stream.retrying,
      });
    } else if (isStreaming && stream.segments.length === 0) {
      items.push({ kind: "stream-empty", key: `${fallbackStreamId}:empty` });
    } else if (!isStreaming && !terminalInterruption) {
      items.push({
        kind: "stream-error",
        key: `${fallbackStreamId}:error`,
        message: stream.error ?? "Response failed",
      });
    }
    if (!providerError && terminalInterruption) {
      items.push({
        kind: "interruption-notice",
        key: `${fallbackStreamId}:interruption`,
        status: stream.status === "cancelled" ? "cancelled" : "interrupted",
        startedAt:
          (stream.userMessageId ? userMessageTimestampMap.get(stream.userMessageId) : undefined) ??
          stream.reasoningStartedAt ??
          undefined,
        completedAt: stream.completedAt ?? undefined,
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
    case "assistant-meta":
      return 42;
    case "assistant-segment":
      if (item.item.collapsed) return 0;
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
    case "interruption-notice":
      return 30;
    case "system":
      return 80;
    case "stream-empty":
      return 24;
    case "stream-error":
      return 36;
    case "provider-error":
      return 86;
  }
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
      ),
    [rounds, streams, expandedTurns, instructionNotices, workspaceRoot, models, session, t],
  );

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => estimateChatItemSize(items[index]),
    overscan: 6,
    getItemKey: (index) => getChatItemKey(items[index], index),
    useFlushSync: false,
    anchorTo: "end",
    scrollEndThreshold: CHAT_SCROLL_BOTTOM_THRESHOLD,
  });
  const totalSize = virtualizer.getTotalSize();
  const followTailRef = useRef(true);
  const scrollFrameRef = useRef<number | null>(null);
  const previousScrollMetricsRef = useRef<{
    sessionId?: string;
    itemCount: number;
    totalSize: number;
  } | null>(null);
  const previousScrollToBottomRequestRef = useRef(scrollToBottomRequest);

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
    const previous = previousScrollMetricsRef.current;
    const scrollWasRequested = previousScrollToBottomRequestRef.current !== scrollToBottomRequest;
    previousScrollToBottomRequestRef.current = scrollToBottomRequest;
    previousScrollMetricsRef.current = { sessionId, itemCount: items.length, totalSize };
    if (!element || (!followTailRef.current && !scrollWasRequested)) return;

    if (scrollWasRequested) followTailRef.current = true;

    const contentChanged =
      !previous ||
      previous.sessionId !== sessionId ||
      previous.itemCount !== items.length ||
      previous.totalSize !== totalSize;
    if (!contentChanged && !scrollWasRequested) return;

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
  }, [items.length, scrollToBottomRequest, sessionId, totalSize]);

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

function ProviderErrorCard({
  error,
  messageId,
  retrying,
  onRetry,
}: {
  error: ProviderErrorData;
  messageId: string;
  retrying?: StreamMessage["retrying"];
  onRetry?: (messageId: string) => void;
}) {
  const { t } = useTranslation();
  const canRetry = error.retryable && !retrying && Boolean(messageId) && Boolean(onRetry);

  return (
    <article className={retrying ? "provider-error-row is-retrying" : "provider-error-row"}>
      <div
        className={retrying ? "provider-error-card is-retrying" : "provider-error-card"}
        role="alert"
        aria-live={retrying ? "polite" : "assertive"}
      >
        <span className={retrying ? "provider-error-icon is-retrying" : "provider-error-icon"}>
          {retrying ? <LoaderCircle className="spin" size={21} /> : <CircleAlert size={21} />}
        </span>
        <div className="provider-error-content">
          <p className="provider-error-message">{error.message}</p>
          {retrying ? (
            <span className="provider-error-meta">
              {t("Retrying {{attempt}}/{{maxAttempts}}", {
                attempt: retrying.attempt,
                maxAttempts: retrying.maxAttempts,
              })}
            </span>
          ) : null}
        </div>
        {canRetry ? (
          <Button
            type="button"
            className="provider-error-retry"
            onClick={() => onRetry?.(messageId)}
            variant="secondary"
            size="sm"
            leadingIcon={<RefreshCw size={15} />}
          >
            {t("Retry")}
          </Button>
        ) : null}
      </div>
    </article>
  );
}

function QuestionApproval({
  tool,
  onDecide,
}: {
  tool: ToolCall;
  onDecide: (decision: ApprovedTool) => void;
}) {
  const { t } = useTranslation();
  const questions = useMemo(() => parseQuestionArgs(tool.arguments), [tool.arguments]);
  const [questionIndex, setQuestionIndex] = useState(0);
  const [answers, setAnswers] = useState<string[][]>(() => questions?.map(() => []) ?? []);
  const [customAnswer, setCustomAnswer] = useState("");
  const [customAnswerOpen, setCustomAnswerOpen] = useState(false);
  const customTextareaRef = useRef<HTMLTextAreaElement>(null);
  const customComposingRef = useRef(false);
  const customCompositionJustCommittedRef = useRef(false);
  const customCompositionEndTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    setQuestionIndex(0);
    setAnswers(questions?.map(() => []) ?? []);
    setCustomAnswer("");
    setCustomAnswerOpen(false);
  }, [questions]);

  useEffect(() => {
    if (!customAnswerOpen) return;
    const textarea = customTextareaRef.current;
    if (!textarea) return;

    textarea.style.height = "auto";
    const height = Math.min(textarea.scrollHeight, 200);
    textarea.style.height = `${height}px`;
    textarea.style.overflowY = textarea.scrollHeight > 200 ? "auto" : "hidden";
  }, [customAnswer, customAnswerOpen]);

  useEffect(
    () => () => {
      if (customCompositionEndTimerRef.current) {
        clearTimeout(customCompositionEndTimerRef.current);
      }
    },
    [],
  );

  if (!questions) {
    return (
      <div className="question-composer question-composer-invalid" role="alert">
        <div className="question-composer-header">
          <p>{t("This question request has invalid or empty arguments.")}</p>
          <IconButton
            className="question-composer-dismiss"
            label={t("Dismiss question")}
            onClick={() => onDecide(invalidQuestionResult(tool))}
            size="sm"
            variant="ghost"
          >
            <X size={16} />
          </IconButton>
        </div>
      </div>
    );
  }

  const question = questions[questionIndex];
  const multiple = question.multiple ?? false;
  const selectedAnswers = answers[questionIndex] ?? [];
  const allowCustom = question.custom ?? true;

  const advance = (nextAnswers: string[][]) => {
    if (questionIndex + 1 >= questions.length) {
      onDecide(questionResult(tool, formatQuestionAnswers(questions, nextAnswers)));
      return;
    }
    setAnswers(nextAnswers);
    setQuestionIndex((index) => index + 1);
    setCustomAnswer("");
    setCustomAnswerOpen(false);
  };

  const setCurrentAnswers = (nextSelected: string[]) =>
    answers.map((selected, index) => (index === questionIndex ? nextSelected : selected));

  const selectOption = (label: string) => {
    if (!multiple) {
      advance(setCurrentAnswers([label]));
      return;
    }

    setAnswers(
      setCurrentAnswers(
        selectedAnswers.includes(label)
          ? selectedAnswers.filter((value) => value !== label)
          : [...selectedAnswers, label],
      ),
    );
  };

  const continueWithCurrentAnswer = () => {
    const custom = customAnswer.trim();
    const nextSelected = custom
      ? multiple
        ? selectedAnswers.includes(custom)
          ? selectedAnswers
          : [...selectedAnswers, custom]
        : [custom]
      : selectedAnswers;
    if (nextSelected.length === 0) return;
    advance(setCurrentAnswers(nextSelected));
  };

  return (
    <div
      className="question-composer"
      role="group"
      aria-label={question.header || question.question}
    >
      <div className="question-composer-header">
        <div>
          <p>{question.question}</p>
          {questions.length > 1 ? (
            <span>
              {t("Question {{current}} of {{total}}", {
                current: questionIndex + 1,
                total: questions.length,
              })}
            </span>
          ) : null}
        </div>
      </div>
      {question.options.length > 0 ? (
        <div className="question-composer-options">
          {question.options.map((option, optionIndex) => {
            const selected = selectedAnswers.includes(option.label);
            return (
              <button
                aria-pressed={multiple ? selected : undefined}
                className={
                  selected ? "question-composer-option selected" : "question-composer-option"
                }
                key={`${tool.id}-${questionIndex}-${optionIndex}`}
                onClick={() => selectOption(option.label)}
                type="button"
              >
                <span className="question-composer-option-index">{optionIndex + 1}</span>
                <span className="question-composer-option-copy">
                  <strong>{option.label}</strong>
                  {option.description ? <small>{option.description}</small> : null}
                </span>
                {multiple ? (
                  selected ? (
                    <Check className="question-composer-option-check" size={17} />
                  ) : null
                ) : (
                  <ChevronRight className="question-composer-option-arrow" size={18} />
                )}
              </button>
            );
          })}
        </div>
      ) : null}
      {allowCustom ? (
        <div className="question-composer-custom">
          {customAnswerOpen ? (
            <div className="question-composer-custom-editor">
              <span className="question-composer-custom-leading" aria-hidden="true">
                <Pencil size={15} />
              </span>
              <Textarea
                autoFocus
                ref={customTextareaRef}
                onChange={(event) => setCustomAnswer(event.target.value)}
                onCompositionStart={() => {
                  customComposingRef.current = true;
                  customCompositionJustCommittedRef.current = false;
                  if (customCompositionEndTimerRef.current) {
                    clearTimeout(customCompositionEndTimerRef.current);
                  }
                }}
                onCompositionEnd={() => {
                  customComposingRef.current = false;
                  customCompositionJustCommittedRef.current = true;
                  if (customCompositionEndTimerRef.current) {
                    clearTimeout(customCompositionEndTimerRef.current);
                  }
                  customCompositionEndTimerRef.current = setTimeout(() => {
                    customCompositionJustCommittedRef.current = false;
                  }, 0);
                }}
                onKeyDown={(event) => {
                  if (
                    event.key === "Enter" &&
                    !event.shiftKey &&
                    !event.nativeEvent.isComposing &&
                    !customComposingRef.current &&
                    !customCompositionJustCommittedRef.current
                  ) {
                    event.preventDefault();
                    continueWithCurrentAnswer();
                  }
                }}
                placeholder={t("Type a custom answer")}
                rows={1}
                value={customAnswer}
              />
            </div>
          ) : (
            <button
              className="question-composer-custom-trigger"
              onClick={() => setCustomAnswerOpen(true)}
              type="button"
            >
              <span className="question-composer-custom-leading" aria-hidden="true">
                <Pencil size={15} />
              </span>
              <span>{t("Type a custom answer")}</span>
            </button>
          )}
        </div>
      ) : null}
      <div className="question-composer-footer">
        <Button
          className="question-composer-skip"
          type="button"
          onClick={() => onDecide(questionResult(tool, null))}
          variant="secondary"
          size="sm"
        >
          {t("Skip")}
        </Button>
        {multiple || customAnswer.trim() ? (
          <Button
            className="question-composer-continue"
            disabled={selectedAnswers.length === 0 && !customAnswer.trim()}
            onClick={continueWithCurrentAnswer}
            size="sm"
            variant="primary"
          >
            {t("Continue")}
          </Button>
        ) : null}
      </div>
    </div>
  );
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
  const [toolIndex, setToolIndex] = useState(0);
  const [decisions, setDecisions] = useState<ApprovedTool[]>([]);
  const current = tools[toolIndex];

  useEffect(() => {
    setToolIndex(0);
    setDecisions([]);
  }, [request.request_id]);

  const decide = useCallback(
    (decision: ApprovedTool) => {
      const next = [...decisions, decision];
      if (toolIndex + 1 >= tools.length) {
        onRespond(next);
        return;
      }
      setDecisions(next);
      setToolIndex((index) => index + 1);
    },
    [decisions, onRespond, toolIndex, tools.length],
  );

  if (!current) {
    return null;
  }

  const isQuestion = isQuestionTool(current.tool_call);
  if (isQuestion) {
    return <QuestionApproval tool={current.tool_call} onDecide={decide} />;
  }

  return (
    <div className="permission-composer">
      <div className="permission-composer-header">
        <p>{t("Allow tidev to run {{toolName}}?", { toolName: current.tool_call.name })}</p>
      </div>
      {tools.length > 1 ? (
        <span className="permission-composer-progress">
          {t("Step {{current}} / {{total}}", { current: toolIndex + 1, total: tools.length })}
        </span>
      ) : null}
      <div className="permission-composer-tool">
        <code>{current.tool_call.arguments}</code>
      </div>
      <div className="permission-composer-risks">
        {current.workspace_boundary_violation ? (
          <div className="permission-composer-risk">
            <CircleAlert size={16} aria-hidden="true" />
            <div>
              <strong>{t("Workspace boundary access")}</strong>
              <code>{current.workspace_boundary_violation}</code>
            </div>
          </div>
        ) : null}
        {current.sensitive_file_violation ? (
          <div className="permission-composer-risk">
            <CircleAlert size={16} aria-hidden="true" />
            <div>
              <strong>{t("Sensitive file access")}</strong>
              <code>{current.sensitive_file_violation}</code>
            </div>
          </div>
        ) : null}
      </div>
      <div className="permission-composer-footer">
        <Button
          className="permission-composer-reject"
          type="button"
          onClick={() => decide(rejectedTool(current.tool_call))}
          variant="secondary"
          size="sm"
          leadingIcon={<X size={15} />}
        >
          {t("Reject")}
        </Button>
        <Button
          className="permission-composer-allow"
          type="button"
          onClick={() => decide(approvedPermissionTool(current))}
          variant="primary"
          size="sm"
          leadingIcon={<Check size={15} />}
        >
          {t("Allow")}
        </Button>
      </div>
    </div>
  );
}
