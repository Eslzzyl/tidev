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
  ws: WebSocket | null;

  createTab: () => string;
  initSession: (tabId: string, cols: number, rows: number) => Promise<void>;
  closeTab: (id: string) => Promise<void>;
  setActiveTab: (id: string | null) => void;
  appendOutput: (sessionId: string, data: string) => void;
  closeBySessionId: (sessionId: string) => void;
  sendInput: (tabId: string, data: string) => Promise<void>;
  sendResize: (tabId: string, cols: number, rows: number) => Promise<void>;
  connectSSE: (sessionId: string, tabId: string) => void;
  connectWS: (sessionId: string, tabId: string) => void;
  disconnect: () => void;
}

export const useTerminalStore = create<TerminalStore>((set, get) => ({
  tabs: [],
  activeTabId: null,
  eventSource: null,
  ws: null,

  createTab: () => {
    const id = crypto.randomUUID();
    const label = `Terminal ${get().tabs.length + 1}`;

    set((state) => ({
      tabs: [
        ...state.tabs,
        { id, sessionId: null, label, buffer: "", lifecycle: "idle" },
      ],
      activeTabId: id,
    }));

    return id;
  },

  initSession: async (tabId, cols, rows) => {
    try {
      const result = await api.startTerminal(cols, rows);
      set((state) => ({
        tabs: state.tabs.map((t) =>
          t.id === tabId
            ? { ...t, sessionId: result.session_id, lifecycle: "running" as const }
            : t,
        ),
      }));

      // Connect via WebSocket (primary), fall back to SSE
      get().connectWS(result.session_id, tabId);
    } catch (err) {
      set((state) => ({
        tabs: state.tabs.map((t) =>
          t.id === tabId
            ? {
                ...t,
                lifecycle: "exited" as const,
                buffer: t.buffer + "\r\nFailed to start terminal\r\n",
              }
            : t,
        ),
      }));
    }
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

    const ws = get().ws;
    // Try WebSocket first
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(data);
      return;
    }

    // Fallback to HTTP
    try {
      await api.terminalInput(tab.sessionId, data);
    } catch (err) {
      console.error("Failed to send terminal input:", err);
    }
  },

  sendResize: async (tabId, cols, rows) => {
    const tab = get().tabs.find((t) => t.id === tabId);
    if (!tab?.sessionId) return;

    const ws = get().ws;
    // Try WebSocket first (control frame)
    if (ws && ws.readyState === WebSocket.OPEN) {
      const ctrl = new TextEncoder().encode(
        `\x01${JSON.stringify({ type: "resize", cols, rows })}`,
      );
      ws.send(ctrl);
      return;
    }

    // Fallback to HTTP
    try {
      await api.terminalResize(tab.sessionId, cols, rows);
    } catch (err) {
      console.error("Failed to resize terminal:", err);
    }
  },

  connectSSE: (sessionId, tabId) => {
    get().disconnect();

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

  connectWS: (sessionId, tabId) => {
    get().disconnect();

    // Try WebSocket
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const wsUrl = `${protocol}//${window.location.host}/api/terminal/ws`;

    try {
      const ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        // Send bind control frame
        const bindMsg = new TextEncoder().encode(
          `\x01${JSON.stringify({ type: "bind", session_id: sessionId })}`,
        );
        ws.send(bindMsg);
      };

      ws.onmessage = (e) => {
        const data = e.data as string;

        // Check for control frames (0x01 prefix)
        if (data.charCodeAt(0) === 0x01) {
          try {
            const ctrl = JSON.parse(data.slice(1));
            if (ctrl.type === "close") {
              get().closeBySessionId(sessionId);
              get().disconnect();
            }
            // "ok" is ack, no action needed
          } catch {
            // Invalid control frame, ignore
          }
          return;
        }

        // Raw text = terminal output
        get().appendOutput(sessionId, data);
      };

      ws.onerror = () => {
        // WebSocket failed, fall back to SSE
        get().disconnect();
        get().connectSSE(sessionId, tabId);
      };

      ws.onclose = () => {
        // If the tab is still running, we might need to reconnect.
        // But since we manage lifecycle via terminal.close event,
        // we don't auto-reconnect here.
        set({ ws: null });
      };

      set({ ws });
    } catch {
      // WebSocket connection failed, fall back to SSE
      get().connectSSE(sessionId, tabId);
    }
  },

  disconnect: () => {
    const es = get().eventSource;
    if (es) {
      es.close();
      set({ eventSource: null });
    }
    const ws = get().ws;
    if (ws) {
      ws.close();
      set({ ws: null });
    }
  },
}));
