import type { ReactNode } from "react";
import { Folder } from "lucide-react";
import { useTranslation } from "react-i18next";

import type {
  ApprovedTool,
  FrontendRequest,
  Model,
  MessageRecord,
  Session,
  TodoItem,
} from "../../types/api";
import type { InstructionNotice, StreamMessage } from "../../types/chat";
import { ChatComposer } from "./ChatComposer";
import type { PendingImage } from "../../utils/imageAttachments";
import { ApprovalCard, MessageList } from "./MessageList";
import { SessionSidebar } from "./SessionSidebar";
import { Button } from "../ui";
import { shortPath } from "../../utils/chat";

export interface ChatPanelProps {
  loading: boolean;
  loadingMoreSessions: boolean;
  hasMoreSessions: boolean;
  sessions: Session[];
  workspaceRoots: string[];
  workspaceRootFilter: string | null;
  selectedSessionId: string | null;
  selectedSession: Session | undefined;
  activeModel: Model | undefined;
  messages: MessageRecord[];
  streams: StreamMessage[];
  instructionNotices: InstructionNotice[];
  requests: FrontendRequest[];
  todos: TodoItem[];
  error: string | null;
  sessionSearch: string;
  renamingSessionId: string | null;
  renameValue: string;
  draft: string;
  mode: "build" | "plan";
  models: Model[];
  thinkingLevel: string | undefined;
  enterToSend: boolean;
  sending: boolean;
  canceling: boolean;
  mobileSidebarOpen: boolean;
  fileMention: { query: string; atPos: number } | null;
  fileMentionIndex: number;
  pendingImages: PendingImage[];
  welcome: ReactNode;
  onSessionSearchChange: (value: string) => void;
  onWorkspaceRootFilterChange: (workspaceRoot: string | null) => void;
  onLoadMoreSessions: () => void;
  onCreateSession: () => void;
  onSelectSession: (sessionId: string) => void;
  onStartRename: (session: Session) => void;
  onRenameChange: (value: string) => void;
  onRename: (sessionId: string) => void;
  onCancelRename: () => void;
  onDeleteSession: (session: Session) => void;
  onRevert: (messageId: string) => void;
  onFork: (messageId: string) => void;
  onRetryProviderError: (messageId: string) => void;
  onRespond: (requestId: string, tools: ApprovedTool[]) => void;
  onMobileSidebarClose: () => void;
  onDraftChange: (value: string) => void;
  onModeChange: (mode: "build" | "plan") => void;
  onSelectModel: (model: Model) => void;
  onSelectThinkingLevel: (level: string) => void;
  onSubmit: () => void;
  onCancel: () => void;
  onFileMentionChange: (text: string, cursor: number) => void;
  onFileMentionIndexChange: (index: number) => void;
  onFileSelect: (path: string) => number | undefined;
  onFileMentionClose: () => void;
  onImagesPasted: (files: File[]) => void;
  onRemoveImage: (id: string) => void;
  focusComposer?: boolean;
  onComposerFocus?: () => void;
  scrollToBottomRequest?: number;
}

export function ChatPanel({
  loading,
  loadingMoreSessions,
  hasMoreSessions,
  sessions,
  workspaceRoots,
  workspaceRootFilter,
  selectedSessionId,
  selectedSession,
  activeModel,
  messages,
  streams,
  instructionNotices,
  requests,
  todos,
  error,
  sessionSearch,
  renamingSessionId,
  renameValue,
  draft,
  mode,
  models,
  thinkingLevel,
  enterToSend,
  sending,
  canceling,
  mobileSidebarOpen,
  fileMention,
  fileMentionIndex,
  pendingImages,
  welcome,
  onSessionSearchChange,
  onWorkspaceRootFilterChange,
  onLoadMoreSessions,
  onCreateSession,
  onSelectSession,
  onStartRename,
  onRenameChange,
  onRename,
  onCancelRename,
  onDeleteSession,
  onRevert,
  onFork,
  onRetryProviderError,
  onRespond,
  onMobileSidebarClose,
  onDraftChange,
  onModeChange,
  onSelectModel,
  onSelectThinkingLevel,
  onSubmit,
  onCancel,
  onFileMentionChange,
  onFileMentionIndexChange,
  onFileSelect,
  onFileMentionClose,
  onImagesPasted,
  onRemoveImage,
  focusComposer = false,
  onComposerFocus,
  scrollToBottomRequest = 0,
}: ChatPanelProps) {
  const { t } = useTranslation();
  const pendingRequests = requests.filter((request) => request.session_id === selectedSessionId);
  const sessionModel = selectedSession
    ? models.find(
        (model) =>
          model.provider_id === selectedSession.provider_id &&
          model.model_id === selectedSession.model_id,
      )
    : undefined;
  const contextWindow = sessionModel?.context_window ?? activeModel?.context_window;

  const handleSelectSession = (sessionId: string) => {
    onMobileSidebarClose();
    onSelectSession(sessionId);
  };

  const handleCreateSession = () => {
    onMobileSidebarClose();
    onCreateSession();
  };

  return (
    <>
      <Button
        type="button"
        className={
          mobileSidebarOpen
            ? "mobile-sidebar-backdrop visible ui-backdrop-button"
            : "mobile-sidebar-backdrop ui-backdrop-button"
        }
        onClick={onMobileSidebarClose}
        aria-label={t("Close conversations")}
        variant="ghost"
        size="sm"
      />
      <SessionSidebar
        loading={loading}
        loadingMore={loadingMoreSessions}
        hasMore={hasMoreSessions}
        mobileOpen={mobileSidebarOpen}
        sessions={sessions}
        workspaceRoots={workspaceRoots}
        workspaceRootFilter={workspaceRootFilter}
        selectedSessionId={selectedSessionId}
        search={sessionSearch}
        renamingSessionId={renamingSessionId}
        renameValue={renameValue}
        onSearchChange={onSessionSearchChange}
        onWorkspaceRootFilterChange={onWorkspaceRootFilterChange}
        onLoadMore={onLoadMoreSessions}
        onCreate={handleCreateSession}
        onSelect={handleSelectSession}
        onStartRename={onStartRename}
        onRenameChange={onRenameChange}
        onRename={onRename}
        onCancelRename={onCancelRename}
        onDelete={onDeleteSession}
      />
      <section className="chat-panel">
        {selectedSessionId === null ? (
          welcome
        ) : (
          <>
            {selectedSession ? (
              <header className="session-context">
                <div>
                  <span className="session-context-title">
                    {selectedSession.title || t("Untitled conversation")}
                  </span>
                  <span
                    className="session-context-workspace"
                    title={selectedSession.workspace_root}
                  >
                    <Folder size={13} aria-hidden="true" />
                    {t("Session directory")}: {shortPath(selectedSession.workspace_root)}
                  </span>
                </div>
                <span className="session-context-model">{selectedSession.model_display_name}</span>
              </header>
            ) : null}
            <div className="message-stage">
              <MessageList
                messages={messages}
                streams={streams}
                instructionNotices={instructionNotices}
                sessionId={selectedSessionId}
                session={selectedSession}
                models={models}
                workspaceRoot={selectedSession?.workspace_root}
                onRevert={onRevert}
                onFork={onFork}
                onRetryProviderError={onRetryProviderError}
                scrollToBottomRequest={scrollToBottomRequest}
              />
              {pendingRequests.map((request) => (
                <ApprovalCard
                  key={request.request_id}
                  request={request}
                  onRespond={(tools) => onRespond(request.request_id, tools)}
                />
              ))}
              {error ? <div className="error-banner">{error}</div> : null}
            </div>
            <ChatComposer
              draft={draft}
              mode={mode}
              messages={messages}
              models={models}
              activeModel={activeModel}
              contextWindow={contextWindow}
              thinkingLevel={thinkingLevel}
              todos={todos}
              enterToSend={enterToSend}
              isBusy={selectedSession?.busy ?? false}
              sending={sending}
              canceling={canceling}
              selectedSessionId={selectedSessionId}
              fileMention={fileMention}
              fileMentionIndex={fileMentionIndex}
              pendingImages={pendingImages}
              onDraftChange={onDraftChange}
              onModeChange={onModeChange}
              onSelectModel={onSelectModel}
              onSelectThinkingLevel={onSelectThinkingLevel}
              onSubmit={onSubmit}
              onCancel={onCancel}
              onFileMentionChange={onFileMentionChange}
              onFileMentionIndexChange={onFileMentionIndexChange}
              onFileSelect={onFileSelect}
              onFileMentionClose={onFileMentionClose}
              onImagesPasted={onImagesPasted}
              onRemoveImage={onRemoveImage}
              autoFocus={focusComposer}
              onAutoFocus={onComposerFocus}
            />
          </>
        )}
      </section>
    </>
  );
}
