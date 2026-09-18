import { describe, expect, it } from "vitest";

import type { Message, MessageRecord } from "../../types/api";
import { mergeMessageRecords } from "./messageState";

function record(id: string, content: string): MessageRecord {
  const message: Message = {
    id,
    role: "user",
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
    created_at: "2026-09-18T00:00:00Z",
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
  return { message, app_data: {} };
}

describe("mergeMessageRecords", () => {
  it("keeps a message received from SSE when a stale load is empty", () => {
    const live = record("user-1", "hello");

    expect(mergeMessageRecords([], [live])).toEqual([live]);
  });

  it("keeps the loaded version when both sources contain the same message", () => {
    const loaded = record("user-1", "loaded");
    const live = record("user-1", "live");

    expect(mergeMessageRecords([loaded], [live])).toEqual([loaded]);
  });
});
