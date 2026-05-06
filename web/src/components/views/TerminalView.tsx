import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Plus, X } from "lucide-react";
import { useTerminalStore } from "../../stores/useTerminalStore";
import { useUIStore, getEffectiveTheme } from "../../stores/useUIStore";
import "@xterm/xterm/css/xterm.css";

/** Dark terminal theme (matches xterm default-ish) */
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

/** Light terminal theme (light paper background, dark text) */
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
  const sendInput = useTerminalStore((s) => s.sendInput);
  const theme = useUIStore((s) => s.theme);
  const isDark = getEffectiveTheme(theme) === "dark";

  // Create an initial terminal tab on first mount
  useEffect(() => {
    if (tabs.length === 0) {
      createTab();
    }
  }, []);

  const activeTab = tabs.find((t) => t.id === activeTabId);

  return (
    <div className={`flex h-full flex-col ${isDark ? "bg-black" : "bg-white"}`}>
      {/* Tab bar */}
      <div className={`flex items-center border-b ${isDark ? "border-neutral-800 bg-neutral-950" : "border-neutral-200 bg-neutral-100"}`}>
        <div className="flex flex-1 overflow-x-auto">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`flex items-center gap-1.5 px-3 py-1.5 text-xs transition-colors ${
                tab.id === activeTabId
                  ? isDark
                    ? "border-b-2 border-neutral-100 bg-neutral-900 text-neutral-100"
                    : "border-b-2 border-neutral-900 bg-white text-neutral-900"
                  : isDark
                    ? "text-neutral-500 hover:bg-neutral-900 hover:text-neutral-300"
                    : "text-neutral-500 hover:bg-neutral-200 hover:text-neutral-700"
              }`}
            >
              <span>{tab.label}</span>
              <span
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.id);
                }}
                className={`rounded p-0.5 ${isDark ? "hover:bg-neutral-700" : "hover:bg-neutral-300"}`}
              >
                <X className="h-3 w-3" />
              </span>
            </button>
          ))}
        </div>

        <button
          onClick={() => createTab()}
          className={`rounded p-1.5 ${isDark ? "text-neutral-500 hover:bg-neutral-800 hover:text-neutral-300" : "text-neutral-500 hover:bg-neutral-200 hover:text-neutral-700"}`}
          title="New terminal"
        >
          <Plus className="h-4 w-4" />
        </button>
      </div>

      {/* Terminal viewport */}
      <div className={`flex-1 overflow-hidden ${isDark ? "bg-black" : "bg-white"}`}>
        {activeTab ? (
          <TerminalViewport
            key={activeTab.id}
            tab={activeTab}
            isDark={isDark}
            onInput={(data) => sendInput(activeTab.id, data)}
          />
        ) : (
          <div className="flex h-full items-center justify-center text-neutral-500">
            <p className="text-sm">No terminal session</p>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Terminal viewport ──────────────────────────────────────────────────────

interface TerminalViewportProps {
  tab: {
    id: string;
    sessionId: string | null;
    lifecycle: string;
    buffer: string;
  };
  isDark: boolean;
  onInput: (data: string) => void;
}

function TerminalViewport({ tab, isDark, onInput }: TerminalViewportProps) {
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const writtenLenRef = useRef(0); // bytes already written to xterm

  // Initialize xterm.js and stream buffer to it in one unified effect
  useEffect(() => {
    if (!terminalRef.current) return;
    writtenLenRef.current = 0; // reset on (re)mount

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

    // Flush any buffer that arrived before xterm was ready
    if (tab.buffer.length > 0) {
      term.write(tab.buffer);
      writtenLenRef.current = tab.buffer.length;
    }

    // Handle user input
    term.onData((data: string) => {
      onInput(data);
    });

    // Fit terminal to container size
    const fitTerminal = () => {
      try {
        fitAddon.fit();
      } catch {
        // Ignore fit errors during resize
      }
    };

    fitTerminal();
    const resizeObserver = new ResizeObserver(() => fitTerminal());
    resizeObserver.observe(terminalRef.current);

    return () => {
      resizeObserver.disconnect();
      term.dispose();
      xtermRef.current = null;
      fitAddonRef.current = null;
    };
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

  // Disable input when session is exited
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
