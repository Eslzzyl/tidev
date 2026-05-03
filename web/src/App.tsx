import { useState, useEffect, useCallback } from 'react';
import { useSessionStore } from './stores/useSessionStore';
import { useUIStore, getEffectiveTheme } from './stores/useUIStore';
import { api } from './api/client';
import { Settings } from './components/Settings';
import { WelcomePage } from './components/WelcomePage';
import { LeftSidebar } from './components/layout/LeftSidebar';
import { RightSidebar } from './components/layout/RightSidebar';
import { ResizeHandle } from './components/layout/ResizeHandle';
import { ChatPanel } from './components/chat/ChatPanel';

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
  const theme = useUIStore((s) => s.theme);
  const leftSidebarWidth = useUIStore((s) => s.leftSidebarWidth);
  const rightSidebarWidth = useUIStore((s) => s.rightSidebarWidth);
  const rightSidebarOpen = useUIStore((s) => s.rightSidebarOpen);
  const mobileMenuOpen = useUIStore((s) => s.mobileMenuOpen);
  const mobileRightSidebarOpen = useUIStore((s) => s.mobileRightSidebarOpen);
  const closeMobileMenu = useUIStore((s) => s.closeMobileMenu);
  const closeMobileRightSidebar = useUIStore((s) => s.closeMobileRightSidebar);
  const setLeftSidebarWidth = useUIStore((s) => s.setLeftSidebarWidth);
  const setRightSidebarWidth = useUIStore((s) => s.setRightSidebarWidth);

  // Apply theme
  useEffect(() => {
    const effectiveTheme = getEffectiveTheme(theme);
    if (effectiveTheme === 'dark') {
      document.documentElement.classList.add('dark');
    } else {
      document.documentElement.classList.remove('dark');
    }
  }, [theme]);

  // Listen for system theme changes
  useEffect(() => {
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const handleChange = () => {
      const effectiveTheme = getEffectiveTheme(theme);
      if (effectiveTheme === 'dark') {
        document.documentElement.classList.add('dark');
      } else {
        document.documentElement.classList.remove('dark');
      }
    };
    mediaQuery.addEventListener('change', handleChange);
    return () => mediaQuery.removeEventListener('change', handleChange);
  }, [theme]);

  // Load initial data
  useEffect(() => {
    const loadData = async () => {
      try {
        const { sessions } = await api.listSessions();
        setSessions(sessions);

        const params = new URLSearchParams(window.location.search);
        const sessionId = params.get('session');
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
        setLoadError(err instanceof Error ? err.message : 'Failed to load sessions');
      } finally {
        setIsLoading(false);
      }
    };

    loadData();
  }, [setSessions, setCurrentSession, setMessages]);

  // Global resize handlers
  useEffect(() => {
    if (!isResizingLeft && !isResizingRight) return;

    const handleResizeMove = (e: MouseEvent) => {
      if (isResizingLeft) {
        const diff = e.clientX - resizeStartX;
        setLeftSidebarWidth(resizeStartWidth + diff);
      } else if (isResizingRight) {
        const diff = resizeStartX - e.clientX;
        setRightSidebarWidth(resizeStartWidth + diff);
      }
    };

    const handleResizeEnd = () => {
      setIsResizingLeft(false);
      setIsResizingRight(false);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };

    document.addEventListener('mousemove', handleResizeMove);
    document.addEventListener('mouseup', handleResizeEnd);
    return () => {
      document.removeEventListener('mousemove', handleResizeMove);
      document.removeEventListener('mouseup', handleResizeEnd);
    };
  }, [isResizingLeft, isResizingRight, resizeStartX, resizeStartWidth, setLeftSidebarWidth, setRightSidebarWidth]);

  const handleLeftResizeStart = useCallback((e: React.MouseEvent) => {
    setIsResizingLeft(true);
    setResizeStartX(e.clientX);
    setResizeStartWidth(leftSidebarWidth);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  }, [leftSidebarWidth]);

  const handleRightResizeStart = useCallback((e: React.MouseEvent) => {
    setIsResizingRight(true);
    setResizeStartX(e.clientX);
    setResizeStartWidth(rightSidebarWidth);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  }, [rightSidebarWidth]);

  if (isLoading) {
    return (
      <div className="flex h-screen items-center justify-center bg-white dark:bg-neutral-950">
        <div className="text-center">
          <div className="mb-4 inline-block h-8 w-8 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-900 dark:border-neutral-700 dark:border-t-neutral-100" />
          <p className="text-sm text-neutral-600 dark:text-neutral-400">Loading...</p>
        </div>
      </div>
    );
  }

  if (loadError) {
    return (
      <div className="flex h-screen items-center justify-center bg-white dark:bg-neutral-950">
        <div className="text-center">
          <p className="mb-2 text-red-600 dark:text-red-400">{loadError}</p>
          <button
            onClick={() => window.location.reload()}
            className="rounded bg-neutral-900 px-4 py-2 text-sm font-medium text-white hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  // Show welcome page when no session is selected
  const showWelcomePage = !currentSessionId && !isDraftSession;

  return (
    <>
      <Settings />

      <div className="h-screen w-full bg-white dark:bg-neutral-950">
        <div className="flex h-full">
          {/* Left Sidebar - hidden on welcome page */}
          {!showWelcomePage && (
            <>
              <aside
                className={`fixed inset-y-0 left-0 z-50 transform border-r border-neutral-200 bg-white transition-transform duration-200 ease-in-out md:relative md:translate-x-0 dark:border-neutral-800 dark:bg-neutral-950 ${
                  mobileMenuOpen ? 'translate-x-0' : '-translate-x-full'
                }`}
                style={{ width: leftSidebarWidth }}
              >
                <LeftSidebar />
              </aside>

              {/* Left Resize Handle */}
              <ResizeHandle onResizeStart={handleLeftResizeStart} isResizing={isResizingLeft} />

              {/* Mobile overlay */}
              {mobileMenuOpen && (
                <button
                  onClick={closeMobileMenu}
                  className="fixed inset-0 z-40 bg-black/50 md:hidden"
                  aria-label="Close menu"
                />
              )}
            </>
          )}

          {/* Main content */}
          <main className="relative flex-1 min-w-0">
            {showWelcomePage ? <WelcomePage /> : <ChatPanel />}
          </main>

          {/* Right Sidebar - hidden on welcome page */}
          {!showWelcomePage && rightSidebarOpen && (
            <>
              <ResizeHandle onResizeStart={handleRightResizeStart} isResizing={isResizingRight} />

              <aside
                className="hidden border-l border-neutral-200 bg-white md:block dark:border-neutral-800 dark:bg-neutral-950"
                style={{ width: rightSidebarWidth }}
              >
                <RightSidebar />
              </aside>
            </>
          )}

          {/* Mobile Right Sidebar - hidden on welcome page */}
          {!showWelcomePage && (
            <aside
              className={`fixed inset-y-0 right-0 z-50 transform border-l border-neutral-200 bg-white transition-transform duration-200 ease-in-out md:hidden dark:border-neutral-800 dark:bg-neutral-950 ${
                mobileRightSidebarOpen ? 'translate-x-0' : 'translate-x-full'
              }`}
              style={{ width: 280 }}
            >
              <RightSidebar />
            </aside>
          )}

          {/* Mobile overlay for right sidebar - hidden on welcome page */}
          {!showWelcomePage && mobileRightSidebarOpen && (
            <button
              onClick={closeMobileRightSidebar}
              className="fixed inset-0 z-40 bg-black/50 md:hidden"
              aria-label="Close info panel"
            />
          )}
        </div>
      </div>
    </>
  );
}

export default App;
