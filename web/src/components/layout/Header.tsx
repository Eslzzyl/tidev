import {
  MessageSquare,
  FolderTree,
  Settings,
  Terminal,
  GitBranch,
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
];

const pageLabels: Record<MainTab, string> = {
  chat: "Chat",
  files: "Files",
  terminal: "Terminal",
  git: "Git",
  settings: "Settings",
};

export function Header() {
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

  return (
    <header className="flex h-11 items-center justify-between border-b border-neutral-200 bg-white px-3 dark:border-neutral-800 dark:bg-neutral-950">
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

        {/* Nav tabs */}
        <nav className="flex items-center gap-0.5" role="tablist">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              role="tab"
              aria-selected={activeTab === tab.id}
              className={`flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors ${
                activeTab === tab.id
                  ? "bg-neutral-100 text-neutral-900 dark:bg-neutral-800 dark:text-neutral-100"
                  : "text-neutral-500 hover:text-neutral-700 dark:text-neutral-400 dark:hover:text-neutral-300"
              }`}
            >
              {tab.icon}
              <span className="hidden sm:inline">{tab.label}</span>
            </button>
          ))}
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
