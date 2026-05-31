import { useCallback, useEffect, useRef, useState } from "react";
import {
  ChevronDown,
  MessageSquare,
  Clock,
  MoreHorizontal,
  X,
  Trash2,
} from "lucide-react";
import { useSessionStore } from "../stores/useSessionStore";
import { useUIStore } from "../stores/useUIStore";
import { SmartInput } from "./SmartInput";
import { SkillsDialog } from "./chat/SkillsDialog";
import { ConnectDialog } from "./chat/ConnectDialog";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { api } from "../api/client";
import { formatSessionDate } from "../utils/format";
import type { Session } from "../types/api";

const MAX_RECENT_SESSIONS = 5;

export function WelcomePage() {
  const [showAllSessions, setShowAllSessions] = useState(false);
  const [skillsDialogOpen, setSkillsDialogOpen] = useState(false);
  const [connectDialogOpen, setConnectDialogOpen] = useState(false);
  const [sessionToDelete, setSessionToDelete] = useState<Session | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  const sessions = useSessionStore((s) => s.sessions);
  const setSessions = useSessionStore((s) => s.setSessions);
  const setCurrentSession = useSessionStore((s) => s.setCurrentSession);
  const setMessages = useSessionStore((s) => s.setMessages);
  const setError = useSessionStore((s) => s.setError);
  const setLoading = useSessionStore((s) => s.setLoading);
  const currentSessionId = useSessionStore((s) => s.currentSessionId);
  const currentRequestId = useSessionStore((s) => s.currentRequestId);
  const isStreaming = useUIStore((s) => s.isStreaming);
  const setStreaming = useUIStore((s) => s.setStreaming);

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

  const handleSlashCommand = useCallback((command: string) => {
    if (command === "skills" || command === "skill") {
      setSkillsDialogOpen(true);
    } else if (command === "connect") {
      setConnectDialogOpen(true);
    }
    // Other commands are not applicable on welcome page
  }, []);

  const handleSubmit = useCallback(
    async (payload: {
      inputValue: string;
      modelId: string | null;
      providerId: string | null;
      mode: "plan" | "build";
      thinkingLevel: string | null;
    }) => {
      try {
        // Get workspace if not loaded yet
        let root = workspaceRoot;
        if (!root) {
          const info = await api.getWorkspace();
          root = info.workspace_root;
          setWorkspaceRoot(root);
        }

        // Create session, passing the selected provider/model if any
        const { session_id } = await api.createSession({
          workspace_root: root,
          title: payload.inputValue,
          provider_id: payload.providerId ?? undefined,
          model_id: payload.modelId ?? undefined,
        });

        // Load the new session
        const [session, { messages, todos }] = await Promise.all([
          api.getSession(session_id),
          api.listMessages(session_id),
        ]);

        setCurrentSession(session);
        setMessages(messages);
        useSessionStore.getState().setTodos(todos ?? []);

        // Send the user message, passing the selected provider/model
        const requestBody: {
          content: string;
          mode?: string;
          model_id?: string;
          provider_id?: string;
          thinking_level?: string;
        } = { content: payload.inputValue };

        if (payload.mode) requestBody.mode = payload.mode;
        if (payload.thinkingLevel)
          requestBody.thinking_level = payload.thinkingLevel;
        if (payload.modelId) requestBody.model_id = payload.modelId;
        if (payload.providerId) requestBody.provider_id = payload.providerId;

        // Start streaming state before sending
        setStreaming(true);

        const { request_id } = await api.sendMessage(session_id, requestBody);
        useSessionStore.getState().setCurrentRequestId(request_id);

        // Add the user message directly to the store so it appears immediately.
        // We avoid calling api.listMessages() here because it races with the
        // SSE handler's handleMessageComplete: if that handler has already
        // fetched the full [user, assistant] messages from the API, our
        // subsequent setMessages() would overwrite them with stale data.
        const pendingId = `pending-${Date.now()}`;
        console.log(
          "[WelcomePage] addMessage before:",
          JSON.stringify(
            useSessionStore.getState().messages.map((m) => ({
              id: m.id,
              role: m.role,
              content: m.content.substring(0, 20),
            })),
          ),
        );
        useSessionStore.getState().addMessage({
          id: pendingId,
          role: "user",
          content: payload.inputValue,
          created_at: new Date().toISOString(),
        });
        console.log(
          "[WelcomePage] addMessage after:",
          JSON.stringify(
            useSessionStore.getState().messages.map((m) => ({
              id: m.id,
              role: m.role,
              content: m.content.substring(0, 20),
            })),
          ),
        );

        // Refresh sessions list
        const { sessions: updatedSessions } = await api.listSessions();
        setSessions(updatedSessions);
      } catch (err) {
        setStreaming(false);
        setError(
          err instanceof Error ? err.message : "Failed to create session",
        );
      }
    },
    [
      workspaceRoot,
      setCurrentSession,
      setMessages,
      setSessions,
      setError,
      setStreaming,
    ],
  );

  const handleStop = useCallback(async () => {
    if (currentSessionId && currentRequestId) {
      try {
        await api.abortRequest(currentSessionId, {
          request_id: currentRequestId,
        });
      } catch {
        // Ignore abort errors
      }
    }
    setStreaming(false);
  }, [currentSessionId, currentRequestId, setStreaming]);

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

  const handleDeleteSession = useCallback(
    async (session: Session) => {
      setIsDeleting(true);
      try {
        await api.deleteSession(session.session_id);
        // Refresh sessions list
        const { sessions: updatedSessions } = await api.listSessions();
        setSessions(updatedSessions);
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Failed to delete session",
        );
      } finally {
        setIsDeleting(false);
        setSessionToDelete(null);
      }
    },
    [setSessions, setError],
  );

  // Sort sessions by updated_at desc and take top 5
  const recentSessions = [...sessions]
    .sort(
      (a, b) =>
        new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
    )
    .slice(0, MAX_RECENT_SESSIONS);

  const hasMoreSessions = sessions.length > MAX_RECENT_SESSIONS;

  return (
    <div className="relative flex h-full flex-col items-center justify-center bg-white px-4 motion-safe:animate-fade-in-up dark:bg-neutral-950">
      {/* Logo/Title */}
      <div className="mb-12 text-center">
        <h1 className="mb-2 text-4xl font-bold tracking-tight text-neutral-900 dark:text-neutral-100">
          tidev
        </h1>
        <p className="text-sm text-neutral-500 dark:text-neutral-400">
          Your intelligent coding assistant
        </p>
      </div>

      {/* Smart Input Box */}
      <div className="w-full max-w-2xl">
        <SmartInput
          onSubmit={handleSubmit}
          onSlashCommand={handleSlashCommand}
          placeholder="What would you like to work on?"
          multiline={true}
          autoFocus
          className="w-full"
          workspacePath={workspaceRoot}
          isStreaming={isStreaming}
          onStop={handleStop}
        />
      </div>

      {/* Recent Sessions */}
      {sessions.length > 0 && (
        <div
          className="mt-12 w-full max-w-2xl motion-safe:animate-fade-in-up"
          style={{ animationDelay: "50ms" }}
        >
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
                            className="group flex w-full items-center gap-3 px-3 py-2 text-left hover:bg-neutral-50 dark:hover:bg-neutral-800"
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
                            <button
                              onClick={(e) => {
                                e.stopPropagation();
                                setSessionToDelete(session);
                              }}
                              className="shrink-0 rounded p-1 text-neutral-400 opacity-100 transition-opacity hover-only:opacity-0 hover-only:group-hover:opacity-100 hover-only:hover:bg-red-100 hover-only:hover:text-red-600 hover-only:dark:hover:bg-red-900/30 hover-only:dark:hover:text-red-400"
                            >
                              <Trash2 className="h-3.5 w-3.5" />
                            </button>
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
                className="group flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left transition-colors hover:bg-neutral-100 dark:hover:bg-neutral-900"
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
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    setSessionToDelete(session);
                  }}
                  className="shrink-0 rounded p-1 text-neutral-400 opacity-100 transition-opacity hover-only:opacity-0 hover-only:group-hover:opacity-100 hover-only:hover:bg-red-100 hover-only:hover:text-red-600 hover-only:dark:hover:bg-red-900/30 hover-only:dark:hover:text-red-400"
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
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

      {/* Skills Dialog */}
      <SkillsDialog
        isOpen={skillsDialogOpen}
        onClose={() => setSkillsDialogOpen(false)}
        onSelect={(content) => {
          // On welcome page, we just close the dialog
          // The skill content would need to be inserted into input
          setSkillsDialogOpen(false);
        }}
      />

      {/* Connect Dialog */}
      <ConnectDialog
        isOpen={connectDialogOpen}
        onClose={() => setConnectDialogOpen(false)}
      />

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
          if (sessionToDelete) handleDeleteSession(sessionToDelete);
        }}
        onCancel={() => setSessionToDelete(null)}
      />
    </div>
  );
}
