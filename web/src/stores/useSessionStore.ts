import { create } from "zustand";
import type { Session, SessionDetail, Message, TodoItem } from "../types/api";

export type SessionMode = "plan" | "build";

export interface UsageStatsData {
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  tokens_per_second?: number;
}

export interface SessionState {
  sessions: Session[];
  currentSessionId: string | null;
  currentSession: SessionDetail | null;
  messages: Message[];
  todos: TodoItem[];
  currentUsageStats: UsageStatsData | null;
  isLoading: boolean;
  error: string | null;
  isDraftSession: boolean;
  draftTitle: string;
  mode: SessionMode;
  currentRequestId: number | null;
}

export interface SessionActions {
  setSessions: (sessions: Session[]) => void;
  setCurrentSession: (session: SessionDetail | null) => void;
  setCurrentSessionId: (id: string | null) => void;
  setMessages: (messages: Message[]) => void;
  setTodos: (todos: TodoItem[]) => void;
  setCurrentUsageStats: (stats: UsageStatsData | null) => void;
  addMessage: (message: Message) => void;
  updateMessageContent: (id: string, content: string) => void;
  setLoading: (isLoading: boolean) => void;
  setError: (error: string | null) => void;
  clearError: () => void;
  removeSession: (sessionId: string) => void;
  setMode: (mode: SessionMode) => void;
  toggleMode: () => void;
  startDraftSession: (title?: string) => void;
  commitDraftSession: (session: SessionDetail) => void;
  cancelDraftSession: () => void;
  setCurrentRequestId: (id: number | null) => void;
  goToWelcome: () => void;
  reset: () => void;
}

const initialState: SessionState = {
  sessions: [],
  currentSessionId: null,
  currentSession: null,
  messages: [],
  todos: [],
  currentUsageStats: null,
  isLoading: false,
  error: null,
  isDraftSession: false,
  draftTitle: "",
  mode: "build",
  currentRequestId: null,
};
export const useSessionStore = create<SessionState & SessionActions>((set) => ({
  ...initialState,

  setSessions: (sessions) => set({ sessions, error: null }),

  setCurrentSession: (session) =>
    set({
      currentSession: session,
      currentSessionId: session?.session_id ?? null,
      isDraftSession: false,
    }),

  setCurrentSessionId: (id) =>
    set({ currentSessionId: id, currentRequestId: null }),

  setMessages: (messages) => set({ messages }),

  setTodos: (todos) => set({ todos }),

  setCurrentUsageStats: (stats) => set({ currentUsageStats: stats }),

  addMessage: (message) =>
    set((state) => ({ messages: [...state.messages, message] })),

  updateMessageContent: (id, content) =>
    set((state) => ({
      messages: state.messages.map((m) =>
        m.id === id ? { ...m, content: m.content + content } : m,
      ),
    })),

  setLoading: (isLoading) => set({ isLoading }),
  setError: (error) => set({ error }),
  clearError: () => set({ error: null }),

  removeSession: (sessionId) =>
    set((state) => ({
      sessions: state.sessions.filter((s) => s.session_id !== sessionId),
      currentSessionId:
        state.currentSessionId === sessionId ? null : state.currentSessionId,
      currentSession:
        state.currentSessionId === sessionId ? null : state.currentSession,
      isDraftSession:
        state.currentSessionId === sessionId ? false : state.isDraftSession,
    })),

  setMode: (mode) => set({ mode }),
  toggleMode: () =>
    set((state) => ({ mode: state.mode === "plan" ? "build" : "plan" })),

  startDraftSession: (title = "New Session") =>
    set({
      currentSessionId: null,
      currentSession: null,
      messages: [],
      isDraftSession: true,
      draftTitle: title,
      error: null,
    }),

  commitDraftSession: (session) =>
    set({
      currentSessionId: session.session_id,
      currentSession: session,
      isDraftSession: false,
      draftTitle: "",
    }),

  cancelDraftSession: () => set({ isDraftSession: false, draftTitle: "" }),

  setCurrentRequestId: (id) => set({ currentRequestId: id }),

  goToWelcome: () =>
    set({
      currentSessionId: null,
      currentSession: null,
      messages: [],
      isDraftSession: false,
      draftTitle: "",
      error: null,
      currentRequestId: null,
    }),

  reset: () => set(initialState),
}));
