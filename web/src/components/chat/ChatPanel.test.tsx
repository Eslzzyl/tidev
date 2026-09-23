// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ChatPanel, type ChatPanelProps } from "./ChatPanel";
import { useUIStore } from "../../stores/useUIStore";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
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
  useUIStore.getState().setLeftSidebarWidth(256);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  useUIStore.getState().setLeftSidebarWidth(256);
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

  it("resizes the conversations sidebar with arrow keys", () => {
    act(() => {
      root.render(createElement(ChatPanel, panelProps("session-a")));
    });

    const separator = container.querySelector<HTMLElement>('[role="separator"]');
    expect(separator?.getAttribute("aria-valuenow")).toBe("256");

    act(() => {
      separator?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    });

    expect(useUIStore.getState().leftSidebarWidth).toBe(246);
    expect(separator?.getAttribute("aria-valuenow")).toBe("246");
  });

  it("updates the sidebar width after a pointer drag", () => {
    act(() => {
      root.render(createElement(ChatPanel, panelProps("session-a")));
    });

    const separator = container.querySelector<HTMLElement>('[role="separator"]');
    expect(separator).not.toBeNull();
    if (!separator) return;

    Object.defineProperty(separator, "setPointerCapture", { value: vi.fn() });
    const dispatchPointer = (type: string, clientX: number) => {
      const event = new Event(type, { bubbles: true });
      Object.assign(event, { pointerId: 1, clientX, button: 0 });
      separator.dispatchEvent(event);
    };

    act(() => {
      dispatchPointer("pointerdown", 100);
      dispatchPointer("pointermove", 145);
      expect(separator.getAttribute("aria-valuenow")).toBe("301");
      dispatchPointer("pointerup", 145);
    });

    expect(useUIStore.getState().leftSidebarWidth).toBe(301);
  });
});
