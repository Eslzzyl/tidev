import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Plus, X } from "lucide-react";
import { useTerminalStore } from "../../stores/useTerminalStore";
import { useUIStore, getEffectiveTheme } from "../../stores/useUIStore";
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
  const theme = useUIStore((s) => s.theme);
  const isDark = getEffectiveTheme(theme) === "dark";

  // Create an initial terminal tab on first mount
  useEffect(() => {
    if (tabs.length === 0) {
      createTab();
    }
  }, [createTab, tabs.length]);

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
              className={`flex items-center gap-1 px-3 py-1.5 text-xs transition-colors ${
                tab.id === activeTabId
                  ? isDark
                    ? "border-b-2 border-blue-500 bg-neutral-900 text-white"
                    : "border-b-2 border-blue-600 bg-white text-black"
                  : isDark
                    ? "text-neutral-400 hover:bg-neutral-900 hover:text-white"
                    : "text-neutral-600 hover:bg-neutral-50 hover:text-black"
              }`}
            >
              <span
                className={`h-1.5 w-1.5 rounded-full ${
                  tab.lifecycle === "running"
                    ? "bg-green-500"
                    : tab.lifecycle === "exited"
                      ? "bg-red-500"
                      : "bg-yellow-500"
                }`}
              />
              <span className="truncate max-w-32">{tab.label}</span>
              {tabs.length > 1 && (
                <X
                  size={12}
                  className="ml-0.5 rounded hover:bg-neutral-700"
                  onClick={(e) => {
                    e.stopPropagation();
                    closeTab(tab.id);
                  }}
                />
              )}
            </button>
          ))}
        </div>
        <button
          onClick={() => createTab()}
          className={`mr-1 rounded p-1 transition-colors ${
            isDark
              ? "text-neutral-400 hover:bg-neutral-800 hover:text-white"
              : "text-neutral-500 hover:bg-neutral-200 hover:text-black"
          }`}
        >
          <Plus size={14} />
        </button>
      </div>

      {/* Terminal area */}
      <div className="relative flex-1 overflow-hidden">
        {activeTab ? (
          <TerminalViewport
            key={activeTab.id}
            tab={activeTab}
            isDark={isDark}
          />
        ) : (
          <div
            className={`flex h-full items-center justify-center text-sm ${isDark ? "text-neutral-500" : "text-neutral-400"}`}
          >
            No terminal session
          </div>
        )}
      </div>
    </div>
  );
}

interface TerminalViewportProps {
  tab: {
    id: string;
    sessionId: string | null;
    lifecycle: string;
    buffer: string;
  };
  isDark: boolean;
}

function TerminalViewport({ tab, isDark }: TerminalViewportProps) {
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const writtenLenRef = useRef(0);
  const initStartedRef = useRef(false);
  const sessionIdRef = useRef<string | null>(tab.sessionId);
  const tabIdRef = useRef(tab.id);

  // Keep refs in sync with latest props
  useEffect(() => {
    sessionIdRef.current = tab.sessionId;
  }, [tab.sessionId]);
  useEffect(() => {
    tabIdRef.current = tab.id;
  }, [tab.id]);

  const sendInput = useTerminalStore((s) => s.sendInput);
  const sendResize = useTerminalStore((s) => s.sendResize);
  const initSession = useTerminalStore((s) => s.initSession);

  // Initialize xterm.js and optionally start a PTY session
  useEffect(() => {
    if (!terminalRef.current) return;
    // Prevent double initialization in StrictMode
    if (xtermRef.current) return;

    writtenLenRef.current = 0;

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

    // Handle user input: forward to the session via WebSocket (or HTTP fallback).
    // Uses refs to always see the latest sessionId/tabId without re-registering.
    term.onData((data: string) => {
      const sid = sessionIdRef.current;
      if (sid) {
        sendInput(tabIdRef.current, data);
      }
    });

    // Fit terminal to container and notify backend of actual size.
    // If no session yet, start one with the measured dimensions.
    const fitAndInit = () => {
      try {
        fitAddon.fit();
        const cols = term.cols;
        const rows = term.rows;

        if (cols <= 0 || rows <= 0) return;

        const sid = sessionIdRef.current;
        if (!sid && !initStartedRef.current) {
          // No session yet: start one with the correct size
          initStartedRef.current = true;
          initSession(tabIdRef.current, cols, rows);
        } else if (sid) {
          // Session exists: send resize if dimensions changed
          sendResize(tabIdRef.current, cols, rows);
        }
      } catch {
        // Fit errors are non-fatal
      }
    };

    // Wait a tick for layout to settle, then measure
    const initTimer = setTimeout(fitAndInit, 50);

    // Observe container for future resize
    const resizeObserver = new ResizeObserver(() => {
      fitAndInit();
    });
    resizeObserver.observe(terminalRef.current);

    return () => {
      clearTimeout(initTimer);
      resizeObserver.disconnect();
      term.dispose();
      xtermRef.current = null;
      fitAddonRef.current = null;
      initStartedRef.current = false;
    };
    // This effect intentionally runs once (mount-only).
    // Store functions are stable; refs prevent stale closures.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Stream new buffer content to xterm when buffer grows
  useEffect(() => {
    const term = xtermRef.current;
    if (!term) return;

    const currLen = tab.buffer.length;
    const written = writtenLenRef.current;

    if (currLen > written) {
      const newData = tab.buffer.slice(written);
      term.write(newData);
      writtenLenRef.current = currLen;
    }
  }, [tab.buffer]);

  // Show exit message when session ends
  useEffect(() => {
    const term = xtermRef.current;
    if (!term) return;
    if (tab.lifecycle === "exited") {
      term.write("\r\n\x1b[31m[Process exited]\x1b[0m\r\n");
    }
  }, [tab.lifecycle]);

  return (
    <div
      ref={terminalRef}
      className="h-full w-full p-1"
      style={{ background: isDark ? "#000" : "#fefefe" }}
    />
  );
}
