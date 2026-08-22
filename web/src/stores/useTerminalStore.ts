import { create } from "zustand";
import { v4 as uuidv4 } from "uuid";
import { api } from "../api/client";
import { queryClient } from "../lib/queryClient";
import { useUIStore } from "./useUIStore";
import { TerminalConnection } from "../terminal/connection";
import i18n from "../i18n";

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
  closeTabs: (ids: string[]) => Promise<void>;
  renameTab: (id: string, label: string) => void;
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
    const label = i18n.t("Terminal");

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
      const tab = get().tabs.find((t) => t.id === tabId);
      const label = tab?.label ?? i18n.t("Terminal");
      const result = await api.startTerminal(cols, rows, shell, label);
      queryClient.invalidateQueries({ queryKey: ["terminal", "list"] });
      const sessionId = result.session_id;

      // Restty owns the PTY transport connection once its viewport mounts.
      const conn = new TerminalConnection(sessionId);
      conn.onStatusChange((status) => {
        if (status === "connected") {
          get().setLifecycle(tabId, "running");
        } else if (status === "disconnected") {
          get().setLifecycle(tabId, "exited");
        }
      });

      set((state) => ({
        tabs: state.tabs.map((t) =>
          t.id === tabId ? { ...t, sessionId, connection: conn, lifecycle: "connecting" } : t,
        ),
      }));

      return sessionId;
    } catch {
      set((state) => ({
        tabs: state.tabs.map((t) => (t.id === tabId ? { ...t, lifecycle: "exited" } : t)),
      }));
      return null;
    }
  },

  closeTab: async (id) => {
    const tab = get().tabs.find((t) => t.id === id);
    if (tab?.connection) {
      tab.connection.dispose();
    }
    if (tab?.sessionId) {
      try {
        await api.closeTerminal(tab.sessionId);
        queryClient.invalidateQueries({ queryKey: ["terminal", "list"] });
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

  closeTabs: async (ids) => {
    const tabs = get().tabs;
    const toClose = tabs.filter((t) => ids.includes(t.id));
    const idSet = new Set(ids);

    // Release all connections synchronously
    for (const tab of toClose) {
      tab.connection?.dispose();
    }

    // Close all sessions on server
    await Promise.allSettled(
      toClose.map((tab) => (tab.sessionId ? api.closeTerminal(tab.sessionId) : Promise.resolve())),
    );
    queryClient.invalidateQueries({ queryKey: ["terminal", "list"] });

    set((state) => {
      const remaining = state.tabs.filter((t) => !idSet.has(t.id));
      return {
        tabs: remaining,
        activeTabId: remaining.length > 0 ? remaining[remaining.length - 1].id : null,
      };
    });
  },

  renameTab: (id, label) => {
    set((state) => ({
      tabs: state.tabs.map((t) => (t.id === id ? { ...t, label } : t)),
    }));
    // Persist rename to server
    const tab = get().tabs.find((t) => t.id === id);
    if (tab?.sessionId) {
      api
        .renameTerminal(tab.sessionId, label)
        .then(() => {
          queryClient.invalidateQueries({ queryKey: ["terminal", "list"] });
        })
        .catch(() => {
          // Ignore rename errors — local state is already updated
        });
    }
  },

  setActiveTab: (id) => set({ activeTabId: id }),

  setLifecycle: (tabId, lifecycle) => {
    set((state) => ({
      tabs: state.tabs.map((t) => (t.id === tabId ? { ...t, lifecycle } : t)),
    }));
  },

  restoreRunningSessions: async () => {
    try {
      const { sessions } = await queryClient.fetchQuery({
        queryKey: ["terminal", "list"],
        queryFn: api.listTerminals,
      });
      if (sessions.length === 0) return;

      const newTabs: TerminalTab[] = [];
      for (const entry of sessions) {
        // Defensive: skip entries without a valid session_id
        if (!entry.session_id) continue;
        const id = uuidv4();
        const conn = new TerminalConnection(entry.session_id);
        conn.onStatusChange((status) => {
          if (status === "connected") {
            get().setLifecycle(id, "running");
          } else if (status === "disconnected") {
            get().setLifecycle(id, "exited");
          }
        });
        newTabs.push({
          id,
          sessionId: entry.session_id,
          label: entry.label || i18n.t("Terminal"),
          connection: conn,
          lifecycle: "connecting" as const,
        });
      }

      if (newTabs.length === 0) return;
      set({ tabs: newTabs, activeTabId: newTabs[0].id });
    } catch {
      // Server unavailable or no running sessions — do nothing
    }
  },

  ctrlLatch: false,
  setCtrlLatch: (v) => set({ ctrlLatch: v }),
}));
