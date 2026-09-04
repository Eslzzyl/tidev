import type {
  Message,
  MessageRecord,
  ProviderErrorData,
  ToolCall,
  ToolExecutionResult,
} from "../types/api";

export type ToolCallStatus = "pending" | "running" | "completed" | "failed" | "cancelled";

export interface ToolCallEntry {
  id: string;
  name: string;
  arguments: string;
  status: ToolCallStatus;
  result?: ToolExecutionResult;
  childSessionId?: string;
  subagentStatus?: string;
  subagentContentDelta?: string;
  subagentReasoningDelta?: string;
}

export type RoundSegment =
  | { type: "text"; content: string }
  | {
      type: "reasoning";
      content: string;
      startedAt?: string;
      completedAt?: string;
    }
  | { type: "instruction"; message: Message }
  | { type: "tool_call"; toolCallId: string };

export type RoundInterruptionKind = "cancelled" | "failed" | "interrupted";

export interface RoundProviderError {
  id: string;
  data: ProviderErrorData;
}

export interface Round {
  id: string;
  userMessage: Message;
  leadingInstructions: Message[];
  segments: RoundSegment[];
  toolCallMap: Record<string, ToolCallEntry>;
  providerErrors: RoundProviderError[];
  status: "user_only" | "streaming" | "complete";
  interrupted: boolean;
  interruptionKind?: RoundInterruptionKind;
  completedAt?: string;
  modelId?: string;
  thinkingLevel?: Message["thinking_level"];
  tokensPerSecond?: number;
  mode?: string;
  reasoningStartedAt?: string;
  reasoningCompletedAt?: string;
}

/**
 * Return the last user-facing text segment to keep visible in a collapsed
 * round, or null when the round has no text preview.
 */
export function getRoundPreviewIndex(round: Round): number | null {
  for (let index = round.segments.length - 1; index >= 0; index -= 1) {
    const segment = round.segments[index];
    if (segment?.type === "text" && segment.content.trim()) return index;
  }
  return null;
}

/**
 * Decide whether a terminal round offers a compact preview. Interrupted rounds
 * start expanded in the message list, but retain their normal fold control.
 */
export function isRoundCollapsible(round: Round): boolean {
  if (round.status !== "complete" || !round.segments.length) return false;

  const previewIndex = getRoundPreviewIndex(round);
  return Boolean(round.completedAt && previewIndex !== null && previewIndex > 0);
}

function markInterrupted(round: Round, kind: RoundInterruptionKind): void {
  round.interrupted = true;
  round.interruptionKind ??= kind;
}

function interruptionKind(record: MessageRecord): RoundInterruptionKind | undefined {
  switch (record.app_data.interruption?.reason) {
    case "user_cancelled":
      return "cancelled";
    case "provider_failed":
      return "failed";
    case "runtime_restarted":
      return "interrupted";
    default:
      return undefined;
  }
}

export interface SystemMessageBlock {
  id: string;
  message: Message;
  kind: "system";
}

export function orderedToolCalls(round: Round): ToolCallEntry[] {
  const result: ToolCallEntry[] = [];
  for (const seg of round.segments) {
    if (seg.type === "tool_call") {
      const entry = round.toolCallMap[seg.toolCallId];
      if (entry) result.push(entry);
    }
  }
  return result;
}

function unwrapMessage(record: MessageRecord): Message {
  return record.message;
}

function applyToolResultMetadata(
  record: MessageRecord,
  entry: ToolCallEntry,
  result: ToolExecutionResult,
): void {
  entry.result = result;
  entry.status = toolResultStatus(result);
  entry.childSessionId = record.app_data.child_session_id ?? entry.childSessionId;
}

export function toolResultStatus(result: ToolExecutionResult): ToolCallStatus {
  const exitCode = result.metadata.exit_code;
  const output = result.output.trim();
  if (exitCode !== null && exitCode !== 0) return "failed";
  if (output.startsWith("User cancelled the request")) return "cancelled";
  if (output.startsWith("Error:")) return "failed";
  return "completed";
}

export function toolResultFromMessage(message: Message): ToolExecutionResult {
  return {
    output: message.content,
    attachments: message.attachments,
    metadata: message.metadata,
  };
}

export function toolCallEntry(toolCall: ToolCall): ToolCallEntry {
  return {
    id: toolCall.id,
    name: toolCall.name,
    arguments: toolCall.arguments,
    status: "pending",
  };
}

export interface InstructionMessageDetails {
  count: number | null;
  sources: string;
}

export function parseInstructionMessage(content: string): InstructionMessageDetails | null {
  const single = content.match(/^Loaded instructions from\s+(.+)$/);
  if (single) return { count: null, sources: single[1].trim() };

  const multiple = content.match(/^Loaded (\d+) instruction files:\s*(.+)$/);
  if (multiple) return { count: Number(multiple[1]), sources: multiple[2].trim() };

  return null;
}

/**
 * Build a list of Rounds and SystemMessageBlocks
 * from a flat list of MessageRecords.
 *
 * Adapted from last-full web/src/utils/round.ts to work with the
 * new tidev-core message shape (MessageRecord wraps Message + app_data).
 */
export function buildRounds(records: MessageRecord[]): (Round | SystemMessageBlock)[] {
  const rounds: (Round | SystemMessageBlock)[] = [];
  let currentRound: Round | null = null;

  for (const record of records) {
    const msg = unwrapMessage(record);

    if (msg.role === "user") {
      currentRound = {
        id: `round-${msg.id}`,
        userMessage: msg,
        leadingInstructions: [],
        segments: [],
        toolCallMap: {},
        providerErrors: [],
        status: "user_only",
        interrupted: false,
        mode: record.app_data.mode ?? undefined,
        thinkingLevel: msg.thinking_level ?? undefined,
      };
      rounds.push(currentRound);
    } else if (msg.role === "system") {
      if (currentRound && parseInstructionMessage(msg.content)) {
        if (currentRound.segments.length === 0) {
          currentRound.leadingInstructions.push(msg);
        } else {
          currentRound.segments.push({ type: "instruction", message: msg });
        }
      } else {
        rounds.push({
          id: `system-${msg.id}`,
          message: msg,
          kind: "system",
        });
      }
    } else if (currentRound) {
      if (msg.role === "assistant") {
        const durableInterruption = interruptionKind(record);
        if (durableInterruption) {
          markInterrupted(currentRound, durableInterruption);
        } else if (!msg.streaming && !msg.completed_at) {
          markInterrupted(currentRound, "interrupted");
        }

        if (msg.reasoning) {
          const reasoningCompletedAt =
            msg.reasoning_completed_at ?? (msg.completed_at ? msg.completed_at : undefined);
          currentRound.segments.push({
            type: "reasoning",
            content: msg.reasoning,
            startedAt: msg.reasoning_started_at ?? undefined,
            completedAt: reasoningCompletedAt,
          });
        }

        if (msg.content) {
          const lastSeg = currentRound.segments[currentRound.segments.length - 1];
          if (lastSeg && lastSeg.type === "text") {
            lastSeg.content += "\n" + msg.content;
          } else {
            currentRound.segments.push({ type: "text", content: msg.content });
          }
        }

        if (msg.tool_calls) {
          for (const tc of msg.tool_calls) {
            const existing = currentRound.toolCallMap[tc.id];
            if (!existing) {
              const entry = toolCallEntry(tc);
              currentRound.toolCallMap[tc.id] = entry;
              currentRound.segments.push({
                type: "tool_call",
                toolCallId: tc.id,
              });
            } else {
              existing.arguments = tc.arguments;
            }
          }
        }

        if (msg.completed_at) {
          currentRound.completedAt = msg.completed_at;
        }
        if (msg.reasoning_started_at) {
          currentRound.reasoningStartedAt = msg.reasoning_started_at;
        }
        if (msg.reasoning_completed_at) {
          currentRound.reasoningCompletedAt = msg.reasoning_completed_at;
        } else if (msg.reasoning && msg.completed_at) {
          currentRound.reasoningCompletedAt = msg.completed_at;
        }
        currentRound.status = msg.streaming && !durableInterruption ? "streaming" : "complete";
        if (msg.model_id) {
          currentRound.modelId = msg.model_id;
        }
        if (msg.thinking_level !== null) {
          currentRound.thinkingLevel = msg.thinking_level;
        }
        if (msg.tokens_per_second !== null) {
          currentRound.tokensPerSecond = msg.tokens_per_second;
        }
      } else if (msg.role === "tool" && msg.tool_call_id) {
        if (msg.content.trim() === "User cancelled the request") {
          markInterrupted(currentRound, "cancelled");
        }
        const entry = currentRound.toolCallMap[msg.tool_call_id];
        if (entry) {
          applyToolResultMetadata(record, entry, toolResultFromMessage(msg));
        } else {
          const result = toolResultFromMessage(msg);
          const entry: ToolCallEntry = {
            id: msg.tool_call_id,
            name: msg.tool_name || "unknown",
            arguments: "",
            status: toolResultStatus(result),
            result,
            childSessionId: record.app_data.child_session_id ?? undefined,
          };
          currentRound.toolCallMap[msg.tool_call_id] = entry;
          currentRound.segments.push({
            type: "tool_call",
            toolCallId: msg.tool_call_id,
          });
        }
      } else if (msg.role === "error") {
        markInterrupted(currentRound, interruptionKind(record) ?? "failed");
        const providerError = record.app_data.provider_error;
        if (providerError) {
          currentRound.providerErrors.push({
            id: msg.id,
            data: {
              ...providerError,
              message: msg.content || providerError.message,
            },
          });
        }
      }
    }
  }

  for (const item of rounds) {
    if ("kind" in item || !item.interrupted) continue;
    const terminalStatus: ToolCallStatus =
      item.interruptionKind === "failed" ? "failed" : "cancelled";
    for (const entry of Object.values(item.toolCallMap)) {
      if (!entry.result && (entry.status === "pending" || entry.status === "running")) {
        entry.status = terminalStatus;
      }
    }
  }

  return rounds;
}
