import { describe, expect, it } from "vitest";

import type { StreamMessage } from "../types/chat";
import { latestLiveStream, segmentReasoningTiming } from "./stream";

function stream(requestId: number, status: StreamMessage["status"] = "streaming"): StreamMessage {
  return {
    key: `session:${requestId}`,
    requestId,
    segments: [],
    toolCallMap: {},
    status,
    providerFinished: false,
    reasoningStartedAt: null,
    reasoningCompletedAt: null,
    userMessageId: "user-1",
  };
}

describe("stream helpers", () => {
  it("keeps a live reasoning segment open when the persisted round has finished", () => {
    expect(
      segmentReasoningTiming({
        isLiveSegment: true,
        reasoningStartedAt: "2026-08-22T00:00:02.000Z",
        reasoningCompletedAt: "2026-08-22T00:00:02.500Z",
        activeReasoningStartedAt: "2026-08-22T00:00:10.000Z",
        activeReasoningCompletedAt: null,
      }),
    ).toEqual({
      startedAt: "2026-08-22T00:00:10.000Z",
      completedAt: null,
    });
  });

  it("uses the live completion timestamp after the active segment finishes", () => {
    expect(
      segmentReasoningTiming({
        isLiveSegment: true,
        reasoningStartedAt: "2026-08-22T00:00:02.000Z",
        reasoningCompletedAt: "2026-08-22T00:00:02.500Z",
        activeReasoningStartedAt: "2026-08-22T00:00:10.000Z",
        activeReasoningCompletedAt: "2026-08-22T00:00:12.000Z",
      }),
    ).toEqual({
      startedAt: "2026-08-22T00:00:10.000Z",
      completedAt: "2026-08-22T00:00:12.000Z",
    });
  });

  it("selects the newest active request and ignores non-streaming entries", () => {
    const older = stream(2);
    const newest = stream(5);
    const failed = stream(8, "failed");

    expect(latestLiveStream([newest, older, failed], "user-1")).toBe(newest);
    expect(latestLiveStream([stream(1)], "other-user")).toBeUndefined();
  });
});
