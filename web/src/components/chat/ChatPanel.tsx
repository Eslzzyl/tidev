import { useTranslation } from "react-i18next";

import type {
  ApprovedTool,
  FrontendRequest,
  Model,
  MessageRecord,
  Session,
  TodoItem,
} from "../../types/api";
import type { StreamMessage } from "../../types/chat";
import { ChatComposer } from "./ChatComposer";
import { ApprovalCard, MessageList } from "./MessageList";
import { SessionSidebar } from "./SessionSidebar";

export interface ChatPanelProps {
  loading: boolean;
  sessions: Session[];
  selectedSessionId: string;
  selectedSession: Session | undefined;
  activeModel: Model | undefined;
  messages: MessageRecord[];
  streams: StreamMessage[];
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
  onSessionSearchChange: (value: string) => void;
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
}

export function ChatPanel({
  loading,
  sessions,
  selectedSessionId,
  selectedSession,
  activeModel,
  messages,
  streams,
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
  onSessionSearchChange,
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
}: ChatPanelProps) {
  const { t } = useTranslation();
  const pendingRequests = requests.filter((request) => request.session_id === selectedSessionId);

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
      <button
        className={
          mobileSidebarOpen ? "mobile-sidebar-backdrop visible" : "mobile-sidebar-backdrop"
        }
        onClick={onMobileSidebarClose}
        aria-label={t("Close conversations")}
      />
      <SessionSidebar
        loading={loading}
        mobileOpen={mobileSidebarOpen}
        sessions={sessions}
        selectedSessionId={selectedSessionId}
        search={sessionSearch}
        renamingSessionId={renamingSessionId}
        renameValue={renameValue}
        onSearchChange={onSessionSearchChange}
        onCreate={handleCreateSession}
        onSelect={handleSelectSession}
        onStartRename={onStartRename}
        onRenameChange={onRenameChange}
        onRename={onRename}
        onCancelRename={onCancelRename}
        onDelete={onDeleteSession}
      />
      <section className="chat-panel">
        <div className="message-stage">
          <MessageList
            messages={messages}
            streams={streams}
            workspaceRoot={selectedSession?.workspace_root}
            onRevert={onRevert}
            onFork={onFork}
            onRetryProviderError={onRetryProviderError}
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
          models={models}
          activeModel={activeModel}
          thinkingLevel={thinkingLevel}
          todos={todos}
          enterToSend={enterToSend}
          isBusy={selectedSession?.busy ?? false}
          sending={sending}
          canceling={canceling}
          selectedSessionId={selectedSessionId}
          fileMention={fileMention}
          fileMentionIndex={fileMentionIndex}
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
        />
      </section>
    </>
  );
}
