import { describe, expect, it } from "vitest";

import { parseBackendEvent, parseFrontendRequest } from "./events";

describe("SSE contract parsing", () => {
  it("accepts backend event envelopes only when their required fields exist", () => {
    const envelope = parseBackendEvent(
      JSON.stringify({
        cursor: 9,
        session_id: "session-1",
        event: { StreamEnd: { request_id: 3 } },
      }),
    );
    expect(envelope?.cursor).toBe(9);
    expect(parseBackendEvent("not-json")).toBeNull();
    expect(parseBackendEvent(JSON.stringify({ cursor: "9" }))).toBeNull();
  });

  it("accepts frontend approval requests using UUID string identifiers", () => {
    const request = parseFrontendRequest(
      JSON.stringify({
        request_id: "request-1",
        session_id: "session-1",
        kind: { ToolApproval: [] },
      }),
    );
    expect(request?.request_id).toBe("request-1");
    expect(parseFrontendRequest(JSON.stringify({ request_id: 1 }))).toBeNull();
  });
});
