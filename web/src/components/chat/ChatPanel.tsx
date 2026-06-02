import { useMemo, useEffect, useState, useCallback, useRef } from "react";
import { ArrowDown } from "lucide-react";
import { useSessionStore } from "../../stores/useSessionStore";
import { useUIStore } from "../../stores/useUIStore";
import { usePermissionStore } from "../../stores/usePermissionStore";
import { api } from "../../api/client";
import { buildRounds } from "../../utils/round";
import { VirtualMessageList } from "./VirtualMessageList";
import { useMessageVirtualizer } from "../../hooks/useMessageVirtualizer";
import { useChatAutoScroll } from "../../hooks/useChatAutoScroll";
import { MessageInput } from "./MessageInput";
import { MessageDialog } from "./MessageDialog";
import { RenameDialog } from "./RenameDialog";
import { SkillsDialog } from "./SkillsDialog";
import { ConnectDialog } from "./ConnectDialog";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { PermissionCard } from "./PermissionCard";

export function ChatPanel() {
  const messages = useSessionStore((s) => s.messages);
  const currentSessionId = useSessionStore((s) => s.currentSessionId);
  const currentSession = useSessionStore((s) => s.currentSession);
  const isDraftSession = useSessionStore((s) => s.isDraftSession);

  const isStreaming = useUIStore((s) => s.isStreaming);

  // buildRounds directly from messages — streaming assistant messages
  // (with streaming: true) naturally produce a streaming-status round.
  // No separate streamingRound merging needed.
  const rounds = useMemo(() => buildRounds(messages), [messages]);

  // Virtual list + auto-scroll
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const { virtualItems, totalSize, isVirtualized, measureElement } = useMessageVirtualizer(
    scrollContainerRef,
    rounds,
  );

  const { handleScroll, scrollToBottom, showScrollButton, endRef } = useChatAutoScroll(
    scrollContainerRef,
    isStreaming,
  );

  // Scroll to bottom once when a session is first loaded
  const scrolledSessionRef = useRef<string | null>(null);
  useEffect(() => {
    if (currentSessionId && currentSessionId !== scrolledSessionRef.current && rounds.length > 0) {
      scrolledSessionRef.current = currentSessionId;
      scrollToBottom(true);
    }
  }, [currentSessionId, rounds.length, scrollToBottom]);

  // Undo state
  const [undoDialogOpen, setUndoDialogOpen] = useState(false);
  const [undoTargetMessageId, setUndoTargetMessageId] = useState<string | null>(null);
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

  // Connect dialog state
  const [connectDialogOpen, setConnectDialogOpen] = useState(false);

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
      await api.revertToMessage(currentSessionId, undoTargetMessageId);

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
    } else if (command === "connect") {
      setConnectDialogOpen(true);
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
        const [session, { messages: forkedMessages, todos }] = await Promise.all([
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
        // Update session detail in store
        const session = await api.getSession(currentSessionId);
        useSessionStore.getState().setCurrentSession(session);
        // Also update title in the sessions list for the sidebar
        useSessionStore.getState().updateSessionTitle(currentSessionId, title);
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
  const canUndo = !!currentSessionId && !isStreaming;

  return (
    <div className="flex h-full flex-col bg-white dark:bg-neutral-950">
      {/* Top safe-area spacer (replaces removed header spacing) */}
      <div className="h-0 pt-[max(0.25rem,env(safe-area-inset-top))]" />

      {/* Messages Area */}
      <div className="relative flex-1 overflow-hidden">
        <div
          ref={scrollContainerRef}
          onScroll={handleScroll}
          className="h-full overflow-y-auto overflow-x-hidden"
        >
          {!currentSessionId && !isDraftSession ? (
            <div className="flex h-full items-center justify-center">
              <div className="text-center">
                <p className="text-neutral-500 dark:text-neutral-400">
                  Select a session or create a new one to start chatting
                </p>
              </div>
            </div>
          ) : rounds.length === 0 && !isStreaming ? (
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
            <VirtualMessageList
              entries={rounds}
              virtualItems={virtualItems}
              totalSize={totalSize}
              isVirtualized={isVirtualized}
              measureElement={measureElement}
              onUndoRequest={handleUndoRequest}
              canUndo={canUndo}
            />
          )}
          <div ref={endRef} />
        </div>

        {/* Scroll-to-bottom floating button */}
        {showScrollButton && (
          <button
            onClick={() => scrollToBottom()}
            className="absolute bottom-4 right-6 z-10 flex items-center justify-center rounded-full bg-neutral-500/60 p-2.5 text-white shadow-lg backdrop-blur-sm transition-all hover:bg-neutral-500/80 active:scale-95 dark:bg-neutral-500/60 dark:hover:bg-neutral-500/80"
            aria-label="Scroll to latest"
          >
            <ArrowDown className="h-4 w-4" />
          </button>
        )}
      </div>

      {/* Permission Request Cards */}
      {currentSessionId && <PermissionArea sessionId={currentSessionId} />}

      {/* Input Area */}
      <MessageInput onSlashCommand={handleSlashCommand} skillInsert={skillInsert} />

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

      {/* Connect Dialog (/connect command) */}
      <ConnectDialog isOpen={connectDialogOpen} onClose={() => setConnectDialogOpen(false)} />
    </div>
  );
}

/** Renders pending permission request cards for the current session */
function PermissionArea({ sessionId }: { sessionId: string }) {
  const pendingPermissions = usePermissionStore((s) => s.pendingPermissions);
  const removePermission = usePermissionStore((s) => s.removePermission);

  const sessionPermissions = pendingPermissions.filter((p) => p.sessionId === sessionId);

  if (sessionPermissions.length === 0) return null;

  const handleResponse = (permissionId: string, response: "once" | "always" | "deny") => {
    // For now, tools needing permission are auto-rejected by the backend.
    // This handler will send the response to the backend once the
    // permission response endpoint is implemented.
    console.log(`[Permission] ${response} permission ${permissionId}`);
    removePermission(permissionId);
  };

  return (
    <div className="px-4">
      {sessionPermissions.map((perm) => (
        <PermissionCard
          key={perm.id}
          permission={perm}
          onResponse={(response) => handleResponse(perm.id, response)}
        />
      ))}
    </div>
  );
}
