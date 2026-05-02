import type { Message } from './api';

/**
 * A single tool call within an assistant's turn, with streaming state.
 */
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
}

/**
 * A segment of assistant response content.
 */
export type RoundSegment =
  | { type: 'text'; content: string }
  | { type: 'reasoning'; content: string }
  | { type: 'tool_call'; toolCallId: string };

/**
 * One user→assistant round.
 */
export interface Round {
  id: string;
  userMessage: Message;
  segments: RoundSegment[];
  toolCallMap: Record<string, ToolCallEntry>;
  status: 'user_only' | 'streaming' | 'complete';
  completedAt?: string;
  modelName?: string;
}

/**
 * Get all tool call entries from a round in segment order.
 */
export function orderedToolCalls(round: Round): ToolCallEntry[] {
  const result: ToolCallEntry[] = [];
  for (const seg of round.segments) {
    if (seg.type === 'tool_call') {
      const entry = round.toolCallMap[seg.toolCallId];
      if (entry) result.push(entry);
    }
  }
  return result;
}
