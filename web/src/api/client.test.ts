import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError, api } from "./client";

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

  it("rejects empty or whitespace-only session IDs without making requests", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    expect(() => api.listMessages("")).toThrow("Session ID is required");
    expect(() => api.listMessages("   ")).toThrow("Session ID is required");
    expect(() => api.getSession("")).toThrow("Session ID is required");
    expect(() => api.getTodos("")).toThrow("Session ID is required");
    expect(() => api.sendPrompt("", "hello", "build", "msg-1")).toThrow("Session ID is required");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("uses the provider list response envelope", async () => {
    const response = {
      providers: [
        {
          id: "deepseek",
          display_name: "DeepSeek",
          source: "bundled",
          can_delete: false,
          connected: false,
          base_url: "https://api.deepseek.com",
          api_type: null,
          models: [],
        },
      ],
    };
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => response,
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.listProviders()).resolves.toEqual(response);
    expect(fetchMock).toHaveBeenCalledWith("/api/providers", {
      headers: { "Content-Type": "application/json" },
    });
  });

  it("surfaces provider mutation errors from the API", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 403,
        json: async () => ({ error: "bundled provider cannot be deleted" }),
      }),
    );

    await expect(api.deleteProvider("deepseek")).rejects.toThrow(
      "bundled provider cannot be deleted",
    );
  });

  it("preserves the HTTP status for API errors", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 404,
        json: async () => ({ error: "session not found" }),
      }),
    );

    const request = api.getSession("session-1");
    await expect(request).rejects.toBeInstanceOf(ApiError);
    await expect(request).rejects.toMatchObject({
      name: "ApiError",
      status: 404,
      message: "session not found",
    });
  });
});
