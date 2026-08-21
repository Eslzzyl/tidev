import { describe, expect, it } from "vitest";

import { asString, eventPayload } from "./events";

describe("event helpers", () => {
  it("returns the discriminant and payload from an envelope", () => {
    expect(
      eventPayload({
        cursor: 42,
        session_id: "session-1",
        event: { Delta: { request_id: 7, content: "hello" } },
      }),
    ).toEqual(["Delta", { request_id: 7, content: "hello" }]);
  });

  it("handles empty events and non-string values safely", () => {
    expect(eventPayload({ cursor: 1, session_id: "session-1", event: {} })).toEqual(["", {}]);
    expect(asString("text")).toBe("text");
    expect(asString(123)).toBe("");
  });
});
