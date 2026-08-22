import { lazy, Suspense, useLayoutEffect, useRef, useState } from "react";
import { BarChart3, FolderTree, GitBranch, MessageSquare, Settings, Terminal } from "lucide-react";
import { useTranslation } from "react-i18next";

import { AuthGate } from "./components/AuthGate";
import { ChatPanel } from "./components/chat/ChatPanel";
import { WelcomePage } from "./components/chat/WelcomePage";
import { SettingsPanel } from "./components/settings/SettingsPanel";
import { useChatRuntime } from "./hooks/useChatRuntime";
import type { Feature } from "./types/chat";

const FilesView = lazy(() =>
  import("./components/views/FilesView").then(({ FilesView: view }) => ({ default: view })),
);
const GitView = lazy(() =>
  import("./components/views/GitView").then(({ GitView: view }) => ({ default: view })),
);
const StatsView = lazy(() =>
  import("./components/views/StatsView").then(({ StatsView: view }) => ({ default: view })),
);
const TerminalView = lazy(() =>
  import("./components/views/TerminalView").then(({ TerminalView: view }) => ({ default: view })),
);

const features: { id: Feature; label: string; icon: typeof MessageSquare }[] = [
  { id: "chat", label: "Chat", icon: MessageSquare },
  { id: "files", label: "Files", icon: FolderTree },
  { id: "terminal", label: "Terminal", icon: Terminal },
  { id: "git", label: "Git", icon: GitBranch },
  { id: "stats", label: "Stats", icon: BarChart3 },
];

export default function App() {
  const { t } = useTranslation();
  const [feature, setFeature] = useState<Feature>("chat");
  const featureNavRef = useRef<HTMLElement>(null);
  const featureButtonRefs = useRef<Partial<Record<Feature, HTMLButtonElement | null>>>({});
  const [featureIndicator, setFeatureIndicator] = useState({
    left: 0,
    width: 0,
    visible: false,
  });

  useLayoutEffect(() => {
    const updateIndicator = () => {
      const nav = featureNavRef.current;
      const button = featureButtonRefs.current[feature];
      if (!nav || !button) return;

      const navRect = nav.getBoundingClientRect();
      const buttonRect = button.getBoundingClientRect();
      setFeatureIndicator({
        left: buttonRect.left - navRect.left,
        width: buttonRect.width,
        visible: true,
      });
    };

    updateIndicator();
    const observer = new ResizeObserver(updateIndicator);
    if (featureNavRef.current) observer.observe(featureNavRef.current);
    return () => observer.disconnect();
  }, [feature]);

  const {
    authChecking,
    authRequired,
    authenticated,
    openSettingsPanel,
    enterToSend,
    sessions,
    selectedSessionId,
    selectedSession,
    activeModel,
    messages,
    visibleStreams,
    requests,
    models,
    todos,
    draft,
    mode,
    loading,
    sending,
    canceling,
    welcomeSending,
    thinkingLevel,
    sessionSearch,
    renamingSessionId,
    renameValue,
    error,
    fileMention,
    fileMentionIndex,
    setDraft,
    setMode,
    setSessionSearch,
    setRenamingSessionId,
    setRenameValue,
    setFileMentionIndex,
    setFileMention,
    selectSession,
    createSession,
    renameSession,
    deleteSession,
    submitWelcome,
    handleRevert,
    handleRedo,
    handleFork,
    handleCompact,
    submit,
    chooseModel,
    chooseThinkingLevel,
    respondToRequest,
    cancelSession,
    updateFileMention,
    handleFileSelect,
  } = useChatRuntime();
  if (authChecking) {
    return <AuthLoading />;
  }

  if (authRequired && !authenticated) return <AuthGate />;

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand-mark">
          <span className="brand-glyph">t</span>
          <span>tidev</span>
        </div>
        <nav ref={featureNavRef} className="feature-nav" aria-label={t("Primary navigation")}>
          {features.map(({ id, label, icon: Icon }) => (
            <button
              className={feature === id ? "feature-link active" : "feature-link"}
              key={id}
              ref={(button) => {
                featureButtonRefs.current[id] = button;
              }}
              onClick={() => setFeature(id)}
            >
              <Icon size={16} strokeWidth={1.8} />
              {t(label)}
            </button>
          ))}
          <span
            className="feature-nav-indicator"
            aria-hidden="true"
            style={{
              width: `${featureIndicator.width}px`,
              transform: `translateX(${featureIndicator.left}px)`,
              opacity: featureIndicator.visible ? 1 : 0,
            }}
          />
        </nav>
        <button
          className="settings-button"
          onClick={openSettingsPanel}
          aria-label={t("Settings")}
          title={t("Settings")}
        >
          <Settings size={16} />
        </button>
      </header>

      <main className="workspace">
        <Suspense fallback={<FeatureLoading />}>
          {feature === "chat" ? (
            selectedSessionId === null ? (
              <WelcomePage
                draft={draft}
                error={error}
                loading={loading}
                mode={mode}
                enterToSend={enterToSend}
                sending={welcomeSending}
                sessions={sessions}
                models={models}
                activeModel={activeModel}
                thinkingLevel={thinkingLevel}
                fileMention={fileMention}
                fileMentionIndex={fileMentionIndex}
                onChangeDraft={setDraft}
                onModeChange={setMode}
                onSelectSession={selectSession}
                onSelectModel={(model) => void chooseModel(model)}
                onSelectThinkingLevel={(level) => void chooseThinkingLevel(level)}
                onSubmit={() => void submitWelcome()}
                onFileMentionChange={updateFileMention}
                onFileMentionIndexChange={setFileMentionIndex}
                onFileSelect={handleFileSelect}
                onFileMentionClose={() => setFileMention(null)}
              />
            ) : (
              <>
                <ChatPanel
                  loading={loading}
                  sessions={sessions}
                  selectedSessionId={selectedSessionId}
                  selectedSession={selectedSession}
                  activeModel={activeModel}
                  messages={messages}
                  streams={visibleStreams}
                  requests={requests}
                  todos={todos}
                  error={error}
                  sessionSearch={sessionSearch}
                  renamingSessionId={renamingSessionId}
                  renameValue={renameValue}
                  draft={draft}
                  mode={mode}
                  models={models}
                  thinkingLevel={thinkingLevel}
                  enterToSend={enterToSend}
                  sending={sending}
                  canceling={canceling}
                  fileMention={fileMention}
                  fileMentionIndex={fileMentionIndex}
                  onSessionSearchChange={setSessionSearch}
                  onCreateSession={createSession}
                  onSelectSession={selectSession}
                  onStartRename={(session) => {
                    setRenamingSessionId(session.session_id);
                    setRenameValue(session.title);
                  }}
                  onRenameChange={setRenameValue}
                  onRename={(sessionId) => void renameSession(sessionId)}
                  onCancelRename={() => setRenamingSessionId(null)}
                  onDeleteSession={(session) => void deleteSession(session)}
                  onRevert={(messageId) => void handleRevert(messageId)}
                  onFork={(messageId) => void handleFork(messageId)}
                  onRespond={(requestId, tools) => void respondToRequest(requestId, tools)}
                  onRedo={() => void handleRedo()}
                  onCompact={() => void handleCompact()}
                  onDraftChange={setDraft}
                  onModeChange={setMode}
                  onSelectModel={(model) => void chooseModel(model)}
                  onSelectThinkingLevel={(level) => void chooseThinkingLevel(level)}
                  onSubmit={() => void submit()}
                  onCancel={() => {
                    if (selectedSessionId) void cancelSession(selectedSessionId);
                  }}
                  onFileMentionChange={updateFileMention}
                  onFileMentionIndexChange={setFileMentionIndex}
                  onFileSelect={handleFileSelect}
                  onFileMentionClose={() => setFileMention(null)}
                />
              </>
            )
          ) : feature === "files" ? (
            <FilesView />
          ) : feature === "terminal" ? (
            <TerminalView />
          ) : feature === "git" ? (
            <GitView />
          ) : feature === "stats" ? (
            <StatsView />
          ) : null}
        </Suspense>
      </main>
      <SettingsPanel />
    </div>
  );
}

function AuthLoading() {
  const { t } = useTranslation();
  return (
    <main className="auth-page">
      <div className="auth-card">
        <div className="welcome-logo">t</div>
        <h1>tidev</h1>
        <p>{t("Connecting to the local runtime…")}</p>
      </div>
    </main>
  );
}

function FeatureLoading() {
  const { t } = useTranslation();
  return (
    <div className="feature-loading" role="status" aria-live="polite">
      <span className="feature-loading-spinner" aria-hidden="true" />
      <span className="sr-only">{t("Loading feature")}</span>
    </div>
  );
}
