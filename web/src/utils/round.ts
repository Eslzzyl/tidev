import type { Message } from "../types/api";
import type {
  Round,
  ToolCallEntry,
  RoundSegment,
  SystemMessageBlock,
  ShellBlock,
} from "../types/round";

/**
 * Build a list of Rounds, SystemMessageBlocks and ShellBlocks
 * from a flat list of Messages.
 */
export function buildRounds(messages: Message[]): (Round | SystemMessageBlock | ShellBlock)[] {
  const rounds: (Round | SystemMessageBlock | ShellBlock)[] = [];
  let currentRound: Round | null = null;
  let pendingShellCmd: Message | null = null;

  for (const msg of messages) {
    // ── Shell command (starts with "$ ") ──
    if (msg.role === "shell" && msg.content.startsWith("$ ")) {
      pendingShellCmd = msg;
      continue;
    }

    // ── Shell output (pairs with pending shell command) ──
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

    // ── User message ──
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
    } else if (msg.role === "shell") {
      // Orphan shell message (no preceding command) — render as system block
      rounds.push({
        id: `system-${msg.id}`,
        message: msg,
        kind: "system",
      });
    }
  }

  return rounds;
}

/**
 * Extract exit code from shell output content.
 * Looks for "Exit code: N" at the end of the content.
 */
function parseExitCode(content: string): number | null {
  const match = content.match(/Exit code:\s*(-?\d+)/);
  return match ? parseInt(match[1], 10) : null;
}

export type { Round, ToolCallEntry, RoundSegment, SystemMessageBlock };
