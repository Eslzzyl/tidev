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
  Search,
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
  const [location, navigate] = useLocation();

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
  const [searchQuery, setSearchQuery] = useState("");
  const returnPathRef = useRef("/chat");

  const isOpen = settingsPanelOpen || matchRoute;

  // Turn programmatic settings opens into route navigation while preserving the current page.
  // The route remains the source of truth after navigation, which also keeps browser back working.
  useEffect(() => {
    if (settingsPanelOpen && !matchRoute) {
      returnPathRef.current = location.startsWith("/settings") ? "/chat" : location;
      const category = allCategories.some((item) => item.id === settingsInitialCategory)
        ? settingsInitialCategory
        : null;
      navigate(routes.settings(category));
    } else if (settingsPanelOpen && matchRoute) {
      closeSettingsStore();
    }
  }, [
    closeSettingsStore,
    location,
    matchRoute,
    navigate,
    settingsInitialCategory,
    settingsPanelOpen,
  ]);

  const closeSettings = useCallback(() => {
    closeSettingsStore();
    setMobileCategoryView(null);
    setSearchQuery("");
    if (matchRoute || settingsPanelOpen) {
      navigate(returnPathRef.current);
    }
  }, [closeSettingsStore, matchRoute, navigate, settingsPanelOpen]);

  useEffect(() => {
    if (matchRoute && params?.category && allCategories.some((c) => c.id === params.category)) {
      const cat = params.category as CategoryId;
      setActiveCategory(cat);
      setMobileCategoryView(isMobile ? cat : null);
    } else if (
      settingsInitialCategory &&
      allCategories.some((c) => c.id === settingsInitialCategory)
    ) {
      const cat = settingsInitialCategory as CategoryId;
      setActiveCategory(cat);
      setMobileCategoryView(isMobile ? cat : null);
    } else {
      // Default to the first category of the first group ("appearance")
      setActiveCategory("appearance");
      setMobileCategoryView(null);
    }
  }, [isMobile, matchRoute, params?.category, settingsInitialCategory]);

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
      if (!activeEl) {
        setActiveRect(null);
        return;
      }

      const navRect = navRef.current.getBoundingClientRect();
      const btnRect = activeEl.getBoundingClientRect();
      setActiveRect({
        top: btnRect.top - navRect.top,
        height: btnRect.height,
      });
    }
  }, [activeCategory, searchQuery]);

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

  const normalizedSearchQuery = searchQuery.trim().toLocaleLowerCase();
  const visibleCategoryGroups = categoryGroups
    .map((group) => ({
      ...group,
      categories: group.categories.filter((category) => {
        if (!normalizedSearchQuery) return true;
        return [category.label, category.description]
          .map((value) => t(value).toLocaleLowerCase())
          .some((value) => value.includes(normalizedSearchQuery));
      }),
    }))
    .filter((group) => group.categories.length > 0);

  const currentCategoryMeta = allCategories.find(
    (c) => c.id === (isMobile ? mobileCategoryView || activeCategory : activeCategory),
  );

  const renderCategory = (category: CategoryId) => {
    if (category === "appearance") return <AppearanceSection />;
    if (category === "editor") return <EditorSection />;
    if (category === "interaction") return <InteractionSection />;
    if (category === "terminal") return <TerminalSection />;
    if (category === "security") return <SecuritySection />;
    if (category === "providers") return <ProvidersSection />;
    if (category === "agents") return <AgentsSection />;
    if (category === "mcp") return <McpSection />;
    if (category === "skills") return <SkillsSection />;
    return <AboutSection />;
  };

  const renderCategoryList = (mobile = false) => (
    <>
      {visibleCategoryGroups.map((group) => (
        <div key={group.id} className="settings-page-nav-group">
          <div className="settings-page-nav-group-title">{t(group.label)}</div>
          {group.categories.map((cat) => {
            const isActive = activeCategory === cat.id;
            return (
              <button
                key={cat.id}
                type="button"
                data-cat-id={cat.id}
                data-active={isActive ? "true" : undefined}
                onClick={() => handleSelectCategory(cat.id)}
                className={`settings-page-nav-item${isActive ? " active" : ""}`}
              >
                <span className="settings-page-nav-icon">{cat.icon}</span>
                <span className="settings-page-nav-copy">
                  <span>{t(cat.label)}</span>
                  {mobile ? <small>{t(cat.description)}</small> : null}
                </span>
                {mobile ? <ChevronRight className="settings-page-nav-chevron" /> : null}
              </button>
            );
          })}
        </div>
      ))}
      {visibleCategoryGroups.length === 0 ? (
        <p className="settings-page-empty">{t("No matching settings")}</p>
      ) : null}
    </>
  );

  return (
    <div className="settings-page motion-safe:animate-fade-in">
      <div className="settings-page-layout">
        <aside className="settings-page-sidebar">
          <button type="button" className="settings-page-back" onClick={closeSettings}>
            <ChevronLeft className="h-4 w-4" />
            <span>{t("Back to application")}</span>
          </button>
          <label className="settings-page-search">
            <Search className="h-4 w-4" aria-hidden="true" />
            <span className="sr-only">{t("Search settings...")}</span>
            <input
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
              placeholder={t("Search settings...")}
              type="search"
            />
          </label>
          <nav ref={navRef} className="settings-page-nav" aria-label={t("Settings")}>
            {activeRect ? (
              <div
                className="ui-settings-nav-indicator"
                style={{ top: activeRect.top, height: activeRect.height }}
              />
            ) : null}
            {renderCategoryList()}
          </nav>
        </aside>

        <main className="settings-page-main">
          <header className="settings-page-header">
            <div className="settings-page-heading">
              {isMobile && mobileCategoryView ? (
                <IconButton
                  type="button"
                  size="sm"
                  variant="ghost"
                  label={t("Back to settings")}
                  onClick={handleBackToMenu}
                >
                  <ChevronLeft className="h-5 w-5" />
                </IconButton>
              ) : null}
              <div className="min-w-0">
                <p className="settings-page-eyebrow">{t("Settings")}</p>
                <h1>
                  {isMobile && mobileCategoryView
                    ? t(currentCategoryMeta?.label || "Settings")
                    : t("Settings")}
                </h1>
              </div>
            </div>
            <IconButton type="button" size="sm" label={t("Close settings")} onClick={closeSettings}>
              <X className="h-5 w-5" />
            </IconButton>
          </header>

          {isMobile && !mobileCategoryView ? (
            <div className="settings-page-mobile-list">
              <label className="settings-page-search settings-page-mobile-search">
                <Search className="h-4 w-4" aria-hidden="true" />
                <span className="sr-only">{t("Search settings...")}</span>
                <input
                  value={searchQuery}
                  onChange={(event) => setSearchQuery(event.target.value)}
                  placeholder={t("Search settings...")}
                  type="search"
                />
              </label>
              {renderCategoryList(true)}
            </div>
          ) : (
            <div className="settings-page-content">
              <div className="settings-page-content-inner">
                {renderCategory(mobileCategoryView || activeCategory)}
              </div>
            </div>
          )}
        </main>
      </div>
    </div>
  );
}
