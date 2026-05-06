import { create } from "zustand";
import type { MainTab } from "../lib/router";

export type { MainTab } from "../lib/router";
export type Theme = "light" | "dark" | "system";

export interface UIState {
  sidebarOpen: boolean;
  rightSidebarOpen: boolean;
  settingsOpen: boolean;
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
}

export interface UIActions {
  toggleSidebar: () => void;
  openSidebar: () => void;
  closeSidebar: () => void;
  toggleRightSidebar: () => void;
  openRightSidebar: () => void;
  closeRightSidebar: () => void;
  toggleSettings: () => void;
  closeSettings: () => void;
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
  navigateToChat: (sessionId?: string) => void;
  navigateToFiles: () => void;
  navigateToSettings: () => void;
}

const DEFAULT_LEFT_SIDEBAR_WIDTH = 256;
const DEFAULT_RIGHT_SIDEBAR_WIDTH = 280;
const MIN_SIDEBAR_WIDTH = 180;
const MAX_SIDEBAR_WIDTH = 500;

function loadLocalStorage() {
  const savedLeftWidth = parseInt(
    localStorage.getItem("leftSidebarWidth") || "",
    10,
  );
  const savedRightWidth = parseInt(
    localStorage.getItem("rightSidebarWidth") || "",
    10,
  );
  const savedTheme = localStorage.getItem("theme") as Theme | null;
  const savedRightSidebarOpen =
    localStorage.getItem("rightSidebarOpen") !== "false";
  const savedActiveTab = localStorage.getItem("activeTab") as MainTab | null;

  return {
    leftSidebarWidth: isNaN(savedLeftWidth)
      ? DEFAULT_LEFT_SIDEBAR_WIDTH
      : savedLeftWidth,
    rightSidebarWidth: isNaN(savedRightWidth)
      ? DEFAULT_RIGHT_SIDEBAR_WIDTH
      : savedRightWidth,
    theme: (savedTheme || "system") as Theme,
    rightSidebarOpen: savedRightSidebarOpen,
    activeTab: (savedActiveTab || "chat") as MainTab,
  };
}

const persisted = loadLocalStorage();

const initialState: UIState = {
  sidebarOpen: true,
  rightSidebarOpen: persisted.rightSidebarOpen,
  settingsOpen: false,
  mobileMenuOpen: false,
  mobileRightSidebarOpen: false,
  theme: persisted.theme,
  inputValue: "",
  isLoading: false,
  isStreaming: false,
  connectionStatus: "disconnected",
  leftSidebarWidth: persisted.leftSidebarWidth,
  rightSidebarWidth: persisted.rightSidebarWidth,
  activeTab: persisted.activeTab,
};

function clampSidebarWidth(width: number): number {
  return Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, width));
}

export const useUIStore = create<UIState & UIActions>((set) => ({
  ...initialState,

  toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen })),
  openSidebar: () => set({ sidebarOpen: true }),
  closeSidebar: () => set({ sidebarOpen: false }),

  toggleRightSidebar: () =>
    set((s) => {
      const newOpen = !s.rightSidebarOpen;
      localStorage.setItem("rightSidebarOpen", String(newOpen));
      return { rightSidebarOpen: newOpen };
    }),

  openRightSidebar: () => {
    localStorage.setItem("rightSidebarOpen", "true");
    set({ rightSidebarOpen: true });
  },

  closeRightSidebar: () => {
    localStorage.setItem("rightSidebarOpen", "false");
    set({ rightSidebarOpen: false });
  },

  toggleSettings: () => set((s) => ({ settingsOpen: !s.settingsOpen })),
  closeSettings: () => set({ settingsOpen: false }),

  toggleMobileMenu: () => set((s) => ({ mobileMenuOpen: !s.mobileMenuOpen })),
  closeMobileMenu: () => set({ mobileMenuOpen: false }),

  toggleMobileRightSidebar: () =>
    set((s) => ({ mobileRightSidebarOpen: !s.mobileRightSidebarOpen })),
  closeMobileRightSidebar: () => set({ mobileRightSidebarOpen: false }),

  setTheme: (theme) => {
    localStorage.setItem("theme", theme);
    set({ theme });
  },

  setLeftSidebarWidth: (width) => {
    const clamped = clampSidebarWidth(width);
    localStorage.setItem("leftSidebarWidth", String(clamped));
    set({ leftSidebarWidth: clamped });
  },

  setRightSidebarWidth: (width) => {
    const clamped = clampSidebarWidth(width);
    localStorage.setItem("rightSidebarWidth", String(clamped));
    set({ rightSidebarWidth: clamped });
  },

  setInputValue: (value) => set({ inputValue: value }),
  setLoading: (isLoading) => set({ isLoading }),
  setStreaming: (isStreaming) => set({ isStreaming }),
  setConnectionStatus: (status) => set({ connectionStatus: status }),

  setActiveTab: (tab) => {
    localStorage.setItem("activeTab", tab);
    set({ activeTab: tab });
  },

  navigateToChat: (sessionId) => {
    set({ activeTab: "chat" });
    localStorage.setItem("activeTab", "chat");
  },

  navigateToFiles: () => {
    set({ activeTab: "files" });
    localStorage.setItem("activeTab", "files");
  },

  navigateToSettings: () => {
    set({ activeTab: "settings" });
    localStorage.setItem("activeTab", "settings");
  },
}));

/**
 * Derive the effective theme (resolving 'system' to light/dark).
 */
export function getEffectiveTheme(theme: Theme): "light" | "dark" {
  if (theme === "system") {
    if (typeof window !== "undefined") {
      return window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light";
    }
    return "light";
  }
  return theme;
}
