import type { TFunction } from "i18next";
import type { Message, Model, ProviderErrorData, Session } from "../../types/api";
import type { CompactionNotice, InstructionNotice, StreamMessage } from "../../types/chat";
import {
  isRoundCollapsible,
  parseInstructionMessage,
  type Round,
  type RoundSegment,
  type SystemMessageBlock as SystemMessageBlockData,
  type ToolCallEntry,
} from "../../utils/round";
import { formatTime, getDuration } from "../../utils/format";
import { formatThinkingLevel, isThinkingLevelEnabled } from "../../utils/chat";
import { latestTurnStream, segmentReasoningTiming } from "../../utils/stream";

export interface SegmentItem {
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

export type AssistantStatus = "streaming" | "complete" | "cancelled" | "failed" | "interrupted";

export type ChatItem =
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
  | { kind: "compaction"; key: string; notice: CompactionNotice }
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

function mergeStreamReasoningSegments(
  segments: RoundSegment[],
  display?: StreamMessage["reasoningDisplay"],
): RoundSegment[] {
  const reasoningSegments = segments.filter(
    (segment): segment is Extract<RoundSegment, { type: "reasoning" }> =>
      segment.type === "reasoning",
  );
  if (reasoningSegments.length === 0 || (reasoningSegments.length === 1 && !display))
    return segments;

  const first = reasoningSegments[0];
  const last = reasoningSegments[reasoningSegments.length - 1];
  const mergedReasoning: RoundSegment = {
    ...first,
    content: reasoningSegments.map((segment) => segment.content).join(""),
    completedAt: last.completedAt,
    ...(display ? { display } : {}),
  };

  return [mergedReasoning, ...segments.filter((segment) => segment.type !== "reasoning")];
}

export function turnContentId(turnId: string) {
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
  const relativePath = !normalizedRoot
    ? normalizedSource
    : normalizedSource === normalizedRoot
      ? "."
      : normalizedSource.startsWith(`${normalizedRoot}/`)
        ? normalizedSource.slice(normalizedRoot.length + 1)
        : normalizedSource;
  return isWindowsInstructionPath(source) || isWindowsInstructionPath(workspaceRoot)
    ? relativePath.replaceAll("/", "\\")
    : relativePath;
}

function instructionMessageContent(sources: string[], workspaceRoot: string) {
  const displayPaths = sources.map((source) => relativeInstructionPath(source, workspaceRoot));
  return displayPaths.length === 1
    ? `Loaded instructions from ${displayPaths[0]}`
    : `Loaded ${displayPaths.length} instruction files: ${displayPaths.join(", ")}`;
}

function isWindowsInstructionPath(path: string) {
  const normalized = path.replaceAll("\\", "/");
  return /^[a-z]:\//i.test(normalized) || normalized.startsWith("//");
}

function collapseInstructionPath(path: string) {
  const normalized = path.trim().replaceAll("\\", "/");
  let prefix = "";
  let remainder = normalized;

  if (normalized.startsWith("//")) {
    prefix = "//";
    remainder = normalized.slice(2);
  } else if (/^[a-z]:\//i.test(normalized)) {
    prefix = `${normalized.slice(0, 2).toLowerCase()}/`;
    remainder = normalized.slice(3);
  } else if (normalized.startsWith("/")) {
    prefix = "/";
    remainder = normalized.slice(1);
  }

  const segments: string[] = [];
  for (const segment of remainder.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === ".." && segments.length > 0 && segments.at(-1) !== "..") {
      segments.pop();
    } else if (segment !== ".." || !prefix) {
      segments.push(segment);
    }
  }

  return `${prefix}${segments.join("/")}` || prefix || ".";
}

function instructionSourceIdentity(source: string, workspaceRoot: string) {
  const caseInsensitive =
    isWindowsInstructionPath(source) || isWindowsInstructionPath(workspaceRoot);
  const normalizedSource = collapseInstructionPath(source);
  const normalizedRoot = workspaceRoot ? collapseInstructionPath(workspaceRoot) : "";
  const sourceKey = caseInsensitive ? normalizedSource.toLowerCase() : normalizedSource;
  const rootKey = caseInsensitive ? normalizedRoot.toLowerCase() : normalizedRoot;

  if (!rootKey) return sourceKey;
  if (sourceKey === rootKey) return ".";

  const rootPrefix = rootKey.endsWith("/") ? rootKey : `${rootKey}/`;
  return sourceKey.startsWith(rootPrefix) ? sourceKey.slice(rootPrefix.length) : sourceKey;
}

function instructionSourcesIdentity(sources: string[], workspaceRoot: string) {
  if (sources.length === 0) return null;
  return JSON.stringify(
    sources.map((source) => instructionSourceIdentity(source, workspaceRoot)).sort(),
  );
}

function persistedInstructionSourcesIdentity(content: string, workspaceRoot: string) {
  const details = parseInstructionMessage(content);
  if (!details) return null;
  const sources =
    details.count === null
      ? [details.sources]
      : details.sources
          .split(",")
          .map((source) => source.trim())
          .filter(Boolean);
  if (details.count !== null && sources.length !== details.count) return null;
  return instructionSourcesIdentity(sources, workspaceRoot);
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
  let textCounter = 0;
  let reasoningCounter = 0;

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

    let segmentSubKey: string;
    if (segment.type === "tool_call") {
      segmentSubKey = `tool-${segment.toolCallId}`;
    } else if (segment.type === "instruction") {
      segmentSubKey = `instruction-${segment.message.id}`;
    } else if (segment.type === "compaction") {
      segmentSubKey = `compaction-${segment.message.id}`;
    } else if (segment.type === "reasoning") {
      segmentSubKey = `reasoning-${reasoningCounter++}`;
    } else {
      segmentSubKey = `text-${textCounter++}`;
    }

    visible.push({
      key: `${keyPrefix}-${segmentSubKey}`,
      contentId:
        segment.type !== "compaction" && !visible.some((item) => item.segment.type !== "compaction")
          ? contentId
          : undefined,
      segment,
      entry,
      instructionContent:
        segment.type === "instruction"
          ? instructionContentByMessageId?.get(segment.message.id)
          : undefined,
      active: segmentActive,
      collapsed: segment.type !== "compaction" && !showAll && index !== previewIndex,
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

const COMPACTION_CONTINUATION_PREFIX = "The conversation context before this point was compacted";

function extractCompactionSummary(content: string): string | null {
  const body = content.split("\n\n").slice(1).join("\n\n").trim();
  if (!body) return null;
  const footerIndex = body.lastIndexOf(COMPACTION_CONTINUATION_PREFIX);
  if (footerIndex !== -1) {
    return body.slice(0, footerIndex).trim() || null;
  }
  return body;
}

function persistedCompactionNotice(message: Message): CompactionNotice {
  return {
    status: "complete",
    manual: message.metadata.compaction_manual === true,
    summary: extractCompactionSummary(message.content),
    error: null,
    modelId: null,
    completedAt: message.created_at || null,
    afterUserMessageId: null,
    beforeRequestId: null,
  };
}

function matchesCompactionNotice(message: Message, notice: CompactionNotice | null): boolean {
  if (!notice || notice.status !== "complete" || !notice.summary) return false;
  return (
    message.metadata.compaction_manual === notice.manual &&
    (message.content === `Compaction\n\n${notice.summary}` ||
      message.content.startsWith(`Compaction\n\n${notice.summary}\n\n`))
  );
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
  compactionNotice: CompactionNotice | null = null,
): ChatItem[] {
  const items: ChatItem[] = [];
  const mergedStreamKeys = new Set<string>();
  const displayedInstructionIdentities = new Set<string>();
  const latestRound = [...rounds].reverse().find((item): item is Round => !isSystemBlock(item));
  const instructionTurnId = latestRound?.userMessage.id;
  const liveProviderErrorUserIds = new Set(
    streams
      .map((stream) => stream.userMessageId)
      .filter((messageId): messageId is string => Boolean(messageId)),
  );
  const persistedLiveCompaction = rounds.some((value) =>
    isSystemBlock(value)
      ? matchesCompactionNotice(value.message, compactionNotice)
      : value.segments.some(
          (segment) =>
            segment.type === "compaction" &&
            matchesCompactionNotice(segment.message, compactionNotice),
        ),
  );
  const liveCompactionNotice = persistedLiveCompaction ? null : compactionNotice;
  let liveCompactionInserted = false;
  const registerPersistedInstruction = (content: string) => {
    const identity = persistedInstructionSourcesIdentity(content, workspaceRoot);
    if (identity === null) return true;
    if (displayedInstructionIdentities.has(identity)) return false;
    displayedInstructionIdentities.add(identity);
    return true;
  };
  const userMessageTimestampMap = new Map<string, string>();
  for (const value of rounds) {
    if (!isSystemBlock(value) && value.userMessage.created_at) {
      userMessageTimestampMap.set(value.userMessage.id, value.userMessage.created_at);
    }
  }

  for (const value of rounds) {
    if (isSystemBlock(value)) {
      if (!registerPersistedInstruction(value.message.content)) continue;
      items.push({ kind: "system", key: value.id, block: value });
      continue;
    }

    const round = value;
    const turnId = round.userMessage.id;
    const persistedSegments: RoundSegment[] = [
      ...round.leadingInstructions
        .filter((message) => registerPersistedInstruction(message.content))
        .map((message) => ({ type: "instruction" as const, message })),
      ...round.segments.filter(
        (segment) =>
          segment.type !== "instruction" || registerPersistedInstruction(segment.message.content),
      ),
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
              identity: instructionSourcesIdentity(notice.sources, workspaceRoot),
              deferred: notice.deferred,
            }))
            .filter(({ identity, deferred }) => {
              if (deferred && turnStream && turnStreamHasToolCall) return false;
              if (identity === null) return true;
              if (displayedInstructionIdentities.has(identity)) return false;
              displayedInstructionIdentities.add(identity);
              return true;
            })
        : [];
    const assistant = hasAssistant(round) || Boolean(turnStream) || pendingInstructions.length > 0;

    items.push({ kind: "user", key: `${turnId}:user`, round });

    if (!assistant) continue;

    const insertInstructionBeforeStream = Boolean(turnStream && pendingInstructions.length > 0);
    const liveSegments = turnStream
      ? mergeStreamReasoningSegments(turnStream.segments, turnStream.reasoningDisplay)
      : [];
    let mergedSegments = turnStream ? [...persistedSegments, ...liveSegments] : persistedSegments;
    let activeFromIndex = persistedSegments.length;
    if (pendingInstructions.length > 0) {
      const instructionSegments = pendingInstructions.map(({ message }) => ({
        type: "instruction" as const,
        message,
      }));
      if (insertInstructionBeforeStream) {
        mergedSegments = [...persistedSegments, ...instructionSegments, ...liveSegments];
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
      (round.interrupted ||
        turnStream?.status === "cancelled" ||
        turnStream?.status === "interrupted");
    const renderableSegmentCount = mergedSegments.filter(
      (segment) => segment.type !== "tool_call" || Boolean(mergedToolCallMap[segment.toolCallId]),
    ).length;
    const collapsible =
      isRoundCollapsible(round) ||
      (renderableSegmentCount > 1 &&
        (hasStreamContinuation ||
          round.status === "complete" ||
          Boolean(round.completedAt) ||
          terminalInterruption));
    const expanded =
      expandedTurns[turnId] ?? (isTurnStreamActive || terminalInterruption || !collapsible);
    const previewIndex = getTextPreviewIndex(mergedSegments);
    const hasFinalAnswer =
      previewIndex >= 0 &&
      mergedSegments.slice(previewIndex + 1).every((segment) => segment.type === "compaction");
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
    const liveCompactionForRound =
      liveCompactionNotice?.afterUserMessageId === turnId ? liveCompactionNotice : null;
    const streamIsAfterAutomaticCompaction =
      liveCompactionForRound &&
      !liveCompactionForRound.manual &&
      turnStream &&
      (liveCompactionForRound.beforeRequestId === null ||
        turnStream.requestId > liveCompactionForRound.beforeRequestId ||
        (turnStream.requestId === liveCompactionForRound.beforeRequestId &&
          turnStream.status === "streaming" &&
          !turnStream.providerFinished));
    const streamSegmentIndex = streamIsAfterAutomaticCompaction
      ? segments.findIndex((segment) => liveSegments.includes(segment.segment))
      : -1;
    const appendSegment = (segment: SegmentItem) => {
      if (segment.segment.type === "compaction") {
        items.push({
          kind: "compaction",
          key: segment.key,
          notice: persistedCompactionNotice(segment.segment.message),
        });
      } else {
        items.push({ kind: "assistant-segment", item: segment });
      }
    };
    segments.forEach((segment, index) => {
      if (index === streamSegmentIndex && liveCompactionForRound) {
        items.push({
          kind: "compaction",
          key: "context-compaction",
          notice: liveCompactionForRound,
        });
        liveCompactionInserted = true;
      }
      appendSegment(segment);
    });
    if (liveCompactionForRound && !liveCompactionInserted) {
      items.push({
        kind: "compaction",
        key: "context-compaction",
        notice: liveCompactionForRound,
      });
      liveCompactionInserted = true;
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
        ? mergedSegments[previewIndex].content
        : "";
    if (
      !hasStreamContinuation &&
      round.status === "complete" &&
      !round.interrupted &&
      hasFinalAnswer &&
      footerParts.length &&
      round.providerErrors.length === 0 &&
      !turnProviderError
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

  if (liveCompactionNotice && !liveCompactionInserted) {
    items.push({ kind: "compaction", key: "context-compaction", notice: liveCompactionNotice });
  }

  return items;
}

export function estimateChatItemSize(item: ChatItem | undefined) {
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
        case "compaction":
          return 26;
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
    case "compaction":
      return item.notice.status === "failed" && item.notice.error ? 46 : 26;
    case "stream-empty":
      return 24;
    case "stream-error":
      return 36;
    case "provider-error":
      return 66;
  }
}
