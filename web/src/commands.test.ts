import { describe, expect, it } from "vitest";

import { commandFragment, getSuggestions } from "./commands";

describe("commandFragment", () => {
  it("returns the command query at the caret", () => {
    expect(commandFragment("  /und", 6)).toBe("und");
    expect(commandFragment("/undo args")).toBeNull();
  });

  it("does not activate for text outside a command prefix", () => {
    expect(commandFragment("explain /undo")).toBeNull();
    expect(commandFragment("/undo", 0)).toBeNull();
  });
});

describe("getSuggestions", () => {
  it("returns matching commands for an empty slash query", () => {
    const suggestions = getSuggestions("");
    expect(suggestions.length).toBeGreaterThan(0);
    expect(suggestions[0]?.spec.name).toBe("compact");
  });

  it("hides fast mode for non-GPT models", () => {
    expect(getSuggestions("fast", true).some((item) => item.spec.name === "fast")).toBe(true);
    expect(getSuggestions("fast", false).some((item) => item.spec.name === "fast")).toBe(false);
  });
});
