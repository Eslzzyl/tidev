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
import { Button, IconButton } from "../ui";

type CategoryId =
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
  icon: React.ReactNode;
}

const categories: Category[] = [
  {
    id: "appearance",
    label: "Appearance",
    icon: <Palette className="h-4 w-4" />,
  },
  { id: "editor", label: "Editor", icon: <Type className="h-4 w-4" /> },
  {
    id: "interaction",
    label: "Interaction",
    icon: <Keyboard className="h-4 w-4" />,
  },
  {
    id: "terminal",
    label: "Terminal",
    icon: <TerminalIcon className="h-4 w-4" />,
  },
  { id: "security", label: "Security", icon: <Lock className="h-4 w-4" /> },
  { id: "providers", label: "Providers", icon: <Server className="h-4 w-4" /> },
  { id: "agents", label: "Agents", icon: <Bot className="h-4 w-4" /> },
  { id: "mcp", label: "MCP Servers", icon: <Boxes className="h-4 w-4" /> },
  { id: "skills", label: "Skills", icon: <Sparkles className="h-4 w-4" /> },
  { id: "about", label: "About", icon: <Info className="h-4 w-4" /> },
];

export function SettingsPanel() {
  const { t } = useTranslation();
  const [matchRoute, params] = useRoute<{ category?: string }>("/settings/:category?");
  const [, navigate] = useLocation();

  const settingsPanelOpen = useUIStore((s) => s.settingsPanelOpen);
  const settingsInitialCategory = useUIStore((s) => s.settingsInitialCategory);
  const closeSettingsStore = useUIStore((s) => s.closeSettingsPanel);
  const [activeCategory, setActiveCategory] = useState<CategoryId>("appearance");

  const isOpen = settingsPanelOpen || matchRoute;

  const closeSettings = useCallback(() => {
    closeSettingsStore();
    if (matchRoute) {
      navigate(routes.chat());
    }
  }, [closeSettingsStore, matchRoute, navigate]);

  useEffect(() => {
    if (matchRoute && params?.category && categories.some((c) => c.id === params.category)) {
      setActiveCategory(params.category as CategoryId);
    } else if (
      settingsInitialCategory &&
      categories.some((c) => c.id === settingsInitialCategory)
    ) {
      setActiveCategory(settingsInitialCategory as CategoryId);
    }
  }, [matchRoute, params?.category, settingsInitialCategory]);

  const handleSelectCategory = (catId: CategoryId) => {
    setActiveCategory(catId);
    if (matchRoute) {
      navigate(routes.settings(catId));
    }
  };

  const navRef = useRef<HTMLDivElement>(null);
  const [activeRect, setActiveRect] = useState<{
    top: number;
    height: number;
  } | null>(null);

  // Measure active button position for sliding highlight indicator
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
        closeSettings();
      }
    },
    [closeSettings],
  );

  useEffect(() => {
    if (isOpen) {
      document.addEventListener("keydown", handleKeyDown);
      // Prevent body scroll while panel is open
      document.body.style.overflow = "hidden";
    }
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      document.body.style.overflow = "";
    };
  }, [isOpen, handleKeyDown]);

  if (!isOpen) return null;

  return (
    <div
      className="fixed inset-0 z-60 flex items-center justify-center motion-safe:animate-fade-in bg-black/50 p-4"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) closeSettings();
      }}
    >
      <div className="motion-safe:animate-scale-fade flex h-[70vh] max-h-[720px] w-full max-w-4xl flex-col overflow-hidden rounded-xl bg-white shadow-2xl dark:bg-neutral-900">
        {/* Header */}
        <div className="flex shrink-0 items-center justify-between border-b border-neutral-200 px-5 py-3 dark:border-neutral-800">
          <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">
            {t("Settings")}
          </h2>
          <IconButton type="button" size="sm" label={t("Close settings")} onClick={closeSettings}>
            <X className="h-5 w-5" />
          </IconButton>
        </div>

        {/* Body: sidebar + content */}
        <div className="flex min-h-0 flex-1">
          {/* Sidebar */}
          <nav
            ref={navRef}
            className="relative w-36 shrink-0 overflow-y-auto border-r border-neutral-200 p-2 dark:border-neutral-800"
          >
            {/* Sliding highlight indicator */}
            {activeRect && (
              <div
                className="ui-settings-nav-indicator"
                style={{ top: activeRect.top, height: activeRect.height }}
              />
            )}
            {categories.map((cat) => (
              <Button
                type="button"
                key={cat.id}
                data-cat-id={cat.id}
                data-active={activeCategory === cat.id ? "true" : undefined}
                onClick={() => handleSelectCategory(cat.id)}
                className="ui-settings-nav-button"
                variant="ghost"
                size="sm"
                leadingIcon={cat.icon}
              >
                {t(cat.label)}
              </Button>
            ))}
          </nav>

          {/* Content */}
          <div
            key={activeCategory}
            className="flex-1 overflow-y-auto p-5 motion-safe:animate-fade-in"
          >
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
        </div>

        {/* Footer */}
        <div className="flex shrink-0 items-center justify-between border-t border-neutral-200 px-5 py-3 dark:border-neutral-800">
          <p className="text-xs text-neutral-500 dark:text-neutral-400">
            {t("Settings are saved automatically")}
          </p>
        </div>
      </div>
    </div>
  );
}
