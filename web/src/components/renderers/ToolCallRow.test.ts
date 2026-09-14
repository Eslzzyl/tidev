import { describe, expect, it } from "vitest";
import { parseMcpCatalogOutput } from "./McpCatalogRenderer";
import { normalizeToolOutput } from "./ToolCallRow";

describe("parseMcpCatalogOutput", () => {
  it("parses server JSONL from mcp_list without a server argument", () => {
    expect(
      parseMcpCatalogOutput(
        "mcp_list",
        { server: "" },
        '{"server":"blender","kind":"stdio","status":"disabled","tool_count":0}',
      ),
    ).toEqual({
      kind: "servers",
      records: [{ server: "blender", kind: "stdio", status: "disabled", tool_count: 0 }],
    });
  });

  it("parses tool JSONL from mcp_search", () => {
    expect(
      parseMcpCatalogOutput(
        "mcp_search",
        { query: "scene" },
        '{"server":"blender","tool":"get_scene","description":"Get the scene","input_schema":{},"read_only":true}',
      ),
    ).toEqual({
      kind: "tools",
      records: [
        {
          server: "blender",
          tool: "get_scene",
          description: "Get the scene",
          input_schema: {},
          read_only: true,
        },
      ],
    });
  });

  it("falls back for a truncated catalog result", () => {
    expect(parseMcpCatalogOutput("mcp_search", { query: "scene" }, "[Output truncated]")).toBe(
      null,
    );
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
