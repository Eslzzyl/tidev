import { useCallback, useState, useRef, useEffect } from "react";
import { Plus, Trash2, Search, Pencil } from "lucide-react";
import { useSessionStore } from "../../stores/useSessionStore";
import { useUIStore } from "../../stores/useUIStore";
import { api } from "../../api/client";
import { formatSessionDate } from "../../utils/format";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import type { Session } from "../../types/api";

export function LeftSidebar() {
  const sessions = useSessionStore((s) => s.sessions);
  const currentSessionId = useSessionStore((s) => s.currentSessionId);
  const isDraftSession = useSessionStore((s) => s.isDraftSession);
  const draftTitle = useSessionStore((s) => s.draftTitle);
  const isStreaming = useUIStore((s) => s.isStreaming);

  const setCurrentSession = useSessionStore((s) => s.setCurrentSession);
  const setMessages = useSessionStore((s) => s.setMessages);
  const removeSession = useSessionStore((s) => s.removeSession);
  const setError = useSessionStore((s) => s.setError);
  const setLoading = useSessionStore((s) => s.setLoading);
  const closeMobileMenu = useUIStore((s) => s.closeMobileMenu);

  const goToWelcome = useSessionStore((s) => s.goToWelcome);

  const [searchQuery, setSearchQuery] = useState("");
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [sessionToDelete, setSessionToDelete] = useState<Session | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const renameInputRef = useRef<HTMLInputElement>(null);

  // Focus rename input when renaming starts
  useEffect(() => {
    if (renamingId && renameInputRef.current) {
      renameInputRef.current.focus();
      renameInputRef.current.select();
    }
  }, [renamingId]);

  const filteredSessions = searchQuery.trim()
    ? sessions.filter((s) => s.title.toLowerCase().includes(searchQuery.toLowerCase()))
    : sessions;

  const handleNewSession = useCallback(() => {
    goToWelcome();
    closeMobileMenu();
  }, [goToWelcome, closeMobileMenu]);

  const handleSelectSession = useCallback(
    async (sessionId: string) => {
      try {
        setLoading(true);
        const [session, { messages, todos }] = await Promise.all([
          api.getSession(sessionId),
          api.listMessages(sessionId),
        ]);
        setCurrentSession(session);
        setMessages(messages);
        useSessionStore.getState().setTodos(todos ?? []);
        closeMobileMenu();
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to load session");
      } finally {
        setLoading(false);
      }
    },
    [setLoading, setCurrentSession, setMessages, closeMobileMenu, setError],
  );

  const handleDeleteSession = useCallback(
    async (sessionId: string) => {
      setIsDeleting(true);
      try {
        await api.deleteSession(sessionId);
        removeSession(sessionId);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to delete session");
      } finally {
        setIsDeleting(false);
        setSessionToDelete(null);
      }
    },
    [removeSession, setError],
  );

  const handleStartRename = useCallback((sessionId: string, currentTitle: string) => {
    setRenamingId(sessionId);
    setRenameValue(currentTitle);
  }, []);

  const handleConfirmRename = useCallback(
    async (sessionId: string) => {
      const trimmed = renameValue.trim();
      if (!trimmed) {
        setRenamingId(null);
        return;
      }
      try {
        await api.renameSession(sessionId, trimmed);
        // Refresh sessions list
        const { sessions } = await api.listSessions();
        useSessionStore.getState().setSessions(sessions);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Failed to rename session");
      } finally {
        setRenamingId(null);
      }
    },
    [renameValue, setError],
  );

  const handleRenameKeyDown = useCallback(
    (e: React.KeyboardEvent, sessionId: string) => {
      if (e.key === "Enter") {
        handleConfirmRename(sessionId);
      } else if (e.key === "Escape") {
        setRenamingId(null);
      }
    },
    [handleConfirmRename],
  );

  return (
    <div className="flex h-full min-h-0 flex-col bg-white dark:bg-neutral-950">
      {/* Header: search + new session */}
      <div className="flex items-center gap-1 border-b border-neutral-200 p-2 dark:border-neutral-800">
        <div className="relative flex-1">
          <Search className="absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-neutral-400" />
          <input
            type="text"
            placeholder="Search sessions..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="w-full rounded border border-neutral-200 bg-white py-1 pl-7 pr-2 text-base outline-none transition-all duration-150 focus:border-neutral-400 focus:ring-1 focus:ring-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:focus:border-neutral-500 dark:focus:ring-neutral-600"
          />
        </div>
        <button
          onClick={handleNewSession}
          className="flex shrink-0 items-center justify-center rounded p-1 text-neutral-400 transition-all duration-150 hover:bg-neutral-100 active:scale-95 dark:hover:bg-neutral-800"
          aria-label="New session"
          title="New session"
        >
          <Plus className="h-4 w-4" />
        </button>
      </div>

      {/* Session List */}
      <div className="flex-1 overflow-y-auto min-h-0">
        {isDraftSession && (
          <div className="px-2 pt-2">
            <div className="flex w-full items-center rounded-lg bg-blue-50 px-3 py-2.5 text-left dark:bg-blue-950/30">
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium text-blue-900 dark:text-blue-100">
                  {draftTitle}
                </p>
                <p className="mt-0.5 text-xs text-blue-600 dark:text-blue-400">
                  Draft • Type to create
                </p>
              </div>
            </div>
          </div>
        )}

        {filteredSessions.length === 0 && !isDraftSession ? (
          <div className="p-4 text-center text-sm text-neutral-500 dark:text-neutral-400">
            {searchQuery.trim() ? "No sessions match your search" : "No sessions yet"}
          </div>
        ) : (
          <ul className="flex flex-col gap-1 p-2">
            {filteredSessions.map((session) => {
              const isActive = currentSessionId === session.session_id;
              const isRenaming = renamingId === session.session_id;

              return (
                <li key={session.session_id} className="group relative">
                  {isRenaming ? (
                    <div className="rounded-lg border border-blue-400 bg-white px-3 py-2 dark:border-blue-500 dark:bg-neutral-800">
                      <input
                        ref={renameInputRef}
                        type="text"
                        value={renameValue}
                        onChange={(e) => setRenameValue(e.target.value)}
                        onKeyDown={(e) => handleRenameKeyDown(e, session.session_id)}
                        onBlur={() => handleConfirmRename(session.session_id)}
                        className="w-full bg-transparent text-base outline-none dark:text-neutral-100"
                      />
                    </div>
                  ) : (
                    <button
                      onClick={() => handleSelectSession(session.session_id)}
                      onDoubleClick={() => handleStartRename(session.session_id, session.title)}
                      className={`flex w-full items-center rounded-lg px-3 py-2.5 text-left transition-all duration-150 active:scale-[0.99] ${
                        isActive
                          ? "bg-neutral-100 font-medium text-neutral-900 shadow-sm ring-1 ring-neutral-200 dark:bg-neutral-800 dark:text-neutral-100 dark:ring-neutral-700"
                          : "text-neutral-700 hover:bg-neutral-100 hover:text-neutral-900 dark:text-neutral-300 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
                      }`}
                    >
                      <div className="min-w-0 flex-1 pr-8">
                        <div className="flex items-center gap-2">
                          <p className="truncate text-sm">{session.title}</p>
                          {/* Status indicator */}
                          {isActive && isStreaming && (
                            <span className="shrink-0">
                              <span className="relative flex h-2 w-2">
                                <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-green-400 opacity-75" />
                                <span className="relative inline-flex h-2 w-2 rounded-full bg-green-500" />
                              </span>
                            </span>
                          )}
                        </div>
                        <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
                          {session.model_display_name} • {formatSessionDate(session.updated_at)}
                        </p>
                      </div>
                    </button>
                  )}

                  {/* Action buttons */}
                  {!isRenaming && (
                    <div className="absolute right-3 top-1/2 flex -translate-y-1/2 gap-0.5 opacity-100 transition-all duration-150 hover-only:opacity-0 hover-only:group-hover:opacity-100">
                      <button
                        onClick={() => handleStartRename(session.session_id, session.title)}
                        className="rounded p-1 text-neutral-400 transition-all duration-150 hover:text-blue-600 hover:bg-neutral-100 active:scale-95 dark:text-neutral-500 dark:hover:text-blue-400 dark:hover:bg-neutral-800"
                        aria-label="Rename session"
                        title="Rename"
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </button>
                      <button
                        onClick={() => setSessionToDelete(session)}
                        className="rounded p-1 text-neutral-400 transition-all duration-150 hover:text-red-600 hover:bg-neutral-100 active:scale-95 dark:text-neutral-500 dark:hover:text-red-400 dark:hover:bg-neutral-800"
                        aria-label="Delete session"
                        title="Delete"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>

      {/* Delete Session Confirmation */}
      <ConfirmDialog
        isOpen={sessionToDelete !== null}
        title="Delete session"
        message={
          sessionToDelete
            ? `Are you sure you want to delete "${sessionToDelete.title || "Untitled"}"?`
            : ""
        }
        confirmText="Delete"
        danger
        isLoading={isDeleting}
        onConfirm={() => {
          if (sessionToDelete) handleDeleteSession(sessionToDelete.session_id);
        }}
        onCancel={() => setSessionToDelete(null)}
      />
    </div>
  );
}
