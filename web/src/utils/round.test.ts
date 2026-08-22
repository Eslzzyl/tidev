import { describe, expect, it } from "vitest";

import type { Message, MessageRecord } from "../types/api";
import { buildRounds, getRoundPreviewIndex, isRoundCollapsible, type Round } from "./round";

function message(content: string, role: Message["role"] = "assistant"): Message {
  return {
    id: `${role}-1`,
    role,
    content,
    created_at: "2026-08-22T00:00:00Z",
  };
}

function record(messageValue: Message): MessageRecord {
  return { message: messageValue, app_data: {} };
}

function round(overrides: Partial<Round> = {}): Round {
  return {
    id: "round-1",
    userMessage: message("prompt", "user"),
    segments: [
      { type: "reasoning", content: "thinking" },
      { type: "text", content: "final reply" },
    ],
    toolCallMap: {},
    status: "complete",
    interrupted: false,
    completedAt: "2026-08-22T00:01:00Z",
    ...overrides,
  };
}

describe("round preview state", () => {
  it("keeps only the final text segment for a normally completed round", () => {
    expect(getRoundPreviewIndex(round())).toBe(1);
    expect(isRoundCollapsible(round())).toBe(true);
  });

  it("keeps the last user-facing text for an interrupted round", () => {
    const interrupted = round({
      interrupted: true,
      interruptionKind: "interrupted",
      segments: [
        { type: "reasoning", content: "thinking" },
        { type: "text", content: "partial reply" },
        { type: "reasoning", content: "stopped while thinking" },
      ],
    });

    expect(getRoundPreviewIndex(interrupted)).toBe(1);
    expect(isRoundCollapsible(interrupted)).toBe(true);
  });

  it("uses a status-only preview when interruption produced no text", () => {
    const interrupted = round({
      interrupted: true,
      interruptionKind: "cancelled",
      segments: [{ type: "reasoning", content: "stopped while thinking" }],
    });

    expect(getRoundPreviewIndex(interrupted)).toBeNull();
    expect(isRoundCollapsible(interrupted)).toBe(true);
  });

  it("marks persisted cancellation as an interrupted round", () => {
    const assistant = message("", "assistant");
    assistant.completed_at = "2026-08-22T00:01:00Z";
    assistant.tool_calls = [{ id: "call-1", name: "bash", arguments: "pwd" }];
    const cancelled = message("User cancelled the request", "tool");
    cancelled.tool_call_id = "call-1";
    cancelled.tool_name = "bash";

    const [built] = buildRounds([
      record(message("prompt", "user")),
      record(assistant),
      record(cancelled),
    ]);

    expect(built).toMatchObject({ interrupted: true, interruptionKind: "cancelled" });
  });
});
