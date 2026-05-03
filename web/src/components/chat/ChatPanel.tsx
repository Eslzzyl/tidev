import { useMemo, useEffect, useState, useCallback } from "react";
import { Menu, Settings, Info } from "lucide-react";
import { useSessionStore } from "../../stores/useSessionStore";
import { useUIStore } from "../../stores/useUIStore";
import { useSSE } from "../../hooks/useSSE";
import { api } from "../../api/client";
import { buildRounds } from "../../utils/round";
import { MessageRound } from "./MessageRound";
import { MessageInput } from "./MessageInput";
import { MessageDialog } from "./MessageDialog";
import { RenameDialog } from "./RenameDialog";
import { SkillsDialog } from "./SkillsDialog";
import { ConfirmDialog } from "../ui/ConfirmDialog";

export function ChatPanel() {
  const messages = useSessionStore((s) => s.messages);
  const currentSessionId = useSessionStore((s) => s.currentSessionId);
  const currentSession = useSessionStore((s) => s.currentSession);
  const isDraftSession = useSessionStore((s) => s.isDraftSession);
  const draftTitle = useSessionStore((s) => s.draftTitle);

  const toggleMobileMenu = useUIStore((s) => s.toggleMobileMenu);
  const toggleRightSidebar = useUIStore((s) => s.toggleRightSidebar);
  const toggleMobileRightSidebar = useUIStore(
    (s) => s.toggleMobileRightSidebar,
  );
  const toggleSettings = useUIStore((s) => s.toggleSettings);

  const streamingRound = useSSE(currentSessionId);

  const completedRounds = useMemo(() => buildRounds(messages), [messages]);

  const allRounds = useMemo(() => {
    const rounds = [...completedRounds];
    if (streamingRound) {
      rounds.push(streamingRound);
    }
    return rounds;
  }, [completedRounds, streamingRound]);

  // Auto-scroll
  const [messagesContainerRef, setMessagesContainerRef] =
    useState<HTMLDivElement | null>(null);
  const [messagesEndRef, setMessagesEndRef] = useState<HTMLDivElement | null>(
    null,
  );
  const [shouldAutoScroll, setShouldAutoScroll] = useState(true);
  const [isFirstLoad, setIsFirstLoad] = useState(true);

  useEffect(() => {
    if (isFirstLoad && allRounds.length > 0 && messagesEndRef) {
      messagesEndRef.scrollIntoView({ behavior: "instant" });
      setIsFirstLoad(false);
    }
  }, [allRounds.length, isFirstLoad, messagesEndRef]);

  useEffect(() => {
    if (!isFirstLoad && shouldAutoScroll && messagesEndRef) {
      messagesEndRef.scrollIntoView({ behavior: "smooth" });
    }
  });

  useEffect(() => {
    if (currentSessionId) {
      setIsFirstLoad(true);
    }
  }, [currentSessionId]);

  const handleScroll = useCallback(() => {
    if (!messagesContainerRef) return;
    const { scrollHeight, scrollTop, clientHeight } = messagesContainerRef;
    const isNearBottom = scrollHeight - scrollTop - clientHeight < 100;
    setShouldAutoScroll(isNearBottom);
  }, [messagesContainerRef]);

  const scrollToBottom = useCallback(() => {
    if (messagesEndRef) {
      messagesEndRef.scrollIntoView({ behavior: "smooth" });
      setShouldAutoScroll(true);
    }
  }, [messagesEndRef]);

  // Undo state
  const [undoDialogOpen, setUndoDialogOpen] = useState(false);
  const [undoTargetMessageId, setUndoTargetMessageId] = useState<string | null>(
    null,
  );
  const [isUndoing, setIsUndoing] = useState(false);
  const [undoError, setUndoError] = useState<string | null>(null);

  // Message dialog state
  const [messageDialogOpen, setMessageDialogOpen] = useState(false);
  const [isForking, setIsForking] = useState(false);

  // Rename dialog state
  const [renameDialogOpen, setRenameDialogOpen] = useState(false);
  const [isRenaming, setIsRenaming] = useState(false);

  // Skills dialog state
  const [skillsDialogOpen, setSkillsDialogOpen] = useState(false);
  const [skillInsert, setSkillInsert] = useState<{ text: string } | null>(null);

  const setMessages = useSessionStore((s) => s.setMessages);
  const setTodos = useSessionStore((s) => s.setTodos);

  const handleUndoRequest = useCallback((messageId: string) => {
    setUndoTargetMessageId(messageId);
    setUndoDialogOpen(true);
    setUndoError(null);
  }, []);

  const handleConfirmUndo = useCallback(async () => {
    if (!currentSessionId || !undoTargetMessageId) return;

    setIsUndoing(true);
    setUndoError(null);

    try {
      const result = await api.revertToMessage(
        currentSessionId,
        undoTargetMessageId,
      );

      // Refresh messages and todos after revert
      const { messages: updatedMessages, todos: updatedTodos } =
        await api.listMessages(currentSessionId);
      setMessages(updatedMessages);
      setTodos(updatedTodos);

      setUndoDialogOpen(false);
      setUndoTargetMessageId(null);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Failed to undo";
      setUndoError(message);
    } finally {
      setIsUndoing(false);
    }
  }, [currentSessionId, undoTargetMessageId, setMessages, setTodos]);

  const handleCancelUndo = useCallback(() => {
    setUndoDialogOpen(false);
    setUndoTargetMessageId(null);
    setUndoError(null);
  }, []);

  // Slash command handler
  const handleSlashCommand = useCallback((command: string) => {
    if (command === "message") {
      setMessageDialogOpen(true);
    } else if (command === "rename") {
      setRenameDialogOpen(true);
    } else if (command === "skills") {
      setSkillsDialogOpen(true);
    }
  }, []);

  // Fork handler
  const handleForkFromMessage = useCallback(
    async (messageId: string) => {
      if (!currentSessionId) return;

      setIsForking(true);
      try {
        const result = await api.forkSession(
          currentSessionId,
          messageId,
          `Fork: ${currentSession?.title || "New Session"}`,
        );

        // Navigate to the new forked session
        const [session, { messages: forkedMessages, todos }] =
          await Promise.all([
            api.getSession(result.session_id),
            api.listMessages(result.session_id),
          ]);

        useSessionStore.getState().setCurrentSession(session);
        useSessionStore.getState().setMessages(forkedMessages);
        useSessionStore.getState().setTodos(todos ?? []);

        // Update URL
        const url = new URL(window.location.href);
        url.searchParams.set("session", result.session_id);
        window.history.replaceState({}, "", url.toString());

        setMessageDialogOpen(false);
      } catch (error) {
        console.error("Fork failed:", error);
      } finally {
        setIsForking(false);
      }
    },
    [currentSessionId, currentSession],
  );

  // Rename handler
  const handleRenameConfirm = useCallback(
    async (title: string) => {
      if (!currentSessionId) return;

      setIsRenaming(true);
      try {
        await api.renameSession(currentSessionId, title);
        // Refresh session to get updated data
        const session = await api.getSession(currentSessionId);
        useSessionStore.getState().setCurrentSession(session);
        setRenameDialogOpen(false);
      } catch (error) {
        console.error("Rename failed:", error);
      } finally {
        setIsRenaming(false);
      }
    },
    [currentSessionId],
  );

  // Skill select handler
  const handleSkillSelect = useCallback((skillName: string) => {
    setSkillsDialogOpen(false);
    setSkillInsert({ text: `/skill ${skillName} ` });
  }, []);

  // Undo from message dialog
  const handleUndoFromDialog = useCallback((messageId: string) => {
    setMessageDialogOpen(false);
    // Reuse the existing undo flow
    setUndoTargetMessageId(messageId);
    setUndoDialogOpen(true);
    setUndoError(null);
  }, []);

  // Determine if undo should be disabled (during streaming or when there's no session)
  const canUndo = !!currentSessionId && !streamingRound;

  return (
    <div className="flex h-full flex-col bg-white dark:bg-neutral-950">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 pt-[max(0.75rem,env(safe-area-inset-top))] dark:border-neutral-800">
        <div className="flex items-center gap-3">
          <button
            onClick={toggleMobileMenu}
            className="rounded p-1 text-neutral-600 hover:bg-neutral-100 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Open menu"
          >
            <Menu className="h-5 w-5" />
          </button>

          {isDraftSession ? (
            <div>
              <h1 className="text-sm font-semibold text-blue-600 dark:text-blue-400">
                {draftTitle}
              </h1>
              <p className="text-xs text-neutral-500 dark:text-neutral-400">
                Draft Session
              </p>
            </div>
          ) : currentSession ? (
            <div>
              <h1 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
                {currentSession.title}
              </h1>
              <p className="text-xs text-neutral-500 dark:text-neutral-400">
                {currentSession.model_display_name}
              </p>
            </div>
          ) : (
            <h1 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
              Select a session
            </h1>
          )}
        </div>

        <div className="flex items-center gap-2">
          {/* Settings button */}
          <button
            onClick={toggleSettings}
            className="rounded p-2 text-neutral-600 hover:bg-neutral-100 dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Settings"
          >
            <Settings className="h-5 w-5" />
          </button>

          {/* Right sidebar toggle (desktop) */}
          <button
            onClick={toggleRightSidebar}
            className="hidden rounded p-2 text-neutral-600 hover:bg-neutral-100 md:block dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Toggle info panel"
          >
            <Info className="h-5 w-5" />
          </button>

          {/* Mobile right sidebar toggle */}
          <button
            onClick={toggleMobileRightSidebar}
            className="rounded p-2 text-neutral-600 hover:bg-neutral-100 md:hidden dark:text-neutral-400 dark:hover:bg-neutral-800"
            aria-label="Open info panel"
          >
            <Info className="h-5 w-5" />
          </button>
        </div>
      </div>

      {/* Messages Area */}
      <div
        ref={setMessagesContainerRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto"
      >
        {!currentSessionId && !isDraftSession ? (
          <div className="flex h-full items-center justify-center">
            <div className="text-center">
              <p className="text-neutral-500 dark:text-neutral-400">
                Select a session or create a new one to start chatting
              </p>
            </div>
          </div>
        ) : allRounds.length === 0 && !streamingRound ? (
          <div className="flex h-full items-center justify-center">
            <div className="text-center">
              <p className="text-neutral-500 dark:text-neutral-400">
                {isDraftSession
                  ? "Type your first message to create the session"
                  : "No messages yet. Start a conversation!"}
              </p>
            </div>
          </div>
        ) : (
          <div className="divide-y divide-neutral-100 dark:divide-neutral-900">
            {allRounds.map((round) => (
              <MessageRound
                key={round.id}
                round={round}
                onUndoRequest={handleUndoRequest}
                canUndo={canUndo}
              />
            ))}
          </div>
        )}
        <div ref={setMessagesEndRef} />
      </div>

      {/* Input Area */}
      <MessageInput
        onSlashCommand={handleSlashCommand}
        skillInsert={skillInsert}
      />

      {/* Undo Confirmation Dialog */}
      <ConfirmDialog
        isOpen={undoDialogOpen}
        title="Undo to this message?"
        message="This will restore files to the state before this message was sent. All messages after this point will be hidden but can be restored later."
        confirmText="Undo"
        cancelText="Cancel"
        onConfirm={handleConfirmUndo}
        onCancel={handleCancelUndo}
        isLoading={isUndoing}
      />

      {/* Undo Error Toast (simple inline display) */}
      {undoError && (
        <div className="absolute bottom-20 left-1/2 z-50 -translate-x-1/2 rounded-lg bg-red-600 px-4 py-2 text-sm text-white shadow-lg">
          {undoError}
        </div>
      )}

      {/* Message Dialog (/message command) */}
      <MessageDialog
        isOpen={messageDialogOpen}
        messages={messages}
        onClose={() => setMessageDialogOpen(false)}
        onFork={handleForkFromMessage}
        onUndo={handleUndoFromDialog}
        isUndoing={isUndoing}
        isForking={isForking}
      />

      {/* Rename Dialog (/rename command) */}
      <RenameDialog
        isOpen={renameDialogOpen}
        currentTitle={currentSession?.title || ""}
        onClose={() => setRenameDialogOpen(false)}
        onConfirm={handleRenameConfirm}
        isLoading={isRenaming}
      />

      {/* Skills Dialog (/skills command) */}
      <SkillsDialog
        isOpen={skillsDialogOpen}
        onClose={() => setSkillsDialogOpen(false)}
        onSelect={handleSkillSelect}
      />
    </div>
  );
}
