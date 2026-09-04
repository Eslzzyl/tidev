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
 * Select the newest request associated with a user turn. Request IDs are
 * monotonic within a session, so this remains correct when stale stream state
 * survives a reconnect or resync. Terminal streams remain associated until
 * their already-received content has been rendered with the originating turn.
 */
export function latestTurnStream(
  streams: readonly StreamMessage[],
  userMessageId: string,
): StreamMessage | undefined {
  let latest: StreamMessage | undefined;
  for (const stream of streams) {
    if (stream.userMessageId !== userMessageId) continue;
    if (!latest || stream.requestId >= latest.requestId) latest = stream;
  }
  return latest;
}
