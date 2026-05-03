import { useCallback } from 'react';
import { Plus, Trash2 } from 'lucide-react';
import { useSessionStore } from '../../stores/useSessionStore';
import { useUIStore } from '../../stores/useUIStore';
import { api } from '../../api/client';
import { formatSessionDate } from '../../utils/format';

export function LeftSidebar() {
  const sessions = useSessionStore((s) => s.sessions);
  const currentSessionId = useSessionStore((s) => s.currentSessionId);
  const isDraftSession = useSessionStore((s) => s.isDraftSession);
  const draftTitle = useSessionStore((s) => s.draftTitle);
  const isLoading = useSessionStore((s) => s.isLoading);

  const startDraftSession = useSessionStore((s) => s.startDraftSession);
  const setCurrentSession = useSessionStore((s) => s.setCurrentSession);
  const setMessages = useSessionStore((s) => s.setMessages);
  const removeSession = useSessionStore((s) => s.removeSession);
  const setError = useSessionStore((s) => s.setError);
  const setLoading = useSessionStore((s) => s.setLoading);
  const closeMobileMenu = useUIStore((s) => s.closeMobileMenu);

  const goToWelcome = useSessionStore((s) => s.goToWelcome);

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
        setError(err instanceof Error ? err.message : 'Failed to load session');
      } finally {
        setLoading(false);
      }
    },
    [setLoading, setCurrentSession, setMessages, closeMobileMenu, setError]
  );

  const handleDeleteSession = useCallback(
    async (sessionId: string) => {
      if (!confirm('Delete this session?')) return;
      try {
        await api.deleteSession(sessionId);
        removeSession(sessionId);
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to delete session');
      }
    },
    [removeSession, setError]
  );

  return (
    <div className="flex h-full flex-col bg-neutral-50 dark:bg-neutral-900">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 pt-[max(0.75rem,env(safe-area-inset-top))] dark:border-neutral-800">
        <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">Sessions</h2>
        <button
          onClick={handleNewSession}
          className="flex items-center gap-1 rounded bg-neutral-900 px-3 py-1.5 text-xs font-medium text-white hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
          aria-label="New session"
        >
          <Plus className="h-4 w-4" />
          <span>New</span>
        </button>
      </div>

      {/* Session List */}
      <div className="flex-1 overflow-y-auto">
        {isDraftSession && (
          <div className="border-b border-neutral-200 dark:border-neutral-800">
            <button className="flex w-full items-center bg-blue-50 px-4 py-3 text-left dark:bg-blue-950/30">
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium text-blue-900 dark:text-blue-100">{draftTitle}</p>
                <p className="mt-0.5 text-xs text-blue-600 dark:text-blue-400">Draft • Type to create</p>
              </div>
            </button>
          </div>
        )}

        {sessions.length === 0 && !isDraftSession ? (
          <div className="p-4 text-center text-sm text-neutral-500 dark:text-neutral-400">
            No sessions yet
          </div>
        ) : (
          <ul className="divide-y divide-neutral-100 dark:divide-neutral-800">
            {sessions.map((session) => (
              <li key={session.session_id} className="group relative">
                <button
                  onClick={() => handleSelectSession(session.session_id)}
                  className={`flex w-full items-center px-4 py-3 text-left hover:bg-neutral-100 dark:hover:bg-neutral-800 ${
                    currentSessionId === session.session_id
                      ? 'bg-neutral-100 dark:bg-neutral-800'
                      : ''
                  }`}
                >
                  <div className="min-w-0 flex-1 pr-8">
                    <p className="truncate text-sm font-medium text-neutral-900 dark:text-neutral-100">
                      {session.title}
                    </p>
                    <p className="mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
                      {session.model_display_name} • {formatSessionDate(session.updated_at)}
                    </p>
                  </div>
                </button>
                <button
                  onClick={() => handleDeleteSession(session.session_id)}
                  className="absolute right-3 top-1/2 -translate-y-1/2 rounded p-1 text-neutral-400 opacity-0 hover:text-red-600 group-hover:opacity-100 dark:text-neutral-500 dark:hover:text-red-400"
                  aria-label="Delete session"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
