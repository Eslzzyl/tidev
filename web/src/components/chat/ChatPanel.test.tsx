// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ChatPanel, type ChatPanelProps } from "./ChatPanel";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("./MessageList", async () => {
  const React = await import("react");

  function MessageListProbe({ sessionId }: { sessionId: string }) {
    const [initialSessionId] = React.useState(sessionId);
    return React.createElement("div", {
      "data-testid": "message-list",
      "data-initial-session": initialSessionId,
    });
  }

  return { ApprovalCard: () => null, MessageList: MessageListProbe };
});

vi.mock("./ChatComposer", () => ({ ChatComposer: () => null }));
vi.mock("./ChangedFilesPanel", () => ({ ChangedFilesPanel: () => null }));
vi.mock("./SessionSidebar", () => ({ SessionSidebar: () => null }));
vi.mock("../ui", () => ({ Button: () => null }));

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: false });
});

function panelProps(sessionId: string): ChatPanelProps {
  const noop = vi.fn();

  return {
    loading: false,
    loadingMoreSessions: false,
    hasMoreSessions: false,
    sessions: [],
    workspaceRoots: [],
    workspaceRootFilter: null,
    selectedSessionId: sessionId,
    selectedSession: undefined,
    sessionStatus: "ready",
    activeModel: undefined,
    messages: [],
    changedFiles: [],
    changedFileDiffs: [],
    changedFilesPanelOpen: false,
    changedFilesLoading: false,
    changedFilesError: null,
    streams: [],
    instructionNotices: [],
    compactionNotice: null,
    requests: [],
    todos: [],
    error: null,
    sessionSearch: "",
    renamingSessionId: null,
    renameValue: "",
    draft: "",
    mode: "build",
    models: [],
    thinkingLevel: undefined,
    fastMode: false,
    onToggleFastMode: noop,
    enterToSend: false,
    sending: false,
    canceling: false,
    mobileSidebarOpen: false,
    fileMention: null,
    fileMentionIndex: 0,
    pendingImages: [],
    welcome: null,
    onSessionSearchChange: noop,
    onWorkspaceRootFilterChange: noop,
    onLoadMoreSessions: noop,
    onCreateSession: noop,
    onSelectSession: noop,
    onStartRename: noop,
    onRenameChange: noop,
    onRename: noop,
    onCancelRename: noop,
    onDeleteSession: noop,
    onRevert: noop,
    onFork: noop,
    onRetryProviderError: noop,
    onOpenChangedFiles: noop,
    onCloseChangedFiles: noop,
    onRespond: noop,
    onMobileSidebarClose: noop,
    onDraftChange: noop,
    onModeChange: noop,
    onSelectModel: noop,
    onSelectThinkingLevel: noop,
    onSubmit: noop,
    onCancel: noop,
    onFileMentionChange: noop,
    onFileMentionIndexChange: noop,
    onFileSelect: () => undefined,
    onFileMentionClose: noop,
    onImagesPasted: noop,
    onRemoveImage: noop,
  };
}

describe("ChatPanel session switching", () => {
  it("remounts the message list for each selected session", () => {
    act(() => {
      root.render(createElement(ChatPanel, panelProps("session-a")));
    });

    expect(
      container.querySelector("[data-testid='message-list']")?.getAttribute("data-initial-session"),
    ).toBe("session-a");

    act(() => {
      root.render(createElement(ChatPanel, panelProps("session-b")));
    });

    expect(
      container.querySelector("[data-testid='message-list']")?.getAttribute("data-initial-session"),
    ).toBe("session-b");
  });
});
