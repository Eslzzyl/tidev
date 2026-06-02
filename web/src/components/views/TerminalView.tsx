import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Plus, X } from "lucide-react";
import { useTerminalStore } from "../../stores/useTerminalStore";
import { useUIStore, getEffectiveTheme } from "../../stores/useUIStore";
import type { TerminalConnection } from "../../terminal/connection";
import "@xterm/xterm/css/xterm.css";

/** Dark terminal theme */
const DARK_THEME = {
  background: "#000000",
  foreground: "#f0f0f0",
  cursor: "#f0f0f0",
  selectionBackground: "#ffffff33",
  black: "#2e3436",
  red: "#cc0000",
  green: "#4e9a06",
  yellow: "#c4a000",
  blue: "#3465a4",
  magenta: "#75507b",
  cyan: "#06989a",
  white: "#d3d7cf",
  brightBlack: "#555753",
  brightRed: "#ef2929",
  brightGreen: "#8ae234",
  brightYellow: "#fce94f",
  brightBlue: "#729fcf",
  brightMagenta: "#ad7fa8",
  brightCyan: "#34e2e2",
  brightWhite: "#eeeeee",
};

/** Light terminal theme */
const LIGHT_THEME = {
  background: "#fefefe",
  foreground: "#1a1a1a",
  cursor: "#1a1a1a",
  selectionBackground: "#8888ff33",
  black: "#1a1a1a",
  red: "#b91c1c",
  green: "#15803d",
  yellow: "#a16207",
  blue: "#1d4ed8",
  magenta: "#7c3aed",
  cyan: "#0e7490",
  white: "#d4d4d4",
  brightBlack: "#525252",
  brightRed: "#ef4444",
  brightGreen: "#22c55e",
  brightYellow: "#eab308",
  brightBlue: "#3b82f6",
  brightMagenta: "#a855f7",
  brightCyan: "#06b6d4",
  brightWhite: "#f5f5f5",
};

export function TerminalView() {
  const tabs = useTerminalStore((s) => s.tabs);
  const activeTabId = useTerminalStore((s) => s.activeTabId);
  const createTab = useTerminalStore((s) => s.createTab);
  const closeTab = useTerminalStore((s) => s.closeTab);
  const setActiveTab = useTerminalStore((s) => s.setActiveTab);
  const restoreRunningSessions = useTerminalStore((s) => s.restoreRunningSessions);
  const theme = useUIStore((s) => s.theme);
  const isDark = getEffectiveTheme(theme) === "dark";

  // On mount: restore running sessions from server, or create a new tab
  const restoreAttempted = useRef(false);
  useEffect(() => {
    if (tabs.length > 0) return;
    if (restoreAttempted.current) return;
    restoreAttempted.current = true;

    restoreRunningSessions().then(() => {
      // If no sessions were restored, create a new terminal
      const state = useTerminalStore.getState();
      if (state.tabs.length === 0) {
        createTab();
      }
    });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const activeTab = tabs.find((t) => t.id === activeTabId);

  return (
    <div className={`flex h-full flex-col ${isDark ? "bg-black" : "bg-white"}`}>
      {/* Tab bar */}
      <div
        className={`flex items-center border-b ${isDark ? "border-neutral-800 bg-neutral-950" : "border-neutral-200 bg-neutral-100"}`}
      >
        <div className="flex flex-1 overflow-x-auto">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`flex items-center gap-1 px-3 py-1.5 text-xs font-medium transition-colors ${
                tab.id === activeTabId
                  ? isDark
                    ? "border-b-2 border-blue-500 bg-neutral-900 text-white"
                    : "border-b-2 border-blue-500 bg-white text-black"
                  : isDark
                    ? "text-neutral-400 hover:text-white"
                    : "text-neutral-500 hover:text-black"
              }`}
            >
              <span>{tab.label}</span>
              <X
                size={12}
                className="cursor-pointer opacity-50 hover:opacity-100"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.id);
                }}
              />
            </button>
          ))}
        </div>
        <button
          onClick={() => createTab()}
          className={`mr-1 rounded p-1 transition-colors ${
            isDark ? "text-neutral-400 hover:bg-neutral-800 hover:text-white" : "text-neutral-500 hover:bg-neutral-200 hover:text-black"
          }`}
        >
          <Plus size={14} />
        </button>
      </div>

      {/* Terminal viewport */}
      <div className="flex-1 overflow-hidden">
        {tabs.map((tab) => (
          <div
            key={tab.id}
            className={tab.id === activeTabId ? "block h-full flex-1" : "hidden"}
          >
            {activeTab && <TerminalViewport tab={tab} isDark={isDark} />}
          </div>
        ))}
      </div>
    </div>
  );
}

interface TerminalViewportProps {
  tab: {
    id: string;
    connection: TerminalConnection | null;
    lifecycle: string;
  };
  isDark: boolean;
}

function TerminalViewport({ tab, isDark }: TerminalViewportProps) {
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const bufferRef = useRef<string>("");
  const connRef = useRef<TerminalConnection | null>(null);
  const initStartedRef = useRef(false);

  const startSession = useTerminalStore((s) => s.startSession);

  // Initialize xterm.js and start the terminal session
  useEffect(() => {
    if (!terminalRef.current) return;
    if (xtermRef.current) return;

    const term = new Terminal({
      cursorBlink: true,
      cursorStyle: "block",
      fontSize: 13,
      fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace",
      theme: isDark ? DARK_THEME : LIGHT_THEME,
      allowProposedApi: true,
      cols: 80,
      rows: 24,
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    fitAddonRef.current = fitAddon;

    term.open(terminalRef.current);
    xtermRef.current = term;

    // Handle user input: forward to terminal connection
    term.onData((data: string) => {
      const conn = connRef.current;
      if (conn) {
        conn.sendMessage("stdin", data);
      }
    });

    // Fit terminal to container and notify backend.
    const fitAndInit = () => {
      try {
        fitAddon.fit();
        const cols = term.cols;
        const rows = term.rows;
        if (cols <= 0 || rows <= 0) return;

        if (!initStartedRef.current) {
          initStartedRef.current = true;
          if (tab.connection) {
            // Connection already created and connect() called by store.
            // Just set ref and send initial resize.
            connRef.current = tab.connection;
            tab.connection.sendMessage("resize", rows, cols);
          } else {
            // Start server session + create connection
            startSession(tab.id, cols, rows);
          }
        } else {
          // Already initialized — send resize via connection
          const conn = connRef.current;
          if (conn) {
            conn.sendMessage("resize", rows, cols);
          }
        }
      } catch {
        // Fit errors are non-fatal
      }
    };

    // Wait a tick for layout to settle
    const initTimer = setTimeout(fitAndInit, 50);

    // Observe container for future resize
    const resizeObserver = new ResizeObserver(() => {
      fitAndInit();
    });
    resizeObserver.observe(terminalRef.current);

    return () => {
      clearTimeout(initTimer);
      resizeObserver.disconnect();
      // Remove message handlers from connection
      const conn = connRef.current;
      if (conn) {
        conn.offMessage(liveHandler);
      }
      connRef.current = null;
      term.dispose();
      xtermRef.current = null;
      fitAddonRef.current = null;
      initStartedRef.current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Live handler: forward stdout directly to xterm
  const liveHandler = (msg: { type: string; content: unknown[] }) => {
    const term = xtermRef.current;
    if (!term) return;

    if (msg.type === "stdout") {
      const text = msg.content[0] as string;
      term.write(text);
    } else if (msg.type === "disconnect") {
      term.write("\r\n\x1b[31m[Process exited]\x1b[0m\r\n");
    }
  };

  // Watch for connection changes on the tab
  useEffect(() => {
    const conn = tab.connection;
    if (!conn || conn === connRef.current) return;

    connRef.current = conn;

    // If xterm is already ready, replay buffer and connect live handler
    if (xtermRef.current) {
      // Replay buffered output from the connection
      const term = xtermRef.current;
      if (bufferRef.current) {
        term.write(bufferRef.current);
        bufferRef.current = "";
      }

      // Switch to live handler
      conn.onMessage(liveHandler);
    } else {
      // Not ready yet — buffer will be handled during init
    }
  }, [tab.connection]);

  return (
    <div
      ref={terminalRef}
      className="h-full w-full p-1"
      style={{ background: isDark ? "#000" : "#fefefe" }}
    />
  );
}
