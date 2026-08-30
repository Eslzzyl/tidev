import { describe, it, expect } from "vitest";
import { routes, getActiveFeature } from "./routes";

describe("routes helper functions", () => {
  it("generates correct chat paths", () => {
    expect(routes.root()).toBe("/");
    expect(routes.chat()).toBe("/chat");
    expect(routes.chat("session-123")).toBe("/chat/session-123");
  });

  it("generates correct files paths with URI encoding", () => {
    expect(routes.files()).toBe("/files");
    expect(routes.files("src/main.rs")).toBe("/files?path=src%2Fmain.rs");
    expect(routes.files("path with spaces/file.ts")).toBe(
      "/files?path=path%20with%20spaces%2Ffile.ts",
    );
  });

  it("generates correct terminal paths", () => {
    expect(routes.terminal()).toBe("/terminal");
    expect(routes.terminal("tab-456")).toBe("/terminal/tab-456");
  });

  it("generates correct git paths", () => {
    expect(routes.git()).toBe("/git/changes");
    expect(routes.git("changes")).toBe("/git/changes");
    expect(routes.git("history")).toBe("/git/history");
    expect(routes.git("history", "c73fa91")).toBe("/git/history/c73fa91");
    expect(routes.git("branches")).toBe("/git/branches");
  });

  it("generates correct stats paths with query parameters", () => {
    expect(routes.stats()).toBe("/stats");
    expect(routes.stats("7d")).toBe("/stats?range=7d");
    expect(routes.stats("30d", "day")).toBe("/stats?range=30d&granularity=day");
  });

  it("generates correct settings paths", () => {
    expect(routes.settings()).toBe("/settings");
    expect(routes.settings("appearance")).toBe("/settings/appearance");
    expect(routes.settings("mcp")).toBe("/settings/mcp");
  });
});

describe("getActiveFeature", () => {
  it("correctly determines active feature from pathname", () => {
    expect(getActiveFeature("/chat")).toBe("chat");
    expect(getActiveFeature("/chat/sess-123")).toBe("chat");
    expect(getActiveFeature("/")).toBe("chat");
    expect(getActiveFeature("/files")).toBe("files");
    expect(getActiveFeature("/files/some/path")).toBe("files");
    expect(getActiveFeature("/terminal")).toBe("terminal");
    expect(getActiveFeature("/terminal/tab-1")).toBe("terminal");
    expect(getActiveFeature("/git")).toBe("git");
    expect(getActiveFeature("/git/changes")).toBe("git");
    expect(getActiveFeature("/git/history/sha123")).toBe("git");
    expect(getActiveFeature("/git/branches")).toBe("git");
    expect(getActiveFeature("/stats")).toBe("stats");
  });
});
