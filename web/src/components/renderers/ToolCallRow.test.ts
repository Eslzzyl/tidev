import { describe, expect, it } from "vitest";
import { parseMcpToolName, normalizeToolOutput } from "./ToolCallRow";

describe("parseMcpToolName", () => {
  it("extracts server and tool name correctly", () => {
    expect(parseMcpToolName("mcp__blender__get_scene_info")).toEqual({
      server: "blender",
      tool: "get_scene_info",
    });
    expect(parseMcpToolName("mcp__github__create_issue")).toEqual({
      server: "github",
      tool: "create_issue",
    });
  });

  it("handles multi-underscore tool names", () => {
    expect(parseMcpToolName("mcp__server__foo__bar")).toEqual({
      server: "server",
      tool: "foo__bar",
    });
  });

  it("returns null for non-mcp tools", () => {
    expect(parseMcpToolName("read")).toBeNull();
    expect(parseMcpToolName("write")).toBeNull();
    expect(parseMcpToolName("bash")).toBeNull();
    expect(parseMcpToolName("mcp_single")).toBeNull();
  });
});

describe("normalizeToolOutput", () => {
  it("unwraps JSON string within result wrapper", () => {
    const raw = JSON.stringify({
      result: JSON.stringify({ name: "Scene", object_count: 3 }),
    });
    const normalized = normalizeToolOutput(raw);
    expect(normalized.isJson).toBe(true);
    expect(normalized.data).toEqual({ name: "Scene", object_count: 3 });
    expect(normalized.text).toContain('"name": "Scene"');
  });

  it("unwraps plain text string within result wrapper", () => {
    const raw = JSON.stringify({
      result: "Code executed successfully: 5.2.1 LTS 3",
    });
    const normalized = normalizeToolOutput(raw);
    expect(normalized.isJson).toBe(false);
    expect(normalized.data).toBeNull();
    expect(normalized.text).toBe("Code executed successfully: 5.2.1 LTS 3");
  });

  it("unwraps nested object within result wrapper", () => {
    const raw = JSON.stringify({
      result: { up_to_date: true, version: "1.0.0" },
    });
    const normalized = normalizeToolOutput(raw);
    expect(normalized.isJson).toBe(true);
    expect(normalized.data).toEqual({ up_to_date: true, version: "1.0.0" });
  });

  it("parses direct JSON object", () => {
    const raw = JSON.stringify({ status: "ok" });
    const normalized = normalizeToolOutput(raw);
    expect(normalized.isJson).toBe(true);
    expect(normalized.data).toEqual({ status: "ok" });
  });

  it("handles plain text output untouched", () => {
    const raw = "File written successfully";
    const normalized = normalizeToolOutput(raw);
    expect(normalized.isJson).toBe(false);
    expect(normalized.data).toBeNull();
    expect(normalized.text).toBe("File written successfully");
  });
});
