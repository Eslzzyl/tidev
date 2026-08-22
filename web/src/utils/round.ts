import type { Message, MessageRecord } from "../types/api";

export interface ToolCallEntry {
  id: string;
  name: string;
  arguments: string;
  argumentsComplete: boolean;
  result?: {
    output: string;
    diff?: string;
    filepath?: string;
    rtk_rewritten?: boolean;
    isError?: boolean;
    exitCode?: number | null;
  };
  resultComplete: boolean;
  childSessionId?: string;
  subagentStatus?: string;
  subagentContentDelta?: string;
  subagentReasoningDelta?: string;
}

export type RoundSegment =
  | { type: "text"; content: string }
  | { type: "reasoning"; content: string }
  | { type: "tool_call"; toolCallId: string };

export type RoundInterruptionKind = "cancelled" | "failed" | "interrupted";

export interface Round {
  id: string;
  userMessage: Message;
  segments: RoundSegment[];
  toolCallMap: Record<string, ToolCallEntry>;
  status: "user_only" | "streaming" | "complete";
  interrupted: boolean;
  interruptionKind?: RoundInterruptionKind;
  completedAt?: string;
  modelName?: string;
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
 * Decide whether a terminal round should render in its compact preview state.
 * Interrupted rounds use the same compact treatment as successful rounds, but
 * keep a status-only preview when no user-facing text was produced.
 */
export function isRoundCollapsible(round: Round): boolean {
  if (round.status !== "complete" || !round.segments.length) return false;
  if (round.interrupted) {
    const previewIndex = getRoundPreviewIndex(round);
    return round.segments.length > 1 || previewIndex === null;
  }

  const previewIndex = getRoundPreviewIndex(round);
  return Boolean(round.completedAt && previewIndex !== null && previewIndex > 0);
}

function markInterrupted(round: Round, kind: RoundInterruptionKind): void {
  round.interrupted = true;
  round.interruptionKind ??= kind;
}

export interface SystemMessageBlock {
  id: string;
  message: Message;
  kind: "system";
}

export interface ShellBlock {
  id: string;
  command: Message;
  output: Message;
  exitCode: number | null;
  kind: "shell";
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
  output: string,
): void {
  entry.result = {
    output,
    diff: record.message.diff,
    filepath: record.message.filepath,
    rtk_rewritten: record.message.rtk_rewritten,
  };
  entry.childSessionId = record.app_data.child_session_id ?? entry.childSessionId;
}

function parseExitCode(content: string): number | null {
  const match = content.match(/Exit code:\s*(-?\d+)/);
  return match ? parseInt(match[1], 10) : null;
}

/**
 * Build a list of Rounds, SystemMessageBlocks and ShellBlocks
 * from a flat list of MessageRecords.
 *
 * Adapted from last-full web/src/utils/round.ts to work with the
 * new tidev-core message shape (MessageRecord wraps Message + app_data).
 */
export function buildRounds(records: MessageRecord[]): (Round | SystemMessageBlock | ShellBlock)[] {
  const rounds: (Round | SystemMessageBlock | ShellBlock)[] = [];
  let currentRound: Round | null = null;
  let pendingShellCmd: Message | null = null;

  for (const record of records) {
    const msg = unwrapMessage(record);

    if (msg.role === "shell" && msg.content.startsWith("$ ")) {
      pendingShellCmd = msg;
      continue;
    }

    if (msg.role === "shell" && pendingShellCmd) {
      const exitCode = parseExitCode(msg.content);
      rounds.push({
        id: `shell-${pendingShellCmd.id}`,
        command: pendingShellCmd,
        output: msg,
        exitCode,
        kind: "shell",
      });
      pendingShellCmd = null;
      continue;
    }

    if (msg.role === "user") {
      currentRound = {
        id: `round-${msg.id}`,
        userMessage: msg,
        segments: [],
        toolCallMap: {},
        status: "user_only",
        interrupted: false,
      };
      rounds.push(currentRound);
    } else if (msg.role === "system") {
      rounds.push({
        id: `system-${msg.id}`,
        message: msg,
        kind: "system",
      });
    } else if (currentRound) {
      if (msg.role === "assistant") {
        if (!msg.streaming && !msg.completed_at) {
          markInterrupted(currentRound, "interrupted");
        }

        if (msg.reasoning) {
          currentRound.segments.push({
            type: "reasoning",
            content: msg.reasoning,
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
              const entry: ToolCallEntry = {
                id: tc.id,
                name: tc.name,
                arguments: tc.arguments,
                argumentsComplete: true,
                resultComplete: false,
              };
              currentRound.toolCallMap[tc.id] = entry;
              currentRound.segments.push({
                type: "tool_call",
                toolCallId: tc.id,
              });
            } else {
              existing.arguments = tc.arguments;
              existing.argumentsComplete = true;
            }
          }
        }

        if (msg.completed_at) {
          currentRound.completedAt = msg.completed_at;
        }
        currentRound.status = msg.streaming ? "streaming" : "complete";
        if (msg.model_id) {
          currentRound.modelName = msg.model_id;
        }
      } else if (msg.role === "tool" && msg.tool_call_id) {
        if (msg.content.trim() === "User cancelled the request") {
          markInterrupted(currentRound, "cancelled");
        }
        const entry = currentRound.toolCallMap[msg.tool_call_id];
        if (entry) {
          applyToolResultMetadata(record, entry, msg.content);
          entry.resultComplete = true;
        } else {
          const entry: ToolCallEntry = {
            id: msg.tool_call_id,
            name: msg.tool_name || "unknown",
            arguments: "",
            argumentsComplete: true,
            result: { output: msg.content, diff: msg.diff, filepath: msg.filepath },
            resultComplete: true,
            childSessionId: record.app_data.child_session_id ?? undefined,
          };
          currentRound.toolCallMap[msg.tool_call_id] = entry;
          currentRound.segments.push({
            type: "tool_call",
            toolCallId: msg.tool_call_id,
          });
        }
      } else if (msg.role === "error") {
        markInterrupted(currentRound, "failed");
      }
    } else if (msg.role === "shell") {
      rounds.push({
        id: `system-${msg.id}`,
        message: msg,
        kind: "system",
      });
    }
  }

  return rounds;
}
