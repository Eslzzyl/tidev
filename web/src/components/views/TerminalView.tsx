import { Edit3, Plus, X, XCircle, XSquare } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Restty, parseGhosttyTheme } from "restty";
import { useTerminalStore } from "../../stores/useTerminalStore";
import { getEffectiveTheme, useUIStore } from "../../stores/useUIStore";
import type { TerminalConnection } from "../../terminal/connection";
import { createTerminalFontInputs } from "../../terminal/fonts";
import { createResttyTransport } from "../../terminal/resttyTransport";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import type { ContextMenuItem } from "../ui/ContextMenu";
import { ContextMenu } from "../ui/ContextMenu";
import { RenameDialog } from "../ui/RenameDialog";
import { IconButton, Tabs } from "../ui";
import { useTranslation } from "react-i18next";

import { useRoute, useLocation } from "wouter";
import { routes } from "../../lib/routes";

const TERMINAL_FONT_SIZE = 17;

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

const ANSI_PALETTE_KEYS = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite",
] as const;

function createResttyTheme(theme: typeof DARK_THEME) {
  const lines = [
    `background = ${theme.background}`,
    `foreground = ${theme.foreground}`,
    `cursor-color = ${theme.cursor}`,
    `cursor-text = ${theme.background}`,
    `selection-background = ${theme.selectionBackground}`,
    ...ANSI_PALETTE_KEYS.map((key, index) => `palette = ${index}=${theme[key]}`),
  ];

  return parseGhosttyTheme(lines.join("\n"));
}

export function TerminalView() {
  const { t } = useTranslation();
  const [, params] = useRoute<{ tabId?: string }>("/terminal/:tabId?");
  const [, navigate] = useLocation();
  const routeTabId = params?.tabId ?? null;

  const tabs = useTerminalStore((s) => s.tabs);
  const activeTabId = useTerminalStore((s) => s.activeTabId);
  const createTab = useTerminalStore((s) => s.createTab);
  const closeTab = useTerminalStore((s) => s.closeTab);
  const closeTabs = useTerminalStore((s) => s.closeTabs);
  const renameTab = useTerminalStore((s) => s.renameTab);
  const setActiveTab = useTerminalStore((s) => s.setActiveTab);
  const startSession = useTerminalStore((s) => s.startSession);
  const restoreRunningSessions = useTerminalStore((s) => s.restoreRunningSessions);
  const theme = useUIStore((s) => s.theme);
  const terminalFontFamily = useUIStore((s) => s.settings.terminalFontFamily);
  const terminalFontRevision = useUIStore((s) => s.terminalFontRevision);
  const isDark = getEffectiveTheme(theme) === "dark";

  // Sync route tabId -> activeTabId
  useEffect(() => {
    if (routeTabId && tabs.some((t) => t.id === routeTabId)) {
      if (activeTabId !== routeTabId) {
        setActiveTab(routeTabId);
      }
    }
  }, [routeTabId, tabs, activeTabId, setActiveTab]);

  const handleSelectTab = useCallback(
    (tabId: string) => {
      setActiveTab(tabId);
      navigate(routes.terminal(tabId));
    },
    [navigate, setActiveTab],
  );

  const handleCreateTab = useCallback(() => {
    const newId = createTab();
    void startSession(newId, 80, 24);
    navigate(routes.terminal(newId));
  }, [createTab, navigate, startSession]);

  const handleCloseTab = useCallback(
    async (tabId: string) => {
      await closeTab(tabId);
      const nextActiveId = useTerminalStore.getState().activeTabId;
      navigate(routes.terminal(nextActiveId));
    },
    [closeTab, navigate],
  );

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
        onClick: () => void handleCloseTab(tabId),
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
        const id = createTab();
        void startSession(id, 80, 24);
        navigate(routes.terminal(id), { replace: true });
      }
    });
  }, [createTab, navigate, restoreRunningSessions, startSession, tabs.length]);

  return (
    <div className={`flex h-full flex-col ${isDark ? "bg-black" : "bg-white"}`}>
      {/* Tab bar */}
      <Tabs.Root
        value={activeTabId ?? undefined}
        onValueChange={handleSelectTab}
        className={isDark ? "terminal-tabs-root is-dark" : "terminal-tabs-root"}
      >
        <Tabs.List className="terminal-tabs-list">
          {tabs.map((tab) => (
            <Tabs.Item
              key={tab.id}
              className={tab.id === activeTabId ? "terminal-tab-item active" : "terminal-tab-item"}
            >
              <div className="terminal-tab-surface">
                <Tabs.Trigger
                  value={tab.id}
                  onContextMenu={(e) => handleContextMenu(e, tab.id)}
                  className="terminal-tab-trigger"
                >
                  <span>{tab.label}</span>
                </Tabs.Trigger>
                <IconButton
                  label={t("Close terminal")}
                  size="sm"
                  className="terminal-tab-close"
                  onClick={() => void handleCloseTab(tab.id)}
                >
                  <X size={12} />
                </IconButton>
              </div>
            </Tabs.Item>
          ))}
        </Tabs.List>
        <IconButton
          label={t("New terminal tab")}
          size="sm"
          onClick={handleCreateTab}
          className="terminal-new-tab-button"
          title={t("New terminal tab")}
        >
          <Plus size={14} />
        </IconButton>
      </Tabs.Root>

      {/* Terminal viewport */}
      <div className="flex-1 overflow-hidden">
        {tabs.map((tab) => (
          <div key={tab.id} className={tab.id === activeTabId ? "block h-full flex-1" : "hidden"}>
            <TerminalViewport
              tab={tab}
              isDark={isDark}
              isActive={tab.id === activeTabId}
              terminalFontFamily={terminalFontFamily}
              terminalFontRevision={terminalFontRevision}
            />
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
          title={t("Close All Terminals")}
          message={t("Are you sure you want to close all terminal tabs?")}
          confirmText={t("Close All")}
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
          title={t("Close Other Terminals")}
          message={t("Are you sure you want to close all other terminal tabs?")}
          confirmText={t("Close Others")}
          danger
          onConfirm={() => {
            const others = tabs.filter((t) => t.id !== confirmClose.excludeId).map((t) => t.id);
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
    lifecycle: "idle" | "connecting" | "running" | "exited";
  };
  isDark: boolean;
  isActive: boolean;
  terminalFontFamily: string;
  terminalFontRevision: number;
}

function TerminalViewport({
  tab,
  isDark,
  isActive,
  terminalFontFamily,
  terminalFontRevision,
}: TerminalViewportProps) {
  const terminalRef = useRef<HTMLDivElement>(null);
  const resttyRef = useRef<Restty | null>(null);
  const appliedTerminalFontRef = useRef<{
    restty: Restty;
    family: string;
    revision: number;
  } | null>(null);
  /** Guards against duplicate HTTP starts during StrictMode mount replay. */
  const httpStartedRef = useRef(false);

  // Keep the connection-mounting effect independent from the selected font.
  const terminalFontFamilyRef = useRef(terminalFontFamily);
  const terminalFontRevisionRef = useRef(terminalFontRevision);
  terminalFontFamilyRef.current = terminalFontFamily;
  terminalFontRevisionRef.current = terminalFontRevision;

  const startSession = useTerminalStore((s) => s.startSession);

  // Create the server-side PTY before mounting its restty transport.
  useEffect(() => {
    if (tab.lifecycle === "idle" && !tab.connection && !httpStartedRef.current) {
      httpStartedRef.current = true;
      startSession(tab.id, 80, 24);
    }
  }, [startSession, tab.connection, tab.id, tab.lifecycle]);

  // Mount one native restty surface for this tab and let it own sizing,
  // keyboard input, IME handling, selection, and terminal rendering.
  useEffect(() => {
    if (!terminalRef.current || !tab.connection || resttyRef.current) return;

    const theme = isDark ? DARK_THEME : LIGHT_THEME;
    const initialFontFamily = terminalFontFamilyRef.current;
    const initialFontRevision = terminalFontRevisionRef.current;
    const restty = new Restty({
      root: terminalRef.current,
      terminal: {
        renderer: "auto",
        fontSize: TERMINAL_FONT_SIZE,
        fontSizeMode: "em",
        fonts: createTerminalFontInputs(initialFontFamily),
        theme: createResttyTheme(theme),
        autoResize: true,
        showResizeOverlay: false,
        maxScrollbackBytes: 10 * 1024 * 1024,
      },
      surface: {
        paneStyles: {
          enabled: true,
          paneBackground: theme.background,
          splitBackground: theme.background,
          dividerColor: "transparent",
        },
        defaultContextMenu: false,
        shortcuts: false,
      },
      services: {
        ptyTransport: createResttyTransport(tab.connection),
      },
    });

    resttyRef.current = restty;
    appliedTerminalFontRef.current = {
      restty,
      family: initialFontFamily,
      revision: initialFontRevision,
    };
    restty.setPaused(!isActive);
    restty.connectPty();
    restty.setFontSize(TERMINAL_FONT_SIZE);
    restty.updateSize(true);

    return () => {
      restty.destroy();
      resttyRef.current = null;
      if (appliedTerminalFontRef.current?.restty === restty) {
        appliedTerminalFontRef.current = null;
      }
    };
  }, [tab.connection]);

  useEffect(() => {
    const restty = resttyRef.current;
    if (!restty) return;

    const applied = appliedTerminalFontRef.current;
    if (
      applied?.restty === restty &&
      applied.family === terminalFontFamily &&
      applied.revision === terminalFontRevision
    ) {
      return;
    }

    const next = { restty, family: terminalFontFamily, revision: terminalFontRevision };
    appliedTerminalFontRef.current = next;

    void restty.setFonts(createTerminalFontInputs(terminalFontFamily)).catch((error: unknown) => {
      if (appliedTerminalFontRef.current === next) {
        appliedTerminalFontRef.current = null;
      }
      console.warn("Failed to apply terminal font:", error);
    });
  }, [tab.connection, terminalFontFamily, terminalFontRevision]);

  useEffect(() => {
    const restty = resttyRef.current;
    if (!restty) return;

    restty.setPaused(!isActive);
    if (isActive) restty.updateSize(true);
  }, [isActive]);

  useEffect(() => {
    const restty = resttyRef.current;
    if (!restty) return;

    const theme = isDark ? DARK_THEME : LIGHT_THEME;
    const resttyTheme = createResttyTheme(theme);
    restty.forEachPane((pane) => pane.applyTheme(resttyTheme));
    restty.setPaneStyleOptions({
      paneBackground: theme.background,
      splitBackground: theme.background,
      dividerColor: "transparent",
    });
  }, [isDark]);

  return (
    <div
      ref={terminalRef}
      className="h-full w-full p-1"
      style={{ background: isDark ? "#000" : "#fefefe" }}
    />
  );
}
