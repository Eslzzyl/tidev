import type { ReactNode } from "react";
import { lazy, Suspense, useLayoutEffect, useRef, useState } from "react";
import {
  BarChart3,
  Folder,
  FolderTree,
  GitBranch,
  Menu,
  MessageSquare,
  Settings,
  Terminal,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { useLocation, useRoute, Switch, Route, Redirect } from "wouter";
import { AuthGate } from "./components/AuthGate";
import { ChatPanel } from "./components/chat/ChatPanel";
import { WelcomePage } from "./components/chat/WelcomePage";
import { SettingsPanel } from "./components/settings/SettingsPanel";
import { ComponentShowcase } from "./components/ui/ComponentShowcase";
import { ConfirmDialog } from "./components/ui/ConfirmDialog";
import { useChatRuntime } from "./hooks/useChatRuntime";
import { useWorkspace } from "./hooks/workspaceQueries";
import { getActiveFeature, routes } from "./lib/routes";
import type { Session } from "./types/api";
import type { Feature } from "./types/chat";
import { shortPath } from "./utils/chat";

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
  const [location, navigate] = useLocation();
  const [isChatRoute, chatParams] = useRoute<{ sessionId?: string }>("/chat/:sessionId?");
  const routeSessionId = isChatRoute ? (chatParams?.sessionId ?? null) : undefined;

  const feature: Feature = getActiveFeature(location);
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false);
  const [sessionToDelete, setSessionToDelete] = useState<Session | null>(null);
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

  const onSelectSessionRoute = useRef((sessionId: string | null) => {
    navigate(routes.chat(sessionId));
  }).current;

  const {
    authChecking,
    authRequired,
    authenticated,
    openSettingsPanel,
    enterToSend,
    sessions,
    sessionWorkspaceRoots,
    sessionWorkspaceRoot,
    nextSessionCursor,
    loadingMoreSessions,
    selectedSessionId,
    selectedSession,
    sessionStatus,
    activeModel,
    messages,
    visibleStreams,
    instructionNotices,
    requests,
    models,
    todos,
    draft,
    pendingImages,
    mode,
    loading,
    sending,
    canceling,
    welcomeSending,
    focusComposerAfterWelcome,
    clearComposerFocusRequest,
    scrollToBottomRequest,
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
    setSessionWorkspaceRoot,
    setRenamingSessionId,
    setRenameValue,
    setFileMentionIndex,
    setFileMention,
    selectSession,
    createSession,
    loadMoreSessions,
    renameSession,
    deleteSession,
    submitWelcome,
    handleRevert,
    handleRetryProviderError,
    handleFork,
    submit,
    chooseModel,
    chooseThinkingLevel,
    respondToRequest,
    cancelSession,
    updateFileMention,
    handleFileSelect,
    handleImagesPasted,
    removePendingImage,
    completedSessions,
  } = useChatRuntime({
    routeSessionId,
    onSelectSessionRoute,
  });
  if (authChecking) {
    return <AuthLoading />;
  }

  if (authRequired && !authenticated) return <AuthGate />;

  const handleFeatureNav = (id: Feature) => {
    setMobileSidebarOpen(false);
    if (id === "chat") {
      navigate(routes.chat(selectedSessionId));
    } else if (id === "files") {
      navigate(routes.files());
    } else if (id === "terminal") {
      navigate(routes.terminal());
    } else if (id === "git") {
      navigate(routes.git("changes"));
    } else if (id === "stats") {
      navigate(routes.stats());
    }
  };

  const renderChat = () => (
    <ChatPanel
      completedSessions={completedSessions}
      loading={loading}
      loadingMoreSessions={loadingMoreSessions}
      hasMoreSessions={nextSessionCursor !== null}
      sessions={sessions}
      workspaceRoots={sessionWorkspaceRoots}
      workspaceRootFilter={sessionWorkspaceRoot}
      selectedSessionId={selectedSessionId}
      selectedSession={selectedSession}
      sessionStatus={sessionStatus}
      activeModel={activeModel}
      messages={messages}
      streams={visibleStreams}
      instructionNotices={instructionNotices}
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
      mobileSidebarOpen={mobileSidebarOpen}
      fileMention={fileMention}
      fileMentionIndex={fileMentionIndex}
      welcome={
        <WelcomePage
          draft={draft}
          error={error}
          loading={loading}
          mode={mode}
          enterToSend={enterToSend}
          sending={welcomeSending}
          models={models}
          activeModel={activeModel}
          thinkingLevel={thinkingLevel}
          fileMention={fileMention}
          fileMentionIndex={fileMentionIndex}
          recentWorkspaceRoots={sessionWorkspaceRoots}
          onChangeDraft={setDraft}
          onModeChange={setMode}
          onSelectModel={(model) => void chooseModel(model)}
          onSelectThinkingLevel={(level) => void chooseThinkingLevel(level)}
          onSubmit={(workspaceRoot) => void submitWelcome(workspaceRoot)}
          onFileMentionChange={updateFileMention}
          onFileMentionIndexChange={setFileMentionIndex}
          onFileSelect={handleFileSelect}
          onFileMentionClose={() => setFileMention(null)}
          pendingImages={pendingImages}
          onImagesPasted={handleImagesPasted}
          onRemoveImage={removePendingImage}
        />
      }
      onSessionSearchChange={setSessionSearch}
      onWorkspaceRootFilterChange={setSessionWorkspaceRoot}
      onLoadMoreSessions={() => void loadMoreSessions()}
      onCreateSession={createSession}
      onSelectSession={selectSession}
      onStartRename={(session) => {
        setRenamingSessionId(session.session_id);
        setRenameValue(session.title);
      }}
      onRenameChange={setRenameValue}
      onRename={(sessionId) => void renameSession(sessionId)}
      onCancelRename={() => setRenamingSessionId(null)}
      onDeleteSession={(session) => setSessionToDelete(session)}
      onRevert={handleRevert}
      onRetryProviderError={handleRetryProviderError}
      onFork={handleFork}
      onRespond={(requestId, tools) => void respondToRequest(requestId, tools)}
      onMobileSidebarClose={() => setMobileSidebarOpen(false)}
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
      pendingImages={pendingImages}
      onImagesPasted={handleImagesPasted}
      onRemoveImage={removePendingImage}
      focusComposer={focusComposerAfterWelcome}
      onComposerFocus={clearComposerFocusRequest}
      scrollToBottomRequest={scrollToBottomRequest}
    />
  );

  return (
    <div className="app-shell">
      <header className="topbar">
        {feature === "chat" ? (
          <button
            className="mobile-chat-sidebar-button"
            onClick={() => setMobileSidebarOpen(true)}
            aria-label={t("Open conversations")}
            title={t("Open conversations")}
          >
            <Menu size={17} strokeWidth={1.8} />
          </button>
        ) : null}
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
              onClick={() => handleFeatureNav(id)}
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
              transform: `${featureIndicator.left ? `translateX(${featureIndicator.left}px)` : "none"}`,
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
          <Switch>
            <Route path="/__components">
              <ComponentShowcase />
            </Route>
            <Route path="/chat/:sessionId?">{renderChat()}</Route>
            <Route path="/files">
              <FeatureWorkspaceContext session={selectedSession}>
                <FilesView />
              </FeatureWorkspaceContext>
            </Route>
            <Route path="/terminal/:tabId?">
              <FeatureWorkspaceContext session={selectedSession}>
                <TerminalView />
              </FeatureWorkspaceContext>
            </Route>
            <Route path="/git/:tab?/:sha?">
              <FeatureWorkspaceContext session={selectedSession}>
                <GitView />
              </FeatureWorkspaceContext>
            </Route>
            <Route path="/stats">
              <StatsView />
            </Route>
            <Route path="/settings/:category?">{renderChat()}</Route>
            <Route path="/">
              <Redirect to="/chat" />
            </Route>
            <Route>
              <Redirect to="/chat" />
            </Route>
          </Switch>
        </Suspense>
      </main>
      <SettingsPanel />
      {sessionToDelete ? (
        <ConfirmDialog
          danger
          title={t("Delete conversation")}
          message={t("Delete conversation “{{title}}”?", {
            title: sessionToDelete.title || t("Untitled conversation"),
          })}
          confirmText={t("Delete")}
          cancelText={t("Cancel")}
          onConfirm={() => {
            const session = sessionToDelete;
            setSessionToDelete(null);
            void deleteSession(session);
          }}
          onCancel={() => setSessionToDelete(null)}
        />
      ) : null}
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

function FeatureWorkspaceContext({
  session,
  children,
}: {
  session: Session | undefined;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const { data: workspaceInfo } = useWorkspace();
  const workspaceRoot = workspaceInfo?.workspace_root;

  if (!session || !workspaceRoot || workspaceRoot === session.workspace_root) return children;

  return (
    <div className="feature-workspace-context">
      <div className="feature-workspace-notice">
        <span title={workspaceRoot}>
          <Folder size={13} aria-hidden="true" />
          {t("Current tool directory")}: {shortPath(workspaceRoot)}
        </span>
        <span title={session.workspace_root}>
          {t("Session directory")}: {shortPath(session.workspace_root)}
        </span>
      </div>
      {children}
    </div>
  );
}
