import { create } from "zustand";
import { v4 as uuidv4 } from "uuid";
import { api } from "../api/client";
import { useUIStore } from "./useUIStore";
import { TerminalConnection } from "../terminal/connection";

export interface TerminalTab {
  id: string;
  sessionId: string | null;
  label: string;
  connection: TerminalConnection | null;
  lifecycle: "idle" | "connecting" | "running" | "exited";
}

interface TerminalStore {
  tabs: TerminalTab[];
  activeTabId: string | null;

  createTab: () => string;
  startSession: (tabId: string, cols: number, rows: number) => Promise<string | null>;
  closeTab: (id: string) => Promise<void>;
  setActiveTab: (id: string | null) => void;
  setLifecycle: (tabId: string, lifecycle: TerminalTab["lifecycle"]) => void;
  restoreRunningSessions: () => Promise<void>;

  /** Ctrl latch state for mobile touch keyboard */
  ctrlLatch: boolean;
  setCtrlLatch: (v: boolean) => void;
}

export const useTerminalStore = create<TerminalStore>((set, get) => ({
  tabs: [],
  activeTabId: null,

  createTab: () => {
    const id = uuidv4();
    const label = `Terminal ${get().tabs.length + 1}`;

    set((state) => ({
      tabs: [...state.tabs, { id, sessionId: null, label, connection: null, lifecycle: "idle" }],
      activeTabId: id,
    }));

    return id;
  },

  startSession: async (tabId, cols, rows) => {
    try {
      const { settings } = useUIStore.getState();
      const shell = settings.terminalShell || undefined;
      const result = await api.startTerminal(cols, rows, shell);
      const sessionId = result.session_id;

      // Create connection with the server-assigned session ID
      const conn = new TerminalConnection(sessionId);
      conn.onStatusChange((status) => {
        if (status === "connected") {
          get().setLifecycle(tabId, "running");
        } else if (status === "disconnected") {
          get().setLifecycle(tabId, "exited");
        }
      });

      // Start connecting
      conn.connect();

      set((state) => ({
        tabs: state.tabs.map((t) =>
          t.id === tabId
            ? { ...t, sessionId, connection: conn, lifecycle: "connecting" }
            : t,
        ),
      }));

      return sessionId;
    } catch {
      set((state) => ({
        tabs: state.tabs.map((t) =>
          t.id === tabId
            ? { ...t, lifecycle: "exited" }
            : t,
        ),
      }));
      return null;
    }
  },

  closeTab: async (id) => {
    const tab = get().tabs.find((t) => t.id === id);
    if (tab?.connection) {
      tab.connection.disconnect();
    }
    if (tab?.sessionId) {
      try {
        await api.closeTerminal(tab.sessionId);
      } catch {
        // Ignore close errors
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

  setLifecycle: (tabId, lifecycle) => {
    set((state) => ({
      tabs: state.tabs.map((t) =>
        t.id === tabId ? { ...t, lifecycle } : t,
      ),
    }));
  },

  restoreRunningSessions: async () => {
    try {
      const { sessions } = await api.listTerminals();
      if (sessions.length === 0) return;

      const newTabs: TerminalTab[] = sessions.map((sessionId, i) => {
        const id = uuidv4();
        const conn = new TerminalConnection(sessionId);
        conn.onStatusChange((status) => {
          if (status === "connected") {
            get().setLifecycle(id, "running");
          } else if (status === "disconnected") {
            get().setLifecycle(id, "exited");
          }
        });
        conn.connect();
        return {
          id,
          sessionId,
          label: `Terminal ${i + 1}`,
          connection: conn,
          lifecycle: "connecting" as const,
        };
      });

      set({ tabs: newTabs, activeTabId: newTabs[0].id });
    } catch {
      // Server unavailable or no running sessions — do nothing
    }
  },

  ctrlLatch: false,
  setCtrlLatch: (v) => set({ ctrlLatch: v }),
}));
