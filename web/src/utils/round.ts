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

export interface Round {
  id: string;
  userMessage: Message;
  segments: RoundSegment[];
  toolCallMap: Record<string, ToolCallEntry>;
  status: "user_only" | "streaming" | "complete";
  completedAt?: string;
  modelName?: string;
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
        const entry = currentRound.toolCallMap[msg.tool_call_id];
        if (entry) {
          entry.result = {
            output: msg.content,
          };
          entry.resultComplete = true;
        } else {
          const entry: ToolCallEntry = {
            id: msg.tool_call_id,
            name: msg.tool_name || "unknown",
            arguments: "",
            argumentsComplete: true,
            result: {
              output: msg.content,
            },
            resultComplete: true,
          };
          currentRound.toolCallMap[msg.tool_call_id] = entry;
          currentRound.segments.push({
            type: "tool_call",
            toolCallId: msg.tool_call_id,
          });
        }
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
