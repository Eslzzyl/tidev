import { create } from "zustand";
import type { Message } from "../types/api";

/** A single block of streaming subagent content, rendered as one UI unit */
export interface SubagentStreamBlock {
  type: "reasoning" | "content" | "tool_call";
  /** Text content for reasoning/content blocks */
  content?: string;
  /** Tool name for tool_call blocks */
  toolName?: string;
  /** Tool arguments JSON for tool_call blocks */
  toolArgs?: string;
  /** Whether this tool call has finished executing */
  complete?: boolean;
}

export interface SubagentState {
  /** Child session ID for navigation */
  childSessionId?: string;
  /** Live status text (e.g. "Thinking...", "Searching files...") */
  statusText?: string;
  /** Ordered list of streaming blocks for the real-time view */
  blocks: SubagentStreamBlock[];
  /** Whether the subagent has completed */
  completed: boolean;
  /** Error message if the subagent failed */
  error?: string;
  /** Cached child session messages (fetched from API after completion) */
  messages?: Message[];
  /** Whether messages are currently being fetched */
  messagesLoading?: boolean;
}

interface SubagentStore {
  /** Map of tool_call_id → subagent state */
  states: Record<string, SubagentState>;
  /** Update or initialize state for a tool call */
  updateState: (toolCallId: string, update: Partial<SubagentState>) => void;
  /** Remove state (cleanup) */
  removeState: (toolCallId: string) => void;
  /** Get state for a tool call */
  getState: (toolCallId: string) => SubagentState | undefined;
}

function initialState(): SubagentState {
  return { completed: false, blocks: [] };
}

export const useSubagentStore = create<SubagentStore>((set, get) => ({
  states: {},

  updateState: (toolCallId, update) => {
    set((state) => ({
      states: {
        ...state.states,
        [toolCallId]: {
          ...(state.states[toolCallId] || initialState()),
          ...update,
        },
      },
    }));
  },

  removeState: (toolCallId) => {
    set((state) => {
      const next = { ...state.states };
      delete next[toolCallId];
      return { states: next };
    });
  },

  getState: (toolCallId) => {
    return get().states[toolCallId];
  },
}));
