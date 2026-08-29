import { describe, expect, it } from "vitest";

import { formatThinkingLevel, isThinkingLevelEnabled } from "./chat";

describe("formatThinkingLevel", () => {
  it("uses the existing friendly Chinese labels for serialized levels", () => {
    expect(formatThinkingLevel("gpt5:high", "zh-CN")).toBe("高");
    expect(formatThinkingLevel({ qwen38: "medium" }, "zh-CN")).toBe("中");
    expect(formatThinkingLevel("qwen38:x_high", "zh-CN")).toBe("极高");
  });

  it("treats disabled levels as unavailable footer metadata", () => {
    expect(isThinkingLevelEnabled("none")).toBe(false);
    expect(isThinkingLevelEnabled({ gpt5: "off" })).toBe(false);
    expect(isThinkingLevelEnabled({ gpt5: "high" })).toBe(true);
  });
});
