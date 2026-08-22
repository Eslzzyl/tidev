import { Sparkles } from "lucide-react";
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
  onRespond: (requestId: string, tools: ApprovedTool[]) => void;
  onRedo: () => void;
  onCompact: () => void;
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
  onRespond,
  onRedo,
  onCompact,
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

  return (
    <>
      <SessionSidebar
        loading={loading}
        sessions={sessions}
        selectedSessionId={selectedSessionId}
        search={sessionSearch}
        renamingSessionId={renamingSessionId}
        renameValue={renameValue}
        onSearchChange={onSessionSearchChange}
        onCreate={onCreateSession}
        onSelect={onSelectSession}
        onStartRename={onStartRename}
        onRenameChange={onRenameChange}
        onRename={onRename}
        onCancelRename={onCancelRename}
        onDelete={onDeleteSession}
      />
      <section className="chat-panel">
        <div className="panel-header">
          <div>
            <span className="eyebrow">{t("Conversation")}</span>
            <h1>{selectedSession?.title ?? t("New conversation")}</h1>
          </div>
          <div className="panel-actions">
            <button className="ghost-button" onClick={onRedo} title={t("Redo")}>
              Redo
            </button>
            <button className="ghost-button" onClick={onCompact} title={t("Compact context")}>
              Compact
            </button>
            <span className="model-label">
              <Sparkles size={15} />
              {selectedSession?.model_display_name ?? "Runtime model"}
            </span>
          </div>
        </div>
        <div className="message-stage">
          <MessageList messages={messages} streams={streams} onRevert={onRevert} onFork={onFork} />
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
