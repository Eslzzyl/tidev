import { create } from "zustand";
import { api } from "../api/client";

export interface TerminalTab {
  id: string;
  sessionId: string | null;
  label: string;
  buffer: string;
  lifecycle: "idle" | "running" | "exited";
}

interface TerminalStore {
  tabs: TerminalTab[];
  activeTabId: string | null;
  eventSource: EventSource | null;

  createTab: () => Promise<string>;
  closeTab: (id: string) => Promise<void>;
  setActiveTab: (id: string | null) => void;
  appendOutput: (sessionId: string, data: string) => void;
  closeBySessionId: (sessionId: string) => void;
  sendInput: (tabId: string, data: string) => Promise<void>;
  connectSSE: (sessionId: string, tabId: string) => void;
  disconnectSSE: () => void;
}

export const useTerminalStore = create<TerminalStore>((set, get) => ({
  tabs: [],
  activeTabId: null,
  eventSource: null,

  createTab: async () => {
    const id = crypto.randomUUID();
    const label = `Terminal ${get().tabs.length + 1}`;

    set((state) => ({
      tabs: [
        ...state.tabs,
        { id, sessionId: null, label, buffer: "", lifecycle: "idle" },
      ],
      activeTabId: id,
    }));

    // Start terminal session
    try {
      const result = await api.startTerminal();
      set((state) => ({
        tabs: state.tabs.map((t) =>
          t.id === id
            ? { ...t, sessionId: result.session_id, lifecycle: "running" as const }
            : t,
        ),
      }));

      // Connect SSE for this session
      get().connectSSE(result.session_id, id);
    } catch (err) {
      set((state) => ({
        tabs: state.tabs.map((t) =>
          t.id === id
            ? { ...t, lifecycle: "exited" as const, buffer: t.buffer + "\r\nFailed to start terminal\r\n" }
            : t,
        ),
      }));
    }

    return id;
  },

  closeTab: async (id: string) => {
    const tab = get().tabs.find((t) => t.id === id);
    if (tab?.sessionId) {
      try {
        await api.closeTerminal(tab.sessionId);
      } catch {
        // Ignore errors on close
      }
    }

    set((state) => {
      const remaining = state.tabs.filter((t) => t.id !== id);
      return {
        tabs: remaining,
        activeTabId:
          state.activeTabId === id
            ? remaining.length > 0
              ? remaining[remaining.length - 1].id
              : null
            : state.activeTabId,
      };
    });
  },

  setActiveTab: (id) => set({ activeTabId: id }),

  appendOutput: (sessionId, data) => {
    set((state) => ({
      tabs: state.tabs.map((t) =>
        t.sessionId === sessionId
          ? { ...t, buffer: t.buffer + data }
          : t,
      ),
    }));
  },

  closeBySessionId: (sessionId) => {
    set((state) => ({
      tabs: state.tabs.map((t) =>
        t.sessionId === sessionId
          ? { ...t, lifecycle: "exited" as const }
          : t,
      ),
    }));
  },

  sendInput: async (tabId, data) => {
    const tab = get().tabs.find((t) => t.id === tabId);
    if (!tab?.sessionId) return;

    try {
      await api.terminalInput(tab.sessionId, data);
    } catch (err) {
      console.error("Failed to send terminal input:", err);
    }
  },

  connectSSE: (sessionId, tabId) => {
    get().disconnectSSE();

    const url = `${window.location.origin}/api/terminal/events?session_id=${sessionId}`;
    const es = new EventSource(url);

    es.addEventListener("terminal.output", (e: MessageEvent) => {
      get().appendOutput(sessionId, e.data);
    });

    es.addEventListener("terminal.close", () => {
      get().closeBySessionId(sessionId);
      es.close();
    });

    es.onerror = () => {
      es.close();
    };

    set({ eventSource: es });
  },

  disconnectSSE: () => {
    const es = get().eventSource;
    if (es) {
      es.close();
      set({ eventSource: null });
    }
  },
}));
