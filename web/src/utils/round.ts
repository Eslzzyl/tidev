import type { Message } from "../types/api";
import type {
  Round,
  ToolCallEntry,
  RoundSegment,
  SystemMessageBlock,
} from "../types/round";

/**
 * Build a list of Rounds and SystemMessageBlocks from a flat list of Messages.
 */
export function buildRounds(
  messages: Message[],
): (Round | SystemMessageBlock)[] {
  const rounds: (Round | SystemMessageBlock)[] = [];
  let currentRound: Round | null = null;

  for (const msg of messages) {
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
      // Standalone system message (e.g. compaction).
      // Do NOT reset currentRound here: subsequent assistant/tool messages
      // that belong to the preceding user round must still be grouped together.
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
          const lastSeg =
            currentRound.segments[currentRound.segments.length - 1];
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
      } else if (msg.role === "tool" && msg.tool_call_id) {
        const entry = currentRound.toolCallMap[msg.tool_call_id];
        if (entry) {
          entry.result = {
            output: msg.content,
            diff: msg.diff,
            filepath: msg.filepath,
            rtk_rewritten: msg.rtk_rewritten ?? false,
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
              diff: msg.diff,
              filepath: msg.filepath,
              rtk_rewritten: msg.rtk_rewritten ?? false,
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
    } else {
      // Unknown role (error, shell, etc.) — render as a system message
      // so it doesn't appear as a spurious user round.
      rounds.push({
        id: `system-${msg.id}`,
        message: msg,
        kind: "system",
      });
    }
  }

  return rounds;
}

export type { Round, ToolCallEntry, RoundSegment, SystemMessageBlock };
