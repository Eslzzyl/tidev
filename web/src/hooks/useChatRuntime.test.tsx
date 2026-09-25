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
  vi.spyOn(api, "listProviders").mockResolvedValue({ providers: [] });
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

    expect(runtime.startupProviderStatus).toBe("needs-setup");

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

  it("updates activeModel and thinkingLevel when selecting a session", async () => {
    const modelGpt6 = {
      provider_id: "cpa",
      provider_display_name: "CLIProxyAPI",
      model_id: "gpt-6-luna",
      model_display_name: "GPT-6 Luna",
      context_window: 272000,
      connected: true,
      active: true,
      supports_vision: true,
      is_gpt: true,
      thinking_levels: ["gpt5:low", "gpt5:medium", "gpt5:high", "gpt5:xhigh", "gpt5:max"],
      thinking_level: "gpt5:xhigh",
    };
    const modelGpt5 = {
      provider_id: "cpa",
      provider_display_name: "CLIProxyAPI",
      model_id: "gpt-5-6-luna",
      model_display_name: "GPT-5.6 Luna",
      context_window: 272000,
      connected: true,
      active: false,
      supports_vision: true,
      is_gpt: true,
      thinking_levels: ["gpt5:off", "gpt5:low", "gpt5:medium", "gpt5:high", "gpt5:xhigh", "gpt5:max"],
      thinking_level: "gpt5:medium",
    };

    const sessionGpt6: Session = {
      session_id: "session-6",
      parent_session_id: null,
      workspace_root: "/test",
      provider_id: "cpa",
      provider_display_name: "CLIProxyAPI",
      model_id: "gpt-6-luna",
      model_display_name: "GPT-6 Luna",
      title: "Session 6",
      created_at: "2026-09-24T00:00:00Z",
      updated_at: "2026-09-24T00:00:00Z",
      status: "idle",
      ended_at: null,
      context_summary: null,
      context_retained_from: 0,
      busy: false,
      thinking_level: "gpt5:xhigh",
    };
    const sessionGpt5: Session = {
      session_id: "session-5",
      parent_session_id: null,
      workspace_root: "/test",
      provider_id: "cpa",
      provider_display_name: "CLIProxyAPI",
      model_id: "gpt-5-6-luna",
      model_display_name: "GPT-5.6 Luna",
      title: "Session 5",
      created_at: "2026-09-25T00:00:00Z",
      updated_at: "2026-09-25T00:00:00Z",
      status: "idle",
      ended_at: null,
      context_summary: null,
      context_retained_from: 0,
      busy: false,
      thinking_level: "gpt5:medium",
    };

    vi.spyOn(api, "listModels").mockResolvedValue([modelGpt6, modelGpt5]);
    vi.spyOn(api, "listSessions").mockResolvedValue({
      items: [sessionGpt6, sessionGpt5],
      next_cursor: null,
      workspace_roots: ["/test"],
    });

    await act(async () => {
      root.render(createElement(RuntimeProbe));
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });

    expect(runtime.models.length).toBe(2);
    expect(runtime.thinkingLevel).toBe("gpt5:xhigh");

    await act(async () => {
      runtime.selectSession("session-5");
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });

    expect(runtime.selectedSessionId).toBe("session-5");
    expect(runtime.selectedSession?.model_id).toBe("gpt-5-6-luna");
    expect(runtime.activeModel?.model_id).toBe("gpt-5-6-luna");
    expect(runtime.thinkingLevel).toBe("gpt5:medium");
  });

  it("updates activeModel and thinkingLevel when mounted with routeSessionId", async () => {
    const modelGpt6 = {
      provider_id: "cpa",
      provider_display_name: "CLIProxyAPI",
      model_id: "gpt-6-luna",
      model_display_name: "GPT-6 Luna",
      context_window: 272000,
      connected: true,
      active: true,
      supports_vision: true,
      is_gpt: true,
      thinking_levels: ["gpt5:low", "gpt5:medium", "gpt5:high", "gpt5:xhigh", "gpt5:max"],
      thinking_level: "gpt5:xhigh",
    };
    const modelGpt5 = {
      provider_id: "cpa",
      provider_display_name: "CLIProxyAPI",
      model_id: "gpt-5-6-luna",
      model_display_name: "GPT-5.6 Luna",
      context_window: 272000,
      connected: true,
      active: false,
      supports_vision: true,
      is_gpt: true,
      thinking_levels: ["gpt5:off", "gpt5:low", "gpt5:medium", "gpt5:high", "gpt5:xhigh", "gpt5:max"],
      thinking_level: "gpt5:medium",
    };

    const sessionGpt5: Session = {
      session_id: "session-5",
      parent_session_id: null,
      workspace_root: "/test",
      provider_id: "cpa",
      provider_display_name: "CLIProxyAPI",
      model_id: "gpt-5-6-luna",
      model_display_name: "GPT-5.6 Luna",
      title: "Session 5",
      created_at: "2026-09-25T00:00:00Z",
      updated_at: "2026-09-25T00:00:00Z",
      status: "idle",
      ended_at: null,
      context_summary: null,
      context_retained_from: 0,
      busy: false,
      thinking_level: "gpt5:medium",
    };

    vi.spyOn(api, "listModels").mockResolvedValue([modelGpt6, modelGpt5]);
    vi.spyOn(api, "getSession").mockResolvedValue(sessionGpt5);
    vi.spyOn(api, "listSessions").mockResolvedValue({
      items: [sessionGpt5],
      next_cursor: null,
      workspace_roots: ["/test"],
    });

    function RoutedProbe() {
      runtime = useChatRuntime({ routeSessionId: "session-5" });
      return null;
    }

    await act(async () => {
      root.render(createElement(RoutedProbe));
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });

    expect(runtime.selectedSessionId).toBe("session-5");
    expect(runtime.selectedSession?.model_id).toBe("gpt-5-6-luna");
    expect(runtime.activeModel?.model_id).toBe("gpt-5-6-luna");
    expect(runtime.thinkingLevel).toBe("gpt5:medium");
  });
});
