import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Edit3, Plus, X, XCircle, XSquare } from "lucide-react";
import { useTerminalStore } from "../../stores/useTerminalStore";
import { useUIStore, getEffectiveTheme } from "../../stores/useUIStore";
import { ContextMenu } from "../ui/ContextMenu";
import type { ContextMenuItem } from "../ui/ContextMenu";
import { RenameDialog } from "../ui/RenameDialog";
import { ConfirmDialog } from "../ui/ConfirmDialog";
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
  const closeTabs = useTerminalStore((s) => s.closeTabs);
  const renameTab = useTerminalStore((s) => s.renameTab);
  const setActiveTab = useTerminalStore((s) => s.setActiveTab);
  const restoreRunningSessions = useTerminalStore((s) => s.restoreRunningSessions);
  const theme = useUIStore((s) => s.theme);
  const isDark = getEffectiveTheme(theme) === "dark";

  // On mount: restore running sessions from server, or create a new tab
  const restoreAttempted = useRef(false);

  // Context menu state
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    tabId: string;
  } | null>(null);
  const [renameTarget, setRenameTarget] = useState<string | null>(null);
  const [confirmClose, setConfirmClose] = useState<{
    type: "all" | "others";
    excludeId?: string;
  } | null>(null);

  const handleContextMenu = useCallback((e: React.MouseEvent, tabId: string) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY, tabId });
  }, []);

  const buildContextMenuItems = (tabId: string): ContextMenuItem[] => {
    const items: ContextMenuItem[] = [
      {
        label: "Rename",
        icon: <Edit3 size={14} />,
        onClick: () => setRenameTarget(tabId),
      },
      { type: "separator" },
      {
        label: "Close",
        icon: <X size={14} />,
        onClick: () => closeTab(tabId),
      },
    ];

    if (tabs.length > 1) {
      items.push({
        label: "Close Others",
        icon: <XCircle size={14} />,
        onClick: () => setConfirmClose({ type: "others", excludeId: tabId }),
      });
    }

    items.push({
      label: "Close All",
      icon: <XSquare size={14} />,
      danger: true,
      onClick: () => setConfirmClose({ type: "all" }),
    });

    return items;
  };
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
              onContextMenu={(e) => handleContextMenu(e, tab.id)}
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

      {/* Context menu */}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          items={buildContextMenuItems(contextMenu.tabId)}
          onClose={() => setContextMenu(null)}
        />
      )}

      {/* Rename dialog */}
      {renameTarget !== null && (
        <RenameDialog
          currentName={tabs.find((t) => t.id === renameTarget)?.label ?? ""}
          onSubmit={(newName) => {
            renameTab(renameTarget, newName);
            setRenameTarget(null);
          }}
          onClose={() => setRenameTarget(null)}
        />
      )}

      {/* Confirm close all */}
      {confirmClose?.type === "all" && (
        <ConfirmDialog
          title="Close All Terminals"
          message="Are you sure you want to close all terminal tabs?"
          confirmText="Close All"
          danger
          onConfirm={() => {
            closeTabs(tabs.map((t) => t.id));
            setConfirmClose(null);
          }}
          onCancel={() => setConfirmClose(null)}
        />
      )}

      {/* Confirm close others */}
      {confirmClose?.type === "others" && confirmClose.excludeId && (
        <ConfirmDialog
          title="Close Other Terminals"
          message="Are you sure you want to close all other terminal tabs?"
          confirmText="Close Others"
          danger
          onConfirm={() => {
            const others = tabs
              .filter((t) => t.id !== confirmClose.excludeId)
              .map((t) => t.id);
            if (others.length > 0) closeTabs(others);
            setConfirmClose(null);
          }}
          onCancel={() => setConfirmClose(null)}
        />
      )}
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
  const connRef = useRef<TerminalConnection | null>(null);
  /** Guards against duplicate HTTP start in StrictMode double-mount. */
  const httpStartedRef = useRef(false);

  const startSession = useTerminalStore((s) => s.startSession);

  // Initialize xterm.js and start the terminal session
  useEffect(() => {
    if (!terminalRef.current) return;
    if (xtermRef.current) return;

    // ★ Optimization: kick off HTTP POST immediately with default 80×24,
    // so PTY creation runs in parallel with xterm initialization.
    // The actual size will be sent via WebSocket resize once xterm is ready.
    if (!tab.connection && !httpStartedRef.current) {
      httpStartedRef.current = true;
      startSession(tab.id, 80, 24);
    }

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

    // Fit terminal and send resize if connection is ready.
    // This is called on first paint (rAF) and on every resize.
    const fitAndInit = () => {
      try {
        fitAddon.fit();
        const cols = term.cols;
        const rows = term.rows;
        if (cols <= 0 || rows <= 0) return;

        // Send current actual size to the PTY
        const conn = connRef.current;
        if (conn) {
          conn.sendMessage("resize", rows, cols);
        }
        // If connection not ready yet, resize will be sent by
        // the connection-watcher effect when it arrives.
      } catch {
        // Fit errors are non-fatal
      }
    };

    // ★ Optimization: use rAF (~16ms) instead of setTimeout(50ms)
    const rafId = requestAnimationFrame(() => {
      fitAndInit();
    });

    // Observe container for future resize
    const resizeObserver = new ResizeObserver(() => {
      fitAndInit();
    });
    resizeObserver.observe(terminalRef.current);

    return () => {
      cancelAnimationFrame(rafId);
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

    // If xterm is already ready, register live handler and send
    // the actual terminal size (may correct default 80×24 used in
    // the parallel HTTP start).
    if (xtermRef.current) {
      const term = xtermRef.current;
      const cols = term.cols;
      const rows = term.rows;
      if (cols > 0 && rows > 0) {
        conn.sendMessage("resize", rows, cols);
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
