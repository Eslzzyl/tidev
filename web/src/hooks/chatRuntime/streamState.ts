import { ApiError } from "../../api/client";
import { asString } from "../../utils/events";
import { toolCallEntry, toolResultStatus, type ToolCallEntry } from "../../utils/round";
import type { ProviderErrorData, Session, ToolCall, ToolExecutionResult } from "../../types/api";
import type { StreamMessage } from "../../types/chat";
import i18n from "../../i18n";

export function isSessionNotFound(reason: unknown): boolean {
  return reason instanceof ApiError && reason.status === 404;
}

export function mergeSessions(current: Session[], incoming: Session[]): Session[] {
  const seen = new Set<string>();
  return [...current, ...incoming].filter((session) => {
    if (seen.has(session.session_id)) return false;
    seen.add(session.session_id);
    return true;
  });
}

export function createStream(key: string, requestId: number): StreamMessage {
  return {
    key,
    requestId,
    segments: [],
    toolCallMap: {},
    status: "streaming",
    providerFinished: false,
    reasoningStartedAt: null,
    reasoningCompletedAt: null,
    completedAt: null,
  };
}

export function cloneStream(stream: StreamMessage): StreamMessage {
  return {
    ...stream,
    segments: stream.segments.slice(),
    toolCallMap: { ...stream.toolCallMap },
  };
}

export function freezeReasoning(stream: StreamMessage) {
  const lastIndex = stream.segments.length - 1;
  const last = stream.segments[lastIndex];
  if (last?.type !== "reasoning" || !stream.reasoningStartedAt) return;

  const completedAt = stream.reasoningCompletedAt ?? new Date().toISOString();
  stream.reasoningCompletedAt = completedAt;
  if (!last.completedAt) {
    stream.segments[lastIndex] = { ...last, completedAt };
  }
}

export function appendSegment(stream: StreamMessage, type: "text" | "reasoning", content: string) {
  if (!content) return;
  if (type === "text") freezeReasoning(stream);
  const lastIndex = stream.segments.length - 1;
  const last = stream.segments[lastIndex];
  if (last?.type === type) {
    stream.segments[lastIndex] = { ...last, content: last.content + content };
  } else {
    stream.segments.push({ type, content });
  }
}

export function reconcileSegment(
  stream: StreamMessage,
  type: "text" | "reasoning",
  content: string,
) {
  if (!content) return;
  const index = stream.segments.findLastIndex((segment) => segment.type === type);
  const existing = index >= 0 ? stream.segments[index] : undefined;
  if (!existing || existing.type !== type) {
    stream.segments.push({ type, content });
    return;
  }
  if (existing.content === content || existing.content.endsWith(content)) return;
  if (content.startsWith(existing.content)) {
    stream.segments[index] = { ...existing, content };
  } else if (!existing.content.includes(content)) {
    stream.segments.push({ type, content });
  }
}

export function ensureToolCall(stream: StreamMessage, toolCall: ToolCall): ToolCallEntry {
  freezeReasoning(stream);
  const existing = stream.toolCallMap[toolCall.id];
  if (existing) {
    const updated = { ...existing, name: toolCall.name, arguments: toolCall.arguments };
    stream.toolCallMap[toolCall.id] = updated;
    return updated;
  }

  const entry = toolCallEntry(toolCall);
  stream.toolCallMap[toolCall.id] = entry;
  stream.segments.push({ type: "tool_call", toolCallId: toolCall.id });
  return entry;
}

export function emptyToolMetadata() {
  return {
    filepath: null,
    diff: null,
    truncated: null,
    exists: null,
    prior_summary: null,
    prior_retained_from: null,
    file_changes: [],
    exit_code: null,
    duration_ms: null,
  };
}

export function updateShellResult(
  entry: ToolCallEntry,
  content: string,
  finished: boolean,
  exitCode: number | null,
): void {
  const result: ToolExecutionResult = {
    output: content,
    attachments: entry.result?.attachments ?? [],
    metadata: {
      ...(entry.result?.metadata ?? emptyToolMetadata()),
      exit_code: exitCode ?? entry.result?.metadata.exit_code ?? null,
    },
  };
  entry.result = result;
  entry.status = finished ? toolResultStatus(result) : "running";
}

export function toolCallFromPayload(value: unknown): ToolCall | null {
  if (!value || typeof value !== "object") return null;
  const candidate = value as Partial<ToolCall>;
  if (
    typeof candidate.id !== "string" ||
    typeof candidate.name !== "string" ||
    typeof candidate.arguments !== "string"
  ) {
    return null;
  }
  return {
    id: candidate.id,
    name: candidate.name,
    arguments: candidate.arguments,
    thought_signature: candidate.thought_signature ?? null,
  };
}

export function toolResultFromPayload(value: unknown): ToolExecutionResult | null {
  if (!value || typeof value !== "object") return null;
  const candidate = value as Partial<ToolExecutionResult>;
  if (
    typeof candidate.output !== "string" ||
    !Array.isArray(candidate.attachments) ||
    !candidate.metadata ||
    typeof candidate.metadata !== "object"
  ) {
    return null;
  }
  return candidate as ToolExecutionResult;
}

export function providerErrorFromPayload(
  value: unknown,
  retryableValue: unknown,
  requestId: number,
  userMessageId: string | null,
): ProviderErrorData {
  const candidate = value && typeof value === "object" ? (value as Record<string, unknown>) : null;
  const message = candidate ? asString(candidate.message) : asString(value);
  return {
    message: message || i18n.t("Response failed"),
    retryable:
      candidate && typeof candidate.retryable === "boolean"
        ? candidate.retryable
        : retryableValue === true,
    request_id:
      candidate && typeof candidate.request_id === "number" ? candidate.request_id : requestId,
    user_message_id:
      candidate && typeof candidate.user_message_id === "string"
        ? candidate.user_message_id
        : userMessageId,
  };
}

export function updateSubagentEntry(
  stream: StreamMessage,
  toolCallId: string,
  childSessionId: string,
  statusText: string,
  currentToolCall: ToolCall | null,
  contentDelta: string,
  reasoningDelta: string,
): void {
  const entry = stream.toolCallMap[toolCallId];
  if (!entry) return;
  const updated: ToolCallEntry = {
    ...entry,
    status: entry.status === "completed" || entry.status === "failed" ? entry.status : "running",
    childSessionId,
    subagentStatus: statusText || entry.subagentStatus,
    subagentContentDelta: `${entry.subagentContentDelta ?? ""}${contentDelta}`,
    subagentReasoningDelta: `${entry.subagentReasoningDelta ?? ""}${reasoningDelta}`,
  };
  if (currentToolCall) {
    updated.name = currentToolCall.name;
  }
  stream.toolCallMap[toolCallId] = updated;
}
