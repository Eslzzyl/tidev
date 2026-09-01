import type { StreamMessage } from "../types/chat";

export interface SegmentReasoningTimingInput {
  isLiveSegment: boolean;
  reasoningStartedAt?: string | null;
  reasoningCompletedAt?: string | null;
  activeReasoningStartedAt?: string | null;
  activeReasoningCompletedAt?: string | null;
}

export interface SegmentReasoningTiming {
  startedAt?: string | null;
  completedAt?: string | null;
}

/**
 * Resolve reasoning timestamps without allowing a persisted round to finish a
 * currently streaming segment.
 */
export function segmentReasoningTiming({
  isLiveSegment,
  reasoningStartedAt,
  reasoningCompletedAt,
  activeReasoningStartedAt,
  activeReasoningCompletedAt,
}: SegmentReasoningTimingInput): SegmentReasoningTiming {
  return {
    startedAt: isLiveSegment
      ? (activeReasoningStartedAt ?? reasoningStartedAt)
      : reasoningStartedAt,
    completedAt: isLiveSegment ? activeReasoningCompletedAt : reasoningCompletedAt,
  };
}

/**
 * Select the newest active request for a user turn. Request IDs are monotonic
 * within a session, so this remains correct when stale stream state survives a
 * reconnect or resync.
 */
export function latestLiveStream(
  streams: readonly StreamMessage[],
  userMessageId: string,
): StreamMessage | undefined {
  let latest: StreamMessage | undefined;
  for (const stream of streams) {
    if (stream.status !== "streaming" || stream.userMessageId !== userMessageId) continue;
    if (!latest || stream.requestId >= latest.requestId) latest = stream;
  }
  return latest;
}
