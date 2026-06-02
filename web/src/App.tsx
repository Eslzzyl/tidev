import {
  useState,
  useEffect,
  useCallback,
  lazy,
  Suspense,
  useRef,
} from "react";
import { useSessionStore } from "./stores/useSessionStore";
import {
  useUIStore,
  getEffectiveTheme,
  type MainTab,
} from "./stores/useUIStore";
import { useAuthStore } from "./stores/useAuthStore";
import { api } from "./api/client";
import { AuthGate } from "./components/AuthGate";
import { SettingsPanel } from "./components/settings/SettingsPanel";
import { WelcomePage } from "./components/WelcomePage";
import { LeftSidebar } from "./components/layout/LeftSidebar";
import { RightSidebar } from "./components/layout/RightSidebar";
import { ResizeHandle } from "./components/layout/ResizeHandle";
import { Header } from "./components/layout/Header";
import { ChatPanel } from "./components/chat/ChatPanel";
import { ToastContainer } from "./components/ui/ToastContainer";
import { CloudOff, RefreshCw } from "lucide-react";
import { useSSE } from "./hooks/useSSE";

// Lazy-loaded views — each will be loaded on first render of that tab
const FilesView = lazy(() =>
  import("./components/views/FilesView").then((m) => ({
    default: m.FilesView,
  })),
);
const TerminalView = lazy(() =>
  import("./components/views/TerminalView").then((m) => ({
    default: m.TerminalView,
  })),
);
const GitView = lazy(() =>
  import("./components/views/GitView").then((m) => ({ default: m.GitView })),
);
const StatsView = lazy(() =>
  import("./components/views/StatsView").then((m) => ({
    default: m.StatsView,
  })),
);

function App() {
  const [isLoading, setIsLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Resizing state
  const [isResizingLeft, setIsResizingLeft] = useState(false);
  const [isResizingRight, setIsResizingRight] = useState(false);
  const [resizeStartX, setResizeStartX] = useState(0);
  const [resizeStartWidth, setResizeStartWidth] = useState(0);

  const setSessions = useSessionStore((s) => s.setSessions);
  const setCurrentSession = useSessionStore((s) => s.setCurrentSession);
  const setMessages = useSessionStore((s) => s.setMessages);
  const currentSessionId = useSessionStore((s) => s.currentSessionId);
  const isDraftSession = useSessionStore((s) => s.isDraftSession);
  // SSE connection must be mounted at App level so it's active even when
  // WelcomePage is showing (ChatPanel hasn't mounted yet). Otherwise events
  // published by the server during the first message are lost.
  useSSE(currentSessionId);
  const theme = useUIStore((s) => s.theme);
  const activeTab = useUIStore((s) => s.activeTab);
  const leftSidebarWidth = useUIStore((s) => s.leftSidebarWidth);
  const rightSidebarWidth = useUIStore((s) => s.rightSidebarWidth);
  const rightSidebarOpen = useUIStore((s) => s.rightSidebarOpen);
  const mobileMenuOpen = useUIStore((s) => s.mobileMenuOpen);
  const mobileRightSidebarOpen = useUIStore((s) => s.mobileRightSidebarOpen);
  const closeMobileMenu = useUIStore((s) => s.closeMobileMenu);
  const closeMobileRightSidebar = useUIStore((s) => s.closeMobileRightSidebar);
  const setLeftSidebarWidth = useUIStore((s) => s.setLeftSidebarWidth);
  const setRightSidebarWidth = useUIStore((s) => s.setRightSidebarWidth);

  // Auth state
  const authIsLoading = useAuthStore((s) => s.isLoading);
  const authIsRequired = useAuthStore((s) => s.isAuthRequired);
  const authIsAuthenticated = useAuthStore((s) => s.isAuthenticated);
  const checkAuthStatus = useAuthStore((s) => s.checkAuthStatus);

  // Check auth status on mount
  useEffect(() => {
    checkAuthStatus();
  }, [checkAuthStatus]);

  // Apply theme
  useEffect(() => {
    const effectiveTheme = getEffectiveTheme(theme);
    if (effectiveTheme === "dark") {
      document.documentElement.classList.add("dark");
    } else {
      document.documentElement.classList.remove("dark");
    }

    // Update body/html background for Safari 26+ browser chrome color
    const bgColor = effectiveTheme === "dark" ? "#0a0a0a" : "#ffffff";
    document.documentElement.style.backgroundColor = bgColor;
    document.body.style.backgroundColor = bgColor;

    // Update theme-color meta tag (fallback for Chrome Android, etc.)
    const themeColorMeta = document.querySelector('meta[name="theme-color"]');
    if (themeColorMeta) {
      themeColorMeta.setAttribute("content", bgColor);
    }
  }, [theme]);

  // Listen for system theme changes
  useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const handleChange = () => {
      const effectiveTheme = getEffectiveTheme(theme);
      const isDark = effectiveTheme === "dark";
      if (isDark) {
        document.documentElement.classList.add("dark");
      } else {
        document.documentElement.classList.remove("dark");
      }
      // Sync body/html background for Safari 26+ browser chrome
      const bgColor = isDark ? "#0a0a0a" : "#ffffff";
      document.documentElement.style.backgroundColor = bgColor;
      document.body.style.backgroundColor = bgColor;
      const meta = document.querySelector('meta[name="theme-color"]');
      if (meta) meta.setAttribute("content", bgColor);
    };
    mediaQuery.addEventListener("change", handleChange);
    return () => mediaQuery.removeEventListener("change", handleChange);
  }, [theme]);

  // Sync URL hash with active tab
  useEffect(() => {
    const handleHashChange = () => {
      const hash = window.location.hash.replace(/^#/, "");
      const parts = hash.split("/").filter(Boolean);
      const tab = parts[0] as MainTab | undefined;
      if (tab === "settings") {
        // Open settings panel, fall back to chat
        useUIStore.getState().openSettingsPanel();
        useUIStore.getState().setActiveTab("chat");
      } else if (tab && ["chat", "files"].includes(tab)) {
        useUIStore.getState().setActiveTab(tab);
      }
    };
    window.addEventListener("hashchange", handleHashChange);
    // Initial sync from URL
    handleHashChange();
    return () => window.removeEventListener("hashchange", handleHashChange);
  }, []);

  // Update URL hash when activeTab changes
  useEffect(() => {
    const hash = window.location.hash.replace(/^#/, "");
    const currentTab = hash.split("/")[0];
    if (currentTab !== activeTab) {
      window.location.hash = activeTab;
    }
  }, [activeTab]);

  // Load initial data
  useEffect(() => {
    // Wait for auth check to complete before deciding what to load
    if (authIsLoading) return;

    const loadData = async () => {
      // If auth is required but not yet authenticated, skip loading
      // to avoid a stale 401 error that would appear after login.
      if (authIsRequired && !authIsAuthenticated) {
        setIsLoading(false);
        return;
      }

      try {
        const { sessions } = await api.listSessions();
        setSessions(sessions);

        const params = new URLSearchParams(window.location.search);
        const sessionId = params.get("session");
        if (sessionId) {
          const [session, { messages, todos }] = await Promise.all([
            api.getSession(sessionId),
            api.listMessages(sessionId),
          ]);
          setCurrentSession(session);
          setMessages(messages);
          useSessionStore.getState().setTodos(todos ?? []);
        }
      } catch (err) {
        setLoadError(
          err instanceof Error ? err.message : "Failed to load sessions",
        );
      } finally {
        setIsLoading(false);
      }
    };

    loadData();
  }, [setSessions, setCurrentSession, setMessages, authIsLoading, authIsRequired, authIsAuthenticated]);

  // Resize RAF ref — throttles state updates to once per frame
  const resizeRafRef = useRef<number | null>(null);

  // Global resize handlers (throttled via requestAnimationFrame)
  useEffect(() => {
    if (!isResizingLeft && !isResizingRight) return;

    const handleResizeMove = (e: MouseEvent) => {
      // Throttle: ignore events if a frame is already queued
      if (resizeRafRef.current !== null) return;

      const clientX = e.clientX; // capture immediately (avoid stale event)

      resizeRafRef.current = requestAnimationFrame(() => {
        resizeRafRef.current = null;

        if (isResizingLeft) {
          const diff = clientX - resizeStartX;
          const newWidth = Math.min(
            500,
            Math.max(180, resizeStartWidth + diff),
          );
          setLeftSidebarWidth(newWidth);
        } else if (isResizingRight) {
          const diff = resizeStartX - clientX;
          const newWidth = Math.min(
            500,
            Math.max(180, resizeStartWidth + diff),
          );
          setRightSidebarWidth(newWidth);
        }
      });
    };

    const handleResizeEnd = () => {
      if (resizeRafRef.current !== null) {
        cancelAnimationFrame(resizeRafRef.current);
        resizeRafRef.current = null;
      }
      setIsResizingLeft(false);
      setIsResizingRight(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    document.addEventListener("mousemove", handleResizeMove);
    document.addEventListener("mouseup", handleResizeEnd);
    return () => {
      document.removeEventListener("mousemove", handleResizeMove);
      document.removeEventListener("mouseup", handleResizeEnd);
      if (resizeRafRef.current !== null) {
        cancelAnimationFrame(resizeRafRef.current);
      }
    };
  }, [
    isResizingLeft,
    isResizingRight,
    resizeStartX,
    resizeStartWidth,
    setLeftSidebarWidth,
    setRightSidebarWidth,
  ]);

  const handleLeftResizeStart = useCallback(
    (e: React.MouseEvent) => {
      setIsResizingLeft(true);
      setResizeStartX(e.clientX);
      setResizeStartWidth(leftSidebarWidth);
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    },
    [leftSidebarWidth],
  );

  const handleRightResizeStart = useCallback(
    (e: React.MouseEvent) => {
      setIsResizingRight(true);
      setResizeStartX(e.clientX);
      setResizeStartWidth(rightSidebarWidth);
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    },
    [rightSidebarWidth],
  );

  // Show welcome page when no session is selected (only in chat tab)
  const showWelcomePage =
    activeTab === "chat" && !currentSessionId && !isDraftSession;
  // Show left sidebar only in chat tab when there's a session
  const showSidebars = activeTab === "chat" && !showWelcomePage;
  // Right sidebar visibility: only in chat and only when explicitly opened
  const showRightSidebar = showSidebars && rightSidebarOpen;

  return (
    <>
      {/* AuthGate overlay — shows full-screen when auth is required but not authenticated */}
      <AuthGate />

      {/* Don't render app content behind the AuthGate overlay — children
          like FilesView would otherwise mount and fire spurious API calls. */}
      {authIsRequired && !authIsAuthenticated ? null : isLoading ? (
        <div className="flex h-[100dvh] items-center justify-center bg-white dark:bg-neutral-950">
          <div className="text-center">
            <div className="mb-4 inline-block h-8 w-8 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-900 dark:border-neutral-700 dark:border-t-neutral-100" />
            <p className="text-sm text-neutral-600 dark:text-neutral-400">
              Loading...
            </p>
          </div>
        </div>
      ) : loadError ? (
        <div className="flex h-[100dvh] items-center justify-center bg-white dark:bg-neutral-950">
          <div className="mx-auto max-w-sm px-6 text-center">
            <div className="mb-6 flex justify-center">
              <div className="flex h-16 w-16 items-center justify-center rounded-2xl bg-red-50 dark:bg-red-900/20">
                <CloudOff className="h-8 w-8 text-red-500" />
              </div>
            </div>
            <h2 className="mb-2 text-lg font-semibold text-neutral-900 dark:text-neutral-100">
              Unable to Connect
            </h2>
            <p className="mb-2 text-sm leading-relaxed text-neutral-500 dark:text-neutral-400">
              {loadError === "Unknown error" ||
              loadError === "Failed to load sessions"
                ? "The server is not responding. Please ensure the backend is running and retry."
                : loadError}
            </p>
            <button
              onClick={() => window.location.reload()}
              className="inline-flex items-center gap-2 rounded-lg bg-neutral-900 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:bg-neutral-800 active:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200 dark:active:bg-neutral-300"
            >
              <RefreshCw className="h-4 w-4" />
              Retry Connection
            </button>
          </div>
        </div>
      ) : (<>
        <SettingsPanel />

      <div className="flex h-[100dvh] flex-col bg-[radial-gradient(ellipse_at_top,_var(--tw-gradient-stops))] from-neutral-100 via-white to-white dark:from-neutral-900 dark:via-neutral-950 dark:to-neutral-950">
        {/* ── Floating Header Card ── */}
        {/* Desktop: mx-3 mt-3 rounded-xl border shadow. Mobile: full-bleed. */}
        <div className="mx-3 mt-3 max-md:mx-0 max-md:mt-0">
          <Header className="md:rounded-xl md:border md:border-neutral-200/60 md:shadow-sm dark:md:border-neutral-800/60 dark:md:shadow-black/20 md:bg-white/95 md:dark:bg-neutral-900/95 max-md:border-b" />
        </div>

        {/* ── Desktop: floating cards layout ── */}
        {/* Cards always same width as header. Sidebars only show in chat tab. */}
        <div className="flex flex-1 px-3 pb-3 pt-2 min-h-0 max-md:p-0 max-md:pt-0">
          {/* ── Left Sidebar (Desktop floating card) ── */}
          {showSidebars && (
            <aside
              className="flex h-full min-h-0 flex-shrink-0 flex-col overflow-hidden rounded-xl border border-neutral-200/60 bg-white shadow-sm max-md:hidden dark:border-neutral-800/60 dark:bg-neutral-900 dark:shadow-black/20"
              style={{ width: leftSidebarWidth, willChange: "width" }}
            >
              <LeftSidebar />
            </aside>
          )}

          {/* Left Resize Handle */}
          {showSidebars && (
            <ResizeHandle
              onResizeStart={handleLeftResizeStart}
              isResizing={isResizingLeft}
            />
          )}

          {/* ── Main Content (floating card) ── */}
          {/* All views stay mounted, hidden via display:none to preserve state */}
          <main className="relative flex flex-1 flex-col min-h-0 overflow-hidden rounded-xl border border-neutral-200/60 bg-white shadow-sm dark:border-neutral-800/60 dark:bg-neutral-950 dark:shadow-black/10 max-md:rounded-none max-md:border-0 max-md:shadow-none">
            <Suspense
              fallback={
                <div className="flex h-full items-center justify-center text-sm text-neutral-400">
                  Loading…
                </div>
              }
            >
              <div
                className="h-full"
                style={{ display: activeTab === "chat" ? "" : "none" }}
              >
                {showWelcomePage ? <WelcomePage /> : <ChatPanel />}
              </div>
              <div
                className="h-full"
                style={{ display: activeTab === "files" ? "" : "none" }}
              >
                <FilesView />
              </div>
              <div
                className="flex h-full flex-col overflow-hidden"
                style={{ display: activeTab === "terminal" ? "" : "none" }}
              >
                <TerminalView />
              </div>
              <div
                className="h-full"
                style={{ display: activeTab === "git" ? "" : "none" }}
              >
                <GitView />
              </div>
              <div
                className="h-full"
                style={{ display: activeTab === "stats" ? "" : "none" }}
              >
                <StatsView />
              </div>
            </Suspense>
          </main>

          {/* Right Resize Handle */}
          {showRightSidebar && (
            <ResizeHandle
              onResizeStart={handleRightResizeStart}
              isResizing={isResizingRight}
            />
          )}

          {/* ── Right Sidebar (Desktop floating card) ── */}
          {showRightSidebar && (
            <aside
              className="flex h-full min-h-0 flex-shrink-0 flex-col overflow-hidden rounded-xl border border-neutral-200/60 bg-white shadow-sm motion-safe:animate-fade-in max-md:hidden dark:border-neutral-800/60 dark:bg-neutral-900 dark:shadow-black/20"
              style={{ width: rightSidebarWidth, willChange: "width" }}
            >
              <RightSidebar />
            </aside>
          )}
        </div>

        {/* ── Mobile Overlays (fixed, outside flex flow) ── */}
        {/* Mobile Left Sidebar */}
        {showSidebars && (
          <>
            <aside
              className={`fixed inset-y-0 left-0 z-50 flex w-[85vw] max-w-[320px] flex-col border-r border-neutral-200/80 bg-white transition-transform duration-200 ease-in-out md:hidden dark:border-neutral-800/60 dark:bg-neutral-950 ${
                mobileMenuOpen ? "translate-x-0" : "-translate-x-full"
              }`}
            >
              <LeftSidebar />
            </aside>
            {mobileMenuOpen && (
              <button
                onClick={closeMobileMenu}
                className="fixed inset-0 z-40 bg-black/20 backdrop-blur-sm md:hidden"
                aria-label="Close menu"
              />
            )}
          </>
        )}

        {/* Mobile Right Sidebar */}
        {showSidebars && (
          <>
            <aside
              className={`fixed inset-y-0 right-0 z-50 flex w-[85vw] max-w-[320px] flex-col border-l border-neutral-200/80 bg-white transition-transform duration-200 ease-in-out md:hidden dark:border-neutral-800/60 dark:bg-neutral-950 ${
                mobileRightSidebarOpen ? "translate-x-0" : "translate-x-full"
              }`}
            >
              <RightSidebar />
            </aside>
            {mobileRightSidebarOpen && (
              <button
                onClick={closeMobileRightSidebar}
                className="fixed inset-0 z-40 bg-black/20 backdrop-blur-sm md:hidden"
                aria-label="Close info panel"
              />
            )}
          </>
        )}
      </div>
      <ToastContainer />
      </>)}
    </>);
}

export default App;
