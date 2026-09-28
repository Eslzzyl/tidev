// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { MessageRecord } from "../../types/api";
import { MessageList } from "./MessageList";

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: () => ({
    getTotalSize: () => 100,
    getVirtualItems: () => [{ index: 0, key: "user-message-1", start: 0 }],
    measureElement: vi.fn(),
  }),
}));

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({ t: (key: string) => key }),
}));

const userMessage: MessageRecord = {
  message: {
    id: "user-message-1",
    role: "user",
    content: "Undo this turn",
    attachments: [],
    reasoning: "",
    tool_calls: [],
    tool_call_id: null,
    tool_name: null,
    metadata: {
      filepath: null,
      diff: null,
      truncated: null,
      exists: null,
      prior_summary: null,
      prior_retained_from: null,
      file_changes: [],
      exit_code: null,
      duration_ms: null,
    },
    created_at: "2026-09-28T00:00:00.000Z",
    completed_at: null,
    streaming: false,
    reasoning_started_at: null,
    reasoning_completed_at: null,
    input_tokens: null,
    output_tokens: null,
    total_tokens: null,
    cache_read_tokens: null,
    cache_write_tokens: null,
    model_id: null,
    tokens_per_second: null,
    thinking_level: null,
  },
  app_data: {},
};

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
  vi.restoreAllMocks();
  container.remove();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: false });
});

describe("MessageList revert action", () => {
  it("calls the revert handler and exposes pending state on the clicked message", async () => {
    const onRevert = vi.fn();
    const props = {
      messages: [userMessage],
      streams: [],
      onRevert,
      sessionId: "session-1",
    };

    await act(async () => {
      root.render(createElement(MessageList, props));
    });

    const action = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Revert to this message (undo later messages)"]',
    );
    expect(action).not.toBeNull();

    act(() => {
      action?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onRevert).toHaveBeenCalledWith("session-1", userMessage.message.id);

    await act(async () => {
      root.render(
        createElement(MessageList, { ...props, revertingMessageId: userMessage.message.id }),
      );
    });

    const pendingAction = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Revert to this message (undo later messages)"]',
    );
    expect(pendingAction?.disabled).toBe(true);
    expect(pendingAction?.getAttribute("aria-busy")).toBe("true");
    expect(pendingAction?.querySelector("svg")?.getAttribute("class")).toContain("spin");
  });
});
