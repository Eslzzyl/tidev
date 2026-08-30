import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";
import { i18n, resolveLocale, type LocalePreference } from "../i18n";
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
  settingsInitialCategory: string | null;
  mobileMenuOpen: boolean;
  mobileRightSidebarOpen: boolean;
  theme: Theme;
  inputValue: string;
  isLoading: boolean;
  isStreaming: boolean;
  connectionStatus: "connected" | "disconnected" | "connecting";
  locale: LocalePreference;
  leftSidebarWidth: number;
  rightSidebarWidth: number;
  settings: SettingsState;
}

export interface UIActions {
  toggleSidebar: () => void;
  openSidebar: () => void;
  closeSidebar: () => void;
  toggleRightSidebar: () => void;
  openRightSidebar: () => void;
  closeRightSidebar: () => void;
  openSettingsPanel: (category?: unknown) => void;
  closeSettingsPanel: () => void;
  toggleMobileMenu: () => void;
  closeMobileMenu: () => void;
  toggleMobileRightSidebar: () => void;
  closeMobileRightSidebar: () => void;
  setTheme: (theme: Theme) => void;
  setLocale: (locale: LocalePreference) => void;
  setLeftSidebarWidth: (width: number) => void;
  setRightSidebarWidth: (width: number) => void;
  setInputValue: (value: string) => void;
  setLoading: (isLoading: boolean) => void;
  setStreaming: (isStreaming: boolean) => void;
  setConnectionStatus: (status: UIState["connectionStatus"]) => void;
  updateSettings: (partial: Partial<SettingsState>) => void;
}

const DEFAULT_LEFT_SIDEBAR_WIDTH = 256;
const DEFAULT_RIGHT_SIDEBAR_WIDTH = 280;
const MIN_SIDEBAR_WIDTH = 180;
const MAX_SIDEBAR_WIDTH = 500;

const defaultSettings: SettingsState = {
  fontFamily: "Inter, system-ui, sans-serif",
  monoFontFamily: "JetBrains Mono, Fira Code, monospace",
  fontSize: 14,
  diffLayout: "side-by-side",
  enterToSend: true,
  terminalShell: "",
};

function loadLegacyPreferences(): { theme?: Theme; settings: Partial<SettingsState> } {
  if (typeof localStorage === "undefined") return { settings: {} };

  try {
    const raw = JSON.parse(localStorage.getItem("tidev-ui-settings") ?? "null") as
      | (Partial<SettingsState> & { theme?: Theme })
      | null;
    if (!raw) return { settings: {} };

    const { theme, ...settings } = raw;
    return { theme, settings };
  } catch {
    return { settings: {} };
  }
}

const legacyPreferences = loadLegacyPreferences();

const initialState: UIState = {
  sidebarOpen: true,
  rightSidebarOpen: true,
  settingsPanelOpen: false,
  settingsInitialCategory: null,
  mobileMenuOpen: false,
  mobileRightSidebarOpen: false,
  theme: legacyPreferences.theme ?? "system",
  locale: "system",
  inputValue: "",
  isLoading: false,
  isStreaming: false,
  connectionStatus: "disconnected",
  leftSidebarWidth: DEFAULT_LEFT_SIDEBAR_WIDTH,
  rightSidebarWidth: DEFAULT_RIGHT_SIDEBAR_WIDTH,
  settings: { ...defaultSettings, ...legacyPreferences.settings },
};

function applyVisualSettings(theme: Theme, settings: SettingsState): void {
  if (typeof document === "undefined") return;

  const root = document.documentElement;
  const effectiveTheme = getEffectiveTheme(theme);
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
  root.classList.toggle("dark", effectiveTheme === "dark");
  root.style.setProperty("--ui-font-family", settings.fontFamily);
  root.style.setProperty("--ui-mono-font", settings.monoFontFamily);
  root.style.setProperty("--ui-font-size", `${settings.fontSize}px`);
}

applyVisualSettings(initialState.theme, initialState.settings);

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

      openSettingsPanel: (category?: unknown) =>
        set({
          settingsPanelOpen: true,
          settingsInitialCategory: typeof category === "string" ? category : null,
        }),
      closeSettingsPanel: () => set({ settingsPanelOpen: false, settingsInitialCategory: null }),

      toggleMobileMenu: () => set((s) => ({ mobileMenuOpen: !s.mobileMenuOpen })),
      closeMobileMenu: () => set({ mobileMenuOpen: false }),

      toggleMobileRightSidebar: () =>
        set((s) => ({ mobileRightSidebarOpen: !s.mobileRightSidebarOpen })),
      closeMobileRightSidebar: () => set({ mobileRightSidebarOpen: false }),

      setTheme: (theme) =>
        set((state) => {
          applyVisualSettings(theme, state.settings);
          return { theme };
        }),

      setLocale: (locale) => {
        void i18n.changeLanguage(resolveLocale(locale));
        set({ locale });
      },

      setLeftSidebarWidth: (width) => set({ leftSidebarWidth: clampSidebarWidth(width) }),

      setRightSidebarWidth: (width) => set({ rightSidebarWidth: clampSidebarWidth(width) }),

      setInputValue: (value) => set({ inputValue: value }),
      setLoading: (isLoading) => set({ isLoading }),
      setStreaming: (isStreaming) => set({ isStreaming }),
      setConnectionStatus: (status) => set({ connectionStatus: status }),

      updateSettings: (partial) =>
        set((state) => {
          const settings = { ...state.settings, ...partial };
          applyVisualSettings(state.theme, settings);
          return { settings };
        }),
    }),
    {
      name: "tidev-ui",
      storage: createJSONStorage(() => localStorage),
      // Only persist user preferences — not transient UI state like loading/streaming
      partialize: (state) => ({
        theme: state.theme,
        locale: state.locale,
        leftSidebarWidth: state.leftSidebarWidth,
        rightSidebarWidth: state.rightSidebarWidth,
        rightSidebarOpen: state.rightSidebarOpen,
        settings: state.settings,
      }),
      onRehydrateStorage: () => (state) => {
        if (state) applyVisualSettings(state.theme, state.settings);
      },
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
