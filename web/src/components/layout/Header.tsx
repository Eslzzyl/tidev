import { useRef, useState, useEffect } from "react";
import {
  MessageSquare,
  FolderTree,
  Settings,
  Terminal,
  GitBranch,
  BarChart3,
  Menu,
  PanelRightClose,
  Info,
} from "lucide-react";
import { useUIStore, type MainTab } from "../../stores/useUIStore";
import { useSessionStore } from "../../stores/useSessionStore";

const tabs: { id: MainTab; label: string; icon: React.ReactNode }[] = [
  { id: "chat", label: "Chat", icon: <MessageSquare className="h-4 w-4" /> },
  { id: "files", label: "Files", icon: <FolderTree className="h-4 w-4" /> },
  { id: "terminal", label: "Terminal", icon: <Terminal className="h-4 w-4" /> },
  { id: "git", label: "Git", icon: <GitBranch className="h-4 w-4" /> },
  { id: "stats", label: "Stats", icon: <BarChart3 className="h-4 w-4" /> },
];

const pageLabels: Record<MainTab, string> = {
  chat: "Chat",
  files: "Files",
  terminal: "Terminal",
  git: "Git",
  settings: "Settings",
  stats: "Statistics",
};

interface HeaderProps {
  className?: string;
}

export function Header({ className }: HeaderProps) {
  const activeTab = useUIStore((s) => s.activeTab);
  const setActiveTab = useUIStore((s) => s.setActiveTab);
  const toggleMobileMenu = useUIStore((s) => s.toggleMobileMenu);
  const toggleRightSidebar = useUIStore((s) => s.toggleRightSidebar);
  const openSettingsPanel = useUIStore((s) => s.openSettingsPanel);
  const toggleMobileRightSidebar = useUIStore(
    (s) => s.toggleMobileRightSidebar,
  );
  const currentSession = useSessionStore((s) => s.currentSession);
  const isDraftSession = useSessionStore((s) => s.isDraftSession);
  const draftTitle = useSessionStore((s) => s.draftTitle);

  // ── Tab sliding indicator ──
  const navRef = useRef<HTMLDivElement>(null);
  const [indicator, setIndicator] = useState({ left: 0, width: 0 });

  // Measure active tab position whenever it changes.
  // Uses data attributes instead of callback refs to avoid infinite render loops.
  useEffect(() => {
    const nav = navRef.current;
    if (!nav) return;
    const activeEl = nav.querySelector<HTMLButtonElement>(
      `[data-tab-id="${activeTab}"]`,
    );
    if (!activeEl) return;
    const navRect = nav.getBoundingClientRect();
    const tabRect = activeEl.getBoundingClientRect();
    setIndicator({
      left: tabRect.left - navRect.left,
      width: tabRect.width,
    });
  }, [activeTab]);

  return (
    <header
      className={`relative z-10 flex h-12 items-center justify-between border-b border-neutral-100/80 bg-white/95 px-3 shadow-[0_1px_2px_-1px_rgba(0,0,0,0.05)] backdrop-blur-sm dark:border-neutral-800/60 dark:bg-neutral-950/95 dark:shadow-[0_1px_2px_-1px_rgba(0,0,0,0.3)] ${className ?? ""}`}
    >
      {/* Left: mobile menu + nav tabs */}
      <div className="flex items-center gap-1">
        {/* Mobile menu toggle */}
        <button
          onClick={toggleMobileMenu}
          className="mr-1 rounded p-1.5 text-neutral-500 hover:bg-neutral-100 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
          aria-label="Toggle menu"
        >
          <Menu className="h-4 w-4" />
        </button>

        {/* Nav tabs with animated sliding indicator */}
        <nav
          ref={navRef}
          className="relative flex items-center gap-0.5"
          role="tablist"
        >
          {tabs.map((tab) => (
            <button
              key={tab.id}
              data-tab-id={tab.id}
              onClick={() => setActiveTab(tab.id)}
              role="tab"
              aria-selected={activeTab === tab.id}
              className={`relative z-10 flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors ${
                activeTab === tab.id
                  ? "text-neutral-900 dark:text-neutral-100"
                  : "text-neutral-500 hover:bg-neutral-100/40 hover:text-neutral-700 dark:text-neutral-400 dark:hover:bg-neutral-800/40 dark:hover:text-neutral-300"
              }`}
            >
              {tab.icon}
              <span className="hidden sm:inline">{tab.label}</span>
            </button>
          ))}
          {/* Animated background pill — slides to the active tab */}
          <div
            className="absolute inset-y-0.5 rounded-md bg-neutral-100/80 shadow-[0_1px_2px_rgba(0,0,0,0.04)] ring-1 ring-neutral-200/50 motion-safe:transition-all motion-safe:duration-200 motion-safe:ease-smooth dark:bg-neutral-800/80 dark:shadow-black/20 dark:ring-neutral-700/50"
            style={{
              left: `${indicator.left}px`,
              width: `${indicator.width}px`,
            }}
          />
        </nav>
      </div>

      {/* Center: page title / session info */}
      <div className="hidden truncate text-center sm:block">
        {activeTab === "chat" && currentSession ? (
          <div className="flex items-center gap-2">
            {isDraftSession && (
              <span className="text-xs font-medium text-blue-500 dark:text-blue-400">
                Draft
              </span>
            )}
            <span className="truncate text-sm font-medium text-neutral-700 dark:text-neutral-300">
              {isDraftSession ? draftTitle : currentSession.title}
            </span>
            {currentSession?.model_display_name && !isDraftSession && (
              <span className="hidden text-xs text-neutral-400 md:inline">
                · {currentSession.model_display_name}
              </span>
            )}
          </div>
        ) : activeTab === "chat" && (isDraftSession || !currentSession) ? (
          <span className="text-sm text-neutral-500 dark:text-neutral-400">
            {isDraftSession ? draftTitle : "Chat"}
          </span>
        ) : (
          <span className="text-sm text-neutral-500 dark:text-neutral-400">
            {pageLabels[activeTab]}
          </span>
        )}
      </div>

      {/* Right: action buttons */}
      <div className="flex items-center gap-1">
        {/* Settings gear — always visible */}
        <button
          onClick={openSettingsPanel}
          className="rounded p-1.5 text-neutral-500 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
          aria-label="Settings"
        >
          <Settings className="h-4 w-4" />
        </button>

        {activeTab === "chat" && (
          <>
            <button
              onClick={toggleRightSidebar}
              className="hidden rounded p-1.5 text-neutral-500 hover:bg-neutral-100 md:block dark:text-neutral-400 dark:hover:bg-neutral-800"
              aria-label="Toggle info panel"
            >
              <PanelRightClose className="h-4 w-4" />
            </button>
            <button
              onClick={toggleMobileRightSidebar}
              className="rounded p-1.5 text-neutral-500 hover:bg-neutral-100 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
              aria-label="Open info panel"
            >
              <Info className="h-4 w-4" />
            </button>
          </>
        )}
      </div>
    </header>
  );
}
