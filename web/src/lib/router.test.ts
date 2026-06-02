import { describe, it, expect, beforeEach } from "vitest";
import { parseRoute, buildURL } from "./router";

describe("buildURL", () => {
  it("builds chat URL with no session", () => {
    expect(buildURL({ tab: "chat", sessionId: null })).toBe("#chat");
  });

  it("builds chat URL with session id", () => {
    expect(buildURL({ tab: "chat", sessionId: "abc-123" })).toBe("#chat/abc-123");
  });

  it("builds settings URL", () => {
    expect(buildURL({ tab: "settings", sessionId: null })).toBe("#settings");
  });

  it("builds files URL", () => {
    expect(buildURL({ tab: "files", sessionId: null })).toBe("#files");
  });

  it("builds terminal URL", () => {
    expect(buildURL({ tab: "terminal", sessionId: null })).toBe("#terminal");
  });

  it("builds git URL", () => {
    expect(buildURL({ tab: "git", sessionId: null })).toBe("#git");
  });

  it("builds stats URL", () => {
    expect(buildURL({ tab: "stats", sessionId: null })).toBe("#stats");
  });
});

describe("parseRoute", () => {
  beforeEach(() => {
    // Default hash
    window.location.hash = "";
  });

  it("parses chat tab with session", () => {
    window.location.hash = "#chat/session-1";
    expect(parseRoute()).toEqual({ tab: "chat", sessionId: "session-1" });
  });

  it("parses chat tab alone", () => {
    window.location.hash = "#chat";
    expect(parseRoute()).toEqual({ tab: "chat", sessionId: null });
  });

  it("defaults to chat when hash is empty", () => {
    window.location.hash = "";
    expect(parseRoute()).toEqual({ tab: "chat", sessionId: null });
  });

  it("defaults to chat when hash is just #", () => {
    window.location.hash = "#";
    expect(parseRoute()).toEqual({ tab: "chat", sessionId: null });
  });

  it("parses settings tab", () => {
    window.location.hash = "#settings";
    expect(parseRoute()).toEqual({ tab: "settings", sessionId: null });
  });

  it("falls back to chat for unknown tab", () => {
    window.location.hash = "#unknown";
    expect(parseRoute()).toEqual({ tab: "chat", sessionId: null });
  });

  it("ignores extra path segments beyond session id", () => {
    window.location.hash = "#chat/abc/extra/stuff";
    expect(parseRoute()).toEqual({ tab: "chat", sessionId: "abc" });
  });

  it("parses files tab", () => {
    window.location.hash = "#files";
    expect(parseRoute()).toEqual({ tab: "files", sessionId: null });
  });

  it("parses terminal tab", () => {
    window.location.hash = "#terminal";
    expect(parseRoute()).toEqual({ tab: "terminal", sessionId: null });
  });

  it("parses git tab", () => {
    window.location.hash = "#git";
    expect(parseRoute()).toEqual({ tab: "git", sessionId: null });
  });

  it("parses stats tab", () => {
    window.location.hash = "#stats";
    expect(parseRoute()).toEqual({ tab: "stats", sessionId: null });
  });
});
