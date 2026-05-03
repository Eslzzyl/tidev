import { useState, useCallback, useEffect, useRef } from "react";
import {
  ChevronDown,
  MessageSquare,
  Clock,
  MoreHorizontal,
  X,
  Settings,
} from "lucide-react";
import { useSessionStore } from "../stores/useSessionStore";
import { useUIStore } from "../stores/useUIStore";
import { api } from "../api/client";
import { formatSessionDate } from "../utils/format";
import type { Session } from "../types/api";

const MAX_RECENT_SESSIONS = 5;

export function WelcomePage() {
  const [inputValue, setInputValue] = useState("");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [showAllSessions, setShowAllSessions] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  const sessions = useSessionStore((s) => s.sessions);
  const setSessions = useSessionStore((s) => s.setSessions);
  const setCurrentSession = useSessionStore((s) => s.setCurrentSession);
  const setMessages = useSessionStore((s) => s.setMessages);
  const setError = useSessionStore((s) => s.setError);
  const setLoading = useSessionStore((s) => s.setLoading);
  const startDraftSession = useSessionStore((s) => s.startDraftSession);
  const theme = useUIStore((s) => s.theme);
  const toggleSettings = useUIStore((s) => s.toggleSettings);

  // Close menu when clicking outside
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        setShowAllSessions(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  // Refresh sessions and workspace on mount
  useEffect(() => {
    Promise.all([
      api.listSessions().then(({ sessions }) => setSessions(sessions)),
      api.getWorkspace().catch(() => null),
    ]).catch(() => {});
  }, [setSessions]);

  const [workspaceRoot, setWorkspaceRoot] = useState<string>("");
  useEffect(() => {
    api
      .getWorkspace()
      .then((info) => setWorkspaceRoot(info.workspace_root))
      .catch(() => setWorkspaceRoot(""));
  }, []);

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!inputValue.trim() || isSubmitting) return;

      setIsSubmitting(true);
      try {
        // Get workspace if not loaded yet
        let root = workspaceRoot;
        if (!root) {
          const info = await api.getWorkspace();
          root = info.workspace_root;
          setWorkspaceRoot(root);
        }

        // Create session with the input as title
        const { session_id } = await api.createSession({
          workspace_root: root,
          title: inputValue.trim(),
        });

        // Load the new session
        const [session, { messages, todos }] = await Promise.all([
          api.getSession(session_id),
          api.listMessages(session_id),
        ]);

        setCurrentSession(session);
        setMessages(messages);
        useSessionStore.getState().setTodos(todos ?? []);

        // Refresh sessions list
        const { sessions: updatedSessions } = await api.listSessions();
        setSessions(updatedSessions);
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Failed to create session",
        );
      } finally {
        setIsSubmitting(false);
        setInputValue("");
      }
    },
    [
      inputValue,
      isSubmitting,
      workspaceRoot,
      setCurrentSession,
      setMessages,
      setSessions,
      setError,
    ],
  );

  const handleSelectSession = useCallback(
    async (session: Session) => {
      try {
        setLoading(true);
        const [sessionDetail, { messages, todos }] = await Promise.all([
          api.getSession(session.session_id),
          api.listMessages(session.session_id),
        ]);
        setCurrentSession(sessionDetail);
        setMessages(messages);
        useSessionStore.getState().setTodos(todos ?? []);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to load session");
      } finally {
        setLoading(false);
      }
    },
    [setLoading, setCurrentSession, setMessages, setError],
  );

  const handleNewSessionClick = useCallback(() => {
    startDraftSession("New Session");
  }, [startDraftSession]);

  // Sort sessions by updated_at desc and take top 5
  const recentSessions = [...sessions]
    .sort(
      (a, b) =>
        new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
    )
    .slice(0, MAX_RECENT_SESSIONS);

  const hasMoreSessions = sessions.length > MAX_RECENT_SESSIONS;

  return (
    <div className="relative flex h-full flex-col items-center justify-center bg-white px-4 dark:bg-neutral-950">
      {/* Settings Button */}
      <button
        onClick={toggleSettings}
        className="absolute right-4 top-[max(1rem,env(safe-area-inset-top))] rounded-lg p-2 text-neutral-500 transition-colors hover:bg-neutral-100 hover:text-neutral-700 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-200"
        aria-label="Settings"
      >
        <Settings className="h-5 w-5" />
      </button>

      {/* Logo/Title */}
      <div className="mb-12 text-center">
        <h1 className="mb-2 text-4xl font-bold tracking-tight text-neutral-900 dark:text-neutral-100">
          TiDev
        </h1>
        <p className="text-sm text-neutral-500 dark:text-neutral-400">
          Your intelligent coding assistant
        </p>
      </div>

      {/* Input Box */}
      <div className="w-full max-w-2xl">
        <form onSubmit={handleSubmit} className="relative">
          <div className="relative rounded-2xl border border-neutral-200 bg-white shadow-lg dark:border-neutral-800 dark:bg-neutral-900">
            <input
              type="text"
              value={inputValue}
              onChange={(e) => setInputValue(e.target.value)}
              placeholder="What would you like to work on?"
              className="w-full rounded-2xl bg-transparent px-6 py-4 pr-16 text-base text-neutral-900 placeholder-neutral-400 outline-none dark:text-neutral-100 dark:placeholder-neutral-500"
              disabled={isSubmitting}
              autoFocus
            />
            <button
              type="submit"
              disabled={!inputValue.trim() || isSubmitting}
              className="absolute right-3 top-1/2 -translate-y-1/2 rounded-xl bg-neutral-900 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-neutral-800 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
            >
              {isSubmitting ? "..." : "Go"}
            </button>
          </div>
          <p className="mt-2 text-center text-xs text-neutral-400 dark:text-neutral-500">
            Press Enter to create a new session
          </p>
        </form>
      </div>

      {/* Recent Sessions */}
      {sessions.length > 0 && (
        <div className="mt-12 w-full max-w-2xl">
          <div className="mb-3 flex items-center justify-between">
            <h2 className="flex items-center gap-2 text-sm font-medium text-neutral-700 dark:text-neutral-300">
              <Clock className="h-4 w-4" />
              Recent Sessions
            </h2>
            {hasMoreSessions && (
              <div className="relative" ref={menuRef}>
                <button
                  onClick={() => setShowAllSessions(!showAllSessions)}
                  className="flex items-center gap-1 text-xs font-medium text-neutral-500 hover:text-neutral-700 dark:text-neutral-400 dark:hover:text-neutral-200"
                >
                  <MoreHorizontal className="h-3.5 w-3.5" />
                  More
                  <ChevronDown
                    className={`h-3 w-3 transition-transform ${showAllSessions ? "rotate-180" : ""}`}
                  />
                </button>

                {/* Dropdown Menu for All Sessions */}
                {showAllSessions && (
                  <div className="absolute right-0 top-full z-50 mt-1 w-80 rounded-lg border border-neutral-200 bg-white shadow-xl dark:border-neutral-700 dark:bg-neutral-900">
                    <div className="flex items-center justify-between border-b border-neutral-100 px-3 py-2 dark:border-neutral-800">
                      <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400">
                        All Sessions ({sessions.length})
                      </span>
                      <button
                        onClick={() => setShowAllSessions(false)}
                        className="rounded p-1 text-neutral-400 hover:bg-neutral-100 hover:text-neutral-600 dark:hover:bg-neutral-800"
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </div>
                    <div className="max-h-80 overflow-y-auto py-1">
                      {[...sessions]
                        .sort(
                          (a, b) =>
                            new Date(b.updated_at).getTime() -
                            new Date(a.updated_at).getTime(),
                        )
                        .map((session) => (
                          <button
                            key={session.session_id}
                            onClick={() => {
                              handleSelectSession(session);
                              setShowAllSessions(false);
                            }}
                            className="flex w-full items-center gap-3 px-3 py-2 text-left hover:bg-neutral-50 dark:hover:bg-neutral-800"
                          >
                            <MessageSquare className="h-4 w-4 shrink-0 text-neutral-400" />
                            <div className="min-w-0 flex-1">
                              <p className="truncate text-sm text-neutral-900 dark:text-neutral-100">
                                {session.title}
                              </p>
                              <p className="text-xs text-neutral-400">
                                {session.model_display_name} •{" "}
                                {formatSessionDate(session.updated_at)}
                              </p>
                            </div>
                          </button>
                        ))}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>

          <div className="space-y-1">
            {recentSessions.map((session) => (
              <button
                key={session.session_id}
                onClick={() => handleSelectSession(session)}
                className="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left transition-colors hover:bg-neutral-100 dark:hover:bg-neutral-900"
              >
                <MessageSquare className="h-4 w-4 shrink-0 text-neutral-400" />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium text-neutral-900 dark:text-neutral-100">
                    {session.title}
                  </p>
                  <p className="text-xs text-neutral-500 dark:text-neutral-400">
                    {session.model_display_name} •{" "}
                    {formatSessionDate(session.updated_at)}
                  </p>
                </div>
              </button>
            ))}
          </div>
        </div>
      )}

      {/* Quick Start Hint */}
      {sessions.length === 0 && (
        <div className="mt-12 text-center">
          <p className="text-sm text-neutral-400 dark:text-neutral-500">
            No sessions yet. Type above to get started!
          </p>
        </div>
      )}
    </div>
  );
}
