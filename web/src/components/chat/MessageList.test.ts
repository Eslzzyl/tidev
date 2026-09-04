import { describe, expect, it } from "vitest";
import type { TFunction } from "i18next";

import { buildChatItems } from "./MessageList";
import type { StreamMessage } from "../../types/chat";
import type { Round } from "../../utils/round";

function round(overrides: Partial<Round> = {}): Round {
  return {
    id: "round-user-1",
    userMessage: {
      id: "user-1",
      role: "user",
      content: "Find the image",
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
      created_at: "2026-09-04T00:00:00.000Z",
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
    leadingInstructions: [],
    segments: [],
    toolCallMap: {},
    providerErrors: [],
    status: "user_only",
    interrupted: false,
    ...overrides,
  };
}

function interruptedStream(): StreamMessage {
  return {
    key: "session-1:3",
    requestId: 3,
    segments: [
      {
        type: "reasoning",
        content: "I will search the workspace.",
        startedAt: "2026-09-04T00:00:00.000Z",
        completedAt: "2026-09-04T00:00:57.500Z",
      },
      { type: "text", content: "The image was not found." },
    ],
    toolCallMap: {},
    status: "interrupted",
    providerFinished: false,
    reasoningStartedAt: "2026-09-04T00:00:00.000Z",
    reasoningCompletedAt: "2026-09-04T00:00:57.500Z",
    error: "The turn was interrupted.",
    userMessageId: "user-1",
  };
}

const translate = ((key: string) => key) as TFunction;

describe("interrupted stream rendering", () => {
  it("keeps an interrupted stream in the originating user turn", () => {
    const items = buildChatItems(
      [round()],
      [interruptedStream()],
      {},
      [],
      "",
      [],
      undefined,
      translate,
    );

    expect(items.map((item) => item.kind)).toEqual([
      "user",
      "assistant-meta",
      "assistant-segment",
      "assistant-segment",
      "interruption-notice",
    ]);
    expect(items.some((item) => item.kind === "stream-error")).toBe(false);
    expect(items.find((item) => item.kind === "assistant-meta")).toMatchObject({
      status: "interrupted",
      collapsible: true,
      expanded: true,
    });
    expect(
      items.find(
        (item) => item.kind === "assistant-segment" && item.item.segment.type === "reasoning",
      ),
    ).toMatchObject({ item: { collapsed: false } });
  });

  it("does not reuse item keys when a persisted interrupted round has a stream", () => {
    const items = buildChatItems(
      [
        round({
          segments: [{ type: "text", content: "Persisted tool call response." }],
          status: "complete",
          interrupted: true,
          interruptionKind: "cancelled",
          completedAt: "2026-09-04T00:00:30.000Z",
        }),
      ],
      [interruptedStream()],
      {},
      [],
      "",
      [],
      undefined,
      translate,
    );
    const keys = items.map((item) =>
      item.kind === "assistant-segment" ? item.item.key : item.key,
    );

    expect(new Set(keys).size).toBe(keys.length);
    expect(items.filter((item) => item.kind === "assistant-meta")).toHaveLength(1);
  });
});
