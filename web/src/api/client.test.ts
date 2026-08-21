import { afterEach, describe, expect, it, vi } from "vitest";

import { api } from "./client";

describe("prompt API contract", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("preserves the prompt endpoint and JSON field order", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ message_id: "message-1", duplicate: false }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await api.sendPrompt("session-1", "inspect this", "plan", "message-1", "high");

    expect(fetchMock).toHaveBeenCalledWith("/api/sessions/session-1/prompts", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: '{"content":"inspect this","mode":"plan","message_id":"message-1","thinking_level":"high"}',
    });
  });

  it("uses the backend message-record response shape", async () => {
    const response = {
      messages: [
        {
          message: { id: "message-1", role: "user", content: "hello", created_at: "now" },
          app_data: {},
        },
      ],
    };
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => response }),
    );

    await expect(api.listMessages("session-1")).resolves.toEqual(response);
  });
});
