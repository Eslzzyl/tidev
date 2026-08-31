import { useState, useEffect, useCallback, useRef } from "react";
import {
  X,
  Palette,
  Type,
  Keyboard,
  Terminal as TerminalIcon,
  Lock,
  Boxes,
  Sparkles,
  Bot,
  Info,
  Server,
  ChevronRight,
  ChevronLeft,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useRoute, useLocation } from "wouter";
import { routes } from "../../lib/routes";
import { useUIStore } from "../../stores/useUIStore";
import { AppearanceSection } from "./AppearanceSection";
import { EditorSection } from "./EditorSection";
import { InteractionSection } from "./InteractionSection";
import { TerminalSection } from "./TerminalSection";
import { SecuritySection } from "./SecuritySection";
import { McpSection } from "./McpSection";
import { SkillsSection } from "./SkillsSection";
import { AgentsSection } from "./AgentsSection";
import { AboutSection } from "./AboutSection";
import { ProvidersSection } from "./ProvidersSection";
import { IconButton } from "../ui";

export type CategoryId =
  | "appearance"
  | "editor"
  | "interaction"
  | "terminal"
  | "security"
  | "providers"
  | "agents"
  | "mcp"
  | "skills"
  | "about";

interface Category {
  id: CategoryId;
  label: string;
  description: string;
  icon: React.ReactNode;
}

interface CategoryGroup {
  id: string;
  label: string;
  categories: Category[];
}

const categoryGroups: CategoryGroup[] = [
  {
    id: "preferences",
    label: "Preferences",
    categories: [
      {
        id: "appearance",
        label: "Appearance",
        description: "Choose your preferred color theme",
        icon: <Palette className="h-4 w-4" />,
      },
      {
        id: "editor",
        label: "Editor",
        description: "Customize fonts and diff layout",
        icon: <Type className="h-4 w-4" />,
      },
      {
        id: "interaction",
        label: "Interaction",
        description: "Customize how the chat input behaves",
        icon: <Keyboard className="h-4 w-4" />,
      },
      {
        id: "terminal",
        label: "Terminal",
        description: "Choose which shell to use in the terminal",
        icon: <TerminalIcon className="h-4 w-4" />,
      },
    ],
  },
  {
    id: "ai",
    label: "Agent & AI",
    categories: [
      {
        id: "providers",
        label: "Providers",
        description: "Manage provider API keys and custom model endpoints",
        icon: <Server className="h-4 w-4" />,
      },
      {
        id: "agents",
        label: "Agents",
        description: "Configure the subagents available to the task tool",
        icon: <Bot className="h-4 w-4" />,
      },
      {
        id: "mcp",
        label: "MCP Servers",
        description: "Manage Model Context Protocol connections",
        icon: <Boxes className="h-4 w-4" />,
      },
      {
        id: "skills",
        label: "Skills",
        description: "Browse, preview, and load agent skills",
        icon: <Sparkles className="h-4 w-4" />,
      },
    ],
  },
  {
    id: "system",
    label: "System",
    categories: [
      {
        id: "security",
        label: "Security",
        description: "Set a password to protect the web interface",
        icon: <Lock className="h-4 w-4" />,
      },
      {
        id: "about",
        label: "About",
        description: "Version, runtime status, and server maintenance",
        icon: <Info className="h-4 w-4" />,
      },
    ],
  },
];

const allCategories = categoryGroups.flatMap((g) => g.categories);

export function SettingsPanel() {
  const { t } = useTranslation();
  const [matchRoute, params] = useRoute<{ category?: string }>("/settings/:category?");
  const [, navigate] = useLocation();

  const [isMobile, setIsMobile] = useState(
    () => typeof window !== "undefined" && window.innerWidth < 768,
  );

  useEffect(() => {
    const handleResize = () => setIsMobile(window.innerWidth < 768);
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  const settingsPanelOpen = useUIStore((s) => s.settingsPanelOpen);
  const settingsInitialCategory = useUIStore((s) => s.settingsInitialCategory);
  const closeSettingsStore = useUIStore((s) => s.closeSettingsPanel);
  const [activeCategory, setActiveCategory] = useState<CategoryId>("appearance");
  const [mobileCategoryView, setMobileCategoryView] = useState<CategoryId | null>(null);

  const isOpen = settingsPanelOpen || matchRoute;

  const closeSettings = useCallback(() => {
    closeSettingsStore();
    setMobileCategoryView(null);
    if (matchRoute) {
      navigate(routes.chat());
    }
  }, [closeSettingsStore, matchRoute, navigate]);

  useEffect(() => {
    if (matchRoute && params?.category && allCategories.some((c) => c.id === params.category)) {
      const cat = params.category as CategoryId;
      setActiveCategory(cat);
      if (typeof window !== "undefined" && window.innerWidth < 768) {
        setMobileCategoryView(cat);
      }
    } else if (
      settingsInitialCategory &&
      allCategories.some((c) => c.id === settingsInitialCategory)
    ) {
      const cat = settingsInitialCategory as CategoryId;
      setActiveCategory(cat);
      if (typeof window !== "undefined" && window.innerWidth < 768) {
        setMobileCategoryView(cat);
      }
    } else {
      // Default to the first category of the first group ("appearance")
      setActiveCategory("appearance");
      setMobileCategoryView(null);
    }
  }, [matchRoute, params?.category, settingsInitialCategory]);

  const handleSelectCategory = (catId: CategoryId) => {
    setActiveCategory(catId);
    if (isMobile) {
      setMobileCategoryView(catId);
    }
    if (matchRoute) {
      navigate(routes.settings(catId));
    }
  };

  const handleBackToMenu = () => {
    setMobileCategoryView(null);
    if (matchRoute) {
      navigate("/settings");
    }
  };

  const navRef = useRef<HTMLDivElement>(null);
  const [activeRect, setActiveRect] = useState<{
    top: number;
    height: number;
  } | null>(null);

  // Measure active button position for sliding highlight indicator on desktop
  useEffect(() => {
    if (navRef.current) {
      const activeEl = navRef.current.querySelector<HTMLElement>(
        `[data-cat-id="${activeCategory}"]`,
      );
      if (activeEl) {
        const navRect = navRef.current.getBoundingClientRect();
        const btnRect = activeEl.getBoundingClientRect();
        setActiveRect({
          top: btnRect.top - navRect.top,
          height: btnRect.height,
        });
      }
    }
  }, [activeCategory]);

  // Close on Escape
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (isMobile && mobileCategoryView) {
          handleBackToMenu();
        } else {
          closeSettings();
        }
      }
    },
    [closeSettings, isMobile, mobileCategoryView],
  );

  useEffect(() => {
    if (isOpen) {
      document.addEventListener("keydown", handleKeyDown);
      document.body.style.overflow = "hidden";
    }
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      document.body.style.overflow = "";
    };
  }, [isOpen, handleKeyDown]);

  if (!isOpen) return null;

  const currentCategoryMeta = allCategories.find(
    (c) => c.id === (isMobile ? mobileCategoryView || activeCategory : activeCategory),
  );

  return (
    <div
      className="fixed inset-0 z-60 flex items-center justify-center bg-black/40 backdrop-blur-xs p-0 md:p-6 motion-safe:animate-fade-in"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) closeSettings();
      }}
    >
      <div className="flex h-full w-full flex-col overflow-hidden bg-white shadow-2xl dark:bg-neutral-900 md:h-[64vh] md:max-h-[530px] md:max-w-[860px] md:rounded-2xl md:border md:border-neutral-200/80 md:dark:border-neutral-800/80">
        {/* Header */}
        <div className="flex shrink-0 items-center justify-between border-b border-neutral-200/80 px-4 py-2.5 dark:border-neutral-800/80 sm:px-6">
          <div className="flex items-center gap-2 min-w-0">
            {/* Mobile Back Button (ONLY visible when viewing category detail on mobile) */}
            {isMobile && mobileCategoryView ? (
              <IconButton
                type="button"
                size="sm"
                variant="ghost"
                label={t("Back to settings")}
                onClick={handleBackToMenu}
                className="-ml-1.5 shrink-0"
              >
                <ChevronLeft className="h-5 w-5" />
              </IconButton>
            ) : null}
            <h2 className="truncate text-base font-semibold text-neutral-900 dark:text-neutral-100">
              {isMobile && mobileCategoryView
                ? t(currentCategoryMeta?.label || "Settings")
                : t("Settings")}
            </h2>
          </div>
          <IconButton type="button" size="sm" label={t("Close settings")} onClick={closeSettings}>
            <X className="h-5 w-5" />
          </IconButton>
        </div>

        {/* Body: Responsive Master-Detail */}
        <div className="flex min-h-0 flex-1 overflow-hidden">
          {/* Desktop Sidebar (visible on md+) */}
          <nav
            ref={navRef}
            className="relative hidden w-52 shrink-0 flex-col overflow-y-auto border-r border-neutral-200/80 p-3 dark:border-neutral-800/80 md:flex"
          >
            {/* Sliding highlight indicator */}
            {activeRect && (
              <div
                className="ui-settings-nav-indicator"
                style={{ top: activeRect.top, height: activeRect.height }}
              />
            )}
            <div className="space-y-3.5">
              {categoryGroups.map((group) => (
                <div key={group.id} className="space-y-1">
                  <div className="px-2.5 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
                    {t(group.label)}
                  </div>
                  {group.categories.map((cat) => {
                    const isActive = activeCategory === cat.id;
                    return (
                      <button
                        key={cat.id}
                        type="button"
                        data-cat-id={cat.id}
                        data-active={isActive ? "true" : undefined}
                        onClick={() => handleSelectCategory(cat.id)}
                        className={`relative z-1 flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-xs font-medium transition-colors ${
                          isActive
                            ? "text-neutral-900 font-semibold dark:text-neutral-100"
                            : "text-neutral-600 hover:bg-neutral-100/60 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800/60 dark:hover:text-neutral-200"
                        }`}
                      >
                        <span
                          className={`shrink-0 ${
                            isActive
                              ? "text-neutral-900 dark:text-neutral-100"
                              : "text-neutral-400 dark:text-neutral-500"
                          }`}
                        >
                          {cat.icon}
                        </span>
                        <span className="truncate">{t(cat.label)}</span>
                      </button>
                    );
                  })}
                </div>
              ))}
            </div>
          </nav>

          {/* Mobile Categories Menu View (when mobileCategoryView is null on small screens) */}
          {isMobile ? (
            mobileCategoryView ? (
              /* Mobile Category Detail View */
              <div className="flex-1 overflow-y-auto p-4">
                {mobileCategoryView === "appearance" && <AppearanceSection />}
                {mobileCategoryView === "editor" && <EditorSection />}
                {mobileCategoryView === "interaction" && <InteractionSection />}
                {mobileCategoryView === "terminal" && <TerminalSection />}
                {mobileCategoryView === "security" && <SecuritySection />}
                {mobileCategoryView === "providers" && <ProvidersSection />}
                {mobileCategoryView === "agents" && <AgentsSection />}
                {mobileCategoryView === "mcp" && <McpSection />}
                {mobileCategoryView === "skills" && <SkillsSection />}
                {mobileCategoryView === "about" && <AboutSection />}
              </div>
            ) : (
              /* Mobile Categories Group List */
              <div className="flex-1 overflow-y-auto p-4 space-y-5">
                {categoryGroups.map((group) => (
                  <div key={group.id} className="space-y-2">
                    <h3 className="px-1 text-xs font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
                      {t(group.label)}
                    </h3>
                    <div className="overflow-hidden rounded-xl border border-neutral-200/80 bg-neutral-50/50 divide-y divide-neutral-200/60 dark:border-neutral-800/80 dark:bg-neutral-800/30 dark:divide-neutral-800/60">
                      {group.categories.map((cat) => (
                        <button
                          key={cat.id}
                          type="button"
                          onClick={() => handleSelectCategory(cat.id)}
                          className="flex w-full items-center justify-between gap-3 p-3 text-left transition-colors active:bg-neutral-100 dark:active:bg-neutral-800"
                        >
                          <div className="flex items-center gap-3 min-w-0">
                            <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-white shadow-xs text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300">
                              {cat.icon}
                            </span>
                            <div className="min-w-0">
                              <span className="block truncate text-sm font-semibold text-neutral-900 dark:text-neutral-100">
                                {t(cat.label)}
                              </span>
                              <span className="block truncate text-xs text-neutral-500 dark:text-neutral-400">
                                {t(cat.description)}
                              </span>
                            </div>
                          </div>
                          <ChevronRight className="h-4 w-4 shrink-0 text-neutral-400" />
                        </button>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            )
          ) : (
            /* Desktop Content View */
            <div className="flex-1 overflow-y-auto p-5 md:p-6">
              {activeCategory === "appearance" && <AppearanceSection />}
              {activeCategory === "editor" && <EditorSection />}
              {activeCategory === "interaction" && <InteractionSection />}
              {activeCategory === "terminal" && <TerminalSection />}
              {activeCategory === "security" && <SecuritySection />}
              {activeCategory === "providers" && <ProvidersSection />}
              {activeCategory === "agents" && <AgentsSection />}
              {activeCategory === "mcp" && <McpSection />}
              {activeCategory === "skills" && <SkillsSection />}
              {activeCategory === "about" && <AboutSection />}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
