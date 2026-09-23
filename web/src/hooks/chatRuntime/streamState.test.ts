import { describe, expect, it } from "vitest";

import {
  appendReasoningDelta,
  appendReasoningSummaryDelta,
  cloneStream,
  createStream,
} from "./streamState";

describe("stream reasoning display", () => {
  it("keeps summary groups separate while preserving the raw delta order", () => {
    const stream = createStream("session-1:1", 1);

    appendReasoningSummaryDelta(stream, 0, "Plan");
    appendReasoningSummaryDelta(stream, 0, " the work");
    appendReasoningSummaryDelta(stream, 1, "Check the result");
    appendReasoningDelta(stream, "Ordinary reasoning");

    expect(stream.segments).toEqual([
      {
        type: "reasoning",
        content: "Plan the workCheck the resultOrdinary reasoning",
      },
    ]);
    expect(stream.reasoningDisplay).toEqual({
      summaries: [
        { summaryIndex: 0, content: "Plan the work" },
        { summaryIndex: 1, content: "Check the result" },
      ],
      ordinary: "Ordinary reasoning",
    });
  });

  it("copies summary display state when cloning a stream", () => {
    const stream = createStream("session-1:1", 1);
    appendReasoningSummaryDelta(stream, null, "Summary");

    const cloned = cloneStream(stream);
    appendReasoningSummaryDelta(cloned, null, " copy");

    expect(stream.reasoningDisplay?.summaries[0]?.content).toBe("Summary");
    expect(cloned.reasoningDisplay?.summaries[0]?.content).toBe("Summary copy");
  });
});
