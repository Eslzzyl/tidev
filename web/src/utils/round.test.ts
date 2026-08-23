import { describe, expect, it } from "vitest";

import type { Message, MessageRecord } from "../types/api";
import {
  buildRounds,
  getRoundPreviewIndex,
  isRoundCollapsible,
  parseInstructionMessage,
  type Round,
} from "./round";

function message(content: string, role: Message["role"] = "assistant"): Message {
  return {
    id: `${role}-1`,
    role,
    content,
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
    created_at: "2026-08-22T00:00:00Z",
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
  };
}

function record(messageValue: Message): MessageRecord {
  return { message: messageValue, app_data: {} };
}

function round(overrides: Partial<Round> = {}): Round {
  return {
    id: "round-1",
    userMessage: message("prompt", "user"),
    leadingInstructions: [],
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

  it("freezes a reasoning segment at the assistant completion time when needed", () => {
    const assistant = message("", "assistant");
    assistant.reasoning = "thinking";
    assistant.reasoning_started_at = "2026-08-22T00:00:10.000Z";
    assistant.completed_at = "2026-08-22T00:00:12.500Z";

    const [built] = buildRounds([record(message("prompt", "user")), record(assistant)]) as [Round];

    expect(built.segments[0]).toMatchObject({
      type: "reasoning",
      startedAt: assistant.reasoning_started_at,
      completedAt: assistant.completed_at,
    });
  });

  it("keeps direct tool metadata and derives completed status", () => {
    const assistant = message("", "assistant");
    assistant.completed_at = "2026-08-22T00:01:00Z";
    assistant.tool_calls = [{ id: "call-1", name: "edit", arguments: "{}" }];
    const tool = message("Updated src/main.rs", "tool");
    tool.tool_call_id = "call-1";
    tool.tool_name = "edit";
    tool.metadata.filepath = "src/main.rs";
    tool.metadata.diff = "@@ -1 +1 @@\n-old\n+new";

    const [built] = buildRounds([
      record(message("prompt", "user")),
      record(assistant),
      record(tool),
    ]) as [Round];
    const entry = built.toolCallMap["call-1"];

    expect(entry.status).toBe("completed");
    expect(entry.result?.metadata.filepath).toBe("src/main.rs");
    expect(entry.result?.metadata.diff).toContain("+new");
  });

  it("marks a non-zero shell result as failed without parsing display text", () => {
    const assistant = message("", "assistant");
    assistant.completed_at = "2026-08-22T00:01:00Z";
    assistant.tool_calls = [{ id: "call-1", name: "shell", arguments: "{}" }];
    const tool = message("[exit 2]\nfailed", "tool");
    tool.tool_call_id = "call-1";
    tool.tool_name = "shell";
    tool.metadata.exit_code = 2;

    const [built] = buildRounds([
      record(message("prompt", "user")),
      record(assistant),
      record(tool),
    ]) as [Round];

    expect(built.toolCallMap["call-1"].status).toBe("failed");
  });

  it("keeps initial instruction notices after the user message in the same round", () => {
    const assistant = message("answer", "assistant");
    assistant.completed_at = "2026-08-22T00:01:00Z";
    const instruction = message("Loaded instructions from AGENTS.md", "system");

    const built = buildRounds([
      record(message("prompt", "user")),
      record(instruction),
      record(assistant),
    ]);

    expect(built).toHaveLength(1);
    expect(built[0]).toMatchObject({
      leadingInstructions: [instruction],
      segments: [{ type: "text", content: "answer" }],
    });
  });

  it("keeps tool-discovered instruction notices between tool calls and the next answer", () => {
    const toolAssistant = message("", "assistant");
    toolAssistant.tool_calls = [{ id: "call-1", name: "read", arguments: "{}" }];
    const tool = message("read output", "tool");
    tool.tool_call_id = "call-1";
    tool.tool_name = "read";
    const instruction = message(
      "Loaded 2 instruction files: src/AGENTS.md, crates/AGENTS.md",
      "system",
    );
    const finalAssistant = message("answer", "assistant");
    finalAssistant.completed_at = "2026-08-22T00:01:00Z";

    const built = buildRounds([
      record(message("prompt", "user")),
      record(toolAssistant),
      record(tool),
      record(instruction),
      record(finalAssistant),
    ]);

    expect(built).toHaveLength(1);
    expect((built[0] as Round).segments.map((segment) => segment.type)).toEqual([
      "tool_call",
      "instruction",
      "text",
    ]);
  });

  it("parses both single-file and multi-file instruction notices", () => {
    expect(parseInstructionMessage("Loaded instructions from AGENTS.md")).toEqual({
      count: null,
      sources: "AGENTS.md",
    });
    expect(parseInstructionMessage("Loaded 2 instruction files: a/AGENTS.md, b/AGENTS.md")).toEqual(
      {
        count: 2,
        sources: "a/AGENTS.md, b/AGENTS.md",
      },
    );
    expect(parseInstructionMessage("Compaction\n\nsummary")).toBeNull();
  });
});
