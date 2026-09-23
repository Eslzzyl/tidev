// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api } from "../api/client";
import { useAuthStore } from "../stores/useAuthStore";
import type { Session } from "../types/api";
import { useChatRuntime } from "./useChatRuntime";

vi.mock("../api/events", () => ({
  openBackendEvents: () => ({ close: vi.fn() }),
  openFrontendRequests: () => ({ close: vi.fn() }),
}));

const createdSession: Session = {
  session_id: "created-session",
  parent_session_id: null,
  workspace_root: "/new-workspace",
  provider_id: "provider",
  provider_display_name: "Provider",
  model_id: "model",
  model_display_name: "Model",
  title: "First message",
  created_at: "2026-09-23T00:00:00Z",
  updated_at: "2026-09-23T00:00:00Z",
  status: "idle",
  ended_at: null,
  context_summary: null,
  context_retained_from: 0,
  busy: false,
};

let container: HTMLDivElement;
let root: Root;
let runtime: ReturnType<typeof useChatRuntime>;

function RuntimeProbe() {
  runtime = useChatRuntime();
  return null;
}

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);

  useAuthStore.setState({
    isLoading: false,
    isAuthRequired: false,
    isAuthenticated: true,
    checkAuthStatus: vi.fn().mockResolvedValue(undefined),
  });

  vi.spyOn(api, "getFastMode").mockResolvedValue({ fast_mode: false });
  vi.spyOn(api, "getStartupStatus").mockResolvedValue({ model_fallback: null });
  vi.spyOn(api, "listModels").mockResolvedValue([]);
  vi.spyOn(api, "listSessions").mockResolvedValue({
    items: [],
    next_cursor: null,
    workspace_roots: ["/existing-workspace"],
  });
  vi.spyOn(api, "createSession").mockResolvedValue({ session: createdSession });
  vi.spyOn(api, "sendPrompt").mockResolvedValue({ message_id: "message", duplicate: false });
  vi.spyOn(api, "listMessages").mockResolvedValue({ messages: [] });
  vi.spyOn(api, "getTodos").mockResolvedValue({ todos: [] });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: false });
});

describe("useChatRuntime welcome submission", () => {
  it("preserves sidebar filters when the first message creates a session", async () => {
    await act(async () => {
      root.render(createElement(RuntimeProbe));
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });

    act(() => {
      runtime.setSessionSearch("important conversation");
      runtime.setSessionWorkspaceRoot("/existing-workspace");
      runtime.setDraft("First message");
    });

    await act(async () => {
      await runtime.submitWelcome("/new-workspace");
    });

    expect(runtime.sessionSearch).toBe("important conversation");
    expect(runtime.sessionWorkspaceRoot).toBe("/existing-workspace");
  });
});
