import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";
import type { MainTab } from "../lib/router";

export type { MainTab } from "../lib/router";
export type Theme = "light" | "dark" | "system";

export interface SettingsState {
  fontFamily: string;
  monoFontFamily: string;
  fontSize: number;
  diffLayout: "inline" | "side-by-side";
  enterToSend: boolean;
  terminalShell: string;
}

export interface UIState {
  sidebarOpen: boolean;
  rightSidebarOpen: boolean;
  settingsPanelOpen: boolean;
  mobileMenuOpen: boolean;
  mobileRightSidebarOpen: boolean;
  theme: Theme;
  inputValue: string;
  isLoading: boolean;
  isStreaming: boolean;
  connectionStatus: "connected" | "disconnected" | "connecting";
  leftSidebarWidth: number;
  rightSidebarWidth: number;
  activeTab: MainTab;
  settings: SettingsState;
}

export interface UIActions {
  toggleSidebar: () => void;
  openSidebar: () => void;
  closeSidebar: () => void;
  toggleRightSidebar: () => void;
  openRightSidebar: () => void;
  closeRightSidebar: () => void;
  openSettingsPanel: () => void;
  closeSettingsPanel: () => void;
  toggleMobileMenu: () => void;
  closeMobileMenu: () => void;
  toggleMobileRightSidebar: () => void;
  closeMobileRightSidebar: () => void;
  setTheme: (theme: Theme) => void;
  setLeftSidebarWidth: (width: number) => void;
  setRightSidebarWidth: (width: number) => void;
  setInputValue: (value: string) => void;
  setLoading: (isLoading: boolean) => void;
  setStreaming: (isStreaming: boolean) => void;
  setConnectionStatus: (status: UIState["connectionStatus"]) => void;
  setActiveTab: (tab: MainTab) => void;
  navigateToChat: () => void;
  navigateToFiles: () => void;
  navigateToTerminal: () => void;
  navigateToGit: () => void;
  navigateToStats: () => void;
  updateSettings: (partial: Partial<SettingsState>) => void;
}

const DEFAULT_LEFT_SIDEBAR_WIDTH = 256;
const DEFAULT_RIGHT_SIDEBAR_WIDTH = 280;
const MIN_SIDEBAR_WIDTH = 180;
const MAX_SIDEBAR_WIDTH = 500;

const initialState: UIState = {
  sidebarOpen: true,
  rightSidebarOpen: true,
  settingsPanelOpen: false,
  mobileMenuOpen: false,
  mobileRightSidebarOpen: false,
  theme: "system",
  inputValue: "",
  isLoading: false,
  isStreaming: false,
  connectionStatus: "disconnected",
  leftSidebarWidth: DEFAULT_LEFT_SIDEBAR_WIDTH,
  rightSidebarWidth: DEFAULT_RIGHT_SIDEBAR_WIDTH,
  activeTab: "chat",
  settings: {
    fontFamily: "Inter, system-ui, sans-serif",
    monoFontFamily: "JetBrains Mono, Fira Code, monospace",
    fontSize: 14,
    diffLayout: "side-by-side",
    enterToSend: true,
    terminalShell: "",
  },
};

function clampSidebarWidth(width: number): number {
  return Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, width));
}

export const useUIStore = create<UIState & UIActions>()(
  persist(
    (set) => ({
      ...initialState,

      toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen })),
      openSidebar: () => set({ sidebarOpen: true }),
      closeSidebar: () => set({ sidebarOpen: false }),

      toggleRightSidebar: () => set((s) => ({ rightSidebarOpen: !s.rightSidebarOpen })),
      openRightSidebar: () => set({ rightSidebarOpen: true }),
      closeRightSidebar: () => set({ rightSidebarOpen: false }),

      openSettingsPanel: () => set({ settingsPanelOpen: true }),
      closeSettingsPanel: () => set({ settingsPanelOpen: false }),

      toggleMobileMenu: () => set((s) => ({ mobileMenuOpen: !s.mobileMenuOpen })),
      closeMobileMenu: () => set({ mobileMenuOpen: false }),

      toggleMobileRightSidebar: () =>
        set((s) => ({ mobileRightSidebarOpen: !s.mobileRightSidebarOpen })),
      closeMobileRightSidebar: () => set({ mobileRightSidebarOpen: false }),

      setTheme: (theme) => set({ theme }),

      setLeftSidebarWidth: (width) => set({ leftSidebarWidth: clampSidebarWidth(width) }),

      setRightSidebarWidth: (width) => set({ rightSidebarWidth: clampSidebarWidth(width) }),

      setInputValue: (value) => set({ inputValue: value }),
      setLoading: (isLoading) => set({ isLoading }),
      setStreaming: (isStreaming) => set({ isStreaming }),
      setConnectionStatus: (status) => set({ connectionStatus: status }),

      setActiveTab: (tab) => set({ activeTab: tab }),

      navigateToChat: () => set({ activeTab: "chat" }),
      navigateToFiles: () => set({ activeTab: "files" }),
      navigateToTerminal: () => set({ activeTab: "terminal" }),
      navigateToGit: () => set({ activeTab: "git" }),
      navigateToStats: () => set({ activeTab: "stats" }),

      updateSettings: (partial) =>
        set((s) => ({ settings: { ...s.settings, ...partial } })),
    }),
    {
      name: "tidev-ui",
      storage: createJSONStorage(() => localStorage),
      // Only persist user preferences — not transient UI state like loading/streaming
      partialize: (state) => ({
        theme: state.theme,
        leftSidebarWidth: state.leftSidebarWidth,
        rightSidebarWidth: state.rightSidebarWidth,
        rightSidebarOpen: state.rightSidebarOpen,
        activeTab: state.activeTab,
        settings: state.settings,
      }),
    },
  ),
);

/**
 * Derive the effective theme (resolving 'system' to light/dark).
 */
export function getEffectiveTheme(theme: Theme): "light" | "dark" {
  if (theme === "system") {
    if (typeof window !== "undefined") {
      return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
    }
    return "light";
  }
  return theme;
}
