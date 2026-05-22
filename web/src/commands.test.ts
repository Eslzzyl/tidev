import { describe, it, expect } from "vitest";
import { getSuggestions, commandFragment, COMMANDS } from "./commands";

describe("commandFragment", () => {
  it("returns fragment when input starts with / and no space", () => {
    expect(commandFragment("/ren")).toBe("ren");
  });

  it("returns null when input starts with / followed by space", () => {
    expect(commandFragment("/ rename")).toBe(null);
  });

  it("returns null when input does not start with /", () => {
    expect(commandFragment("hello")).toBe(null);
  });

  it("returns null for empty input", () => {
    expect(commandFragment("")).toBe(null);
  });

  it("handles whitespace before slash", () => {
    expect(commandFragment("  /undo")).toBe("undo");
  });

  it("returns null for just a slash", () => {
    expect(commandFragment("/")).toBe("");
  });

  it("works with full command name", () => {
    expect(commandFragment("/rename")).toBe("rename");
  });
});

describe("getSuggestions", () => {
  it("returns all commands sorted by score when query is empty", () => {
    const suggestions = getSuggestions("");
    expect(suggestions).toHaveLength(COMMANDS.length);
    // All items should have score 1000
    suggestions.forEach((s) => expect(s.score).toBe(1000));
  });

  it("returns exact name match with top score", () => {
    const suggestions = getSuggestions("rename");
    expect(suggestions.length).toBeGreaterThanOrEqual(1);
    expect(suggestions[0].spec.name).toBe("rename");
    expect(suggestions[0].score).toBe(10000);
  });

  it("sorts by score descending, then alphabetically", () => {
    const suggestions = getSuggestions("re");
    for (let i = 1; i < suggestions.length; i++) {
      if (suggestions[i].score === suggestions[i - 1].score) {
        expect(
          suggestions[i].spec.name.localeCompare(suggestions[i - 1].spec.name),
        ).toBeGreaterThanOrEqual(0);
      } else {
        expect(suggestions[i].score).toBeLessThanOrEqual(
          suggestions[i - 1].score,
        );
      }
    }
  });

  it("matches alias exactly (msg -> message)", () => {
    const suggestions = getSuggestions("msg");
    expect(suggestions.length).toBeGreaterThanOrEqual(1);
    expect(suggestions[0].spec.name).toBe("message");
    expect(suggestions[0].score).toBe(9500);
  });

  it("matches alias prefix (tit -> rename)", () => {
    const suggestions = getSuggestions("tit");
    const rename = suggestions.find((s) => s.spec.name === "rename");
    expect(rename).toBeDefined();
    expect(rename!.score).toBe(7500);
  });

  it("does case-insensitive matching", () => {
    const lower = getSuggestions("rename");
    const upper = getSuggestions("RENAME");
    expect(lower[0].score).toBe(upper[0].score);
  });

  it("returns multiple matches for partial query", () => {
    const suggestions = getSuggestions("re");
    // "rename" and "redo" both start with "re"
    expect(suggestions.length).toBeGreaterThanOrEqual(2);
  });

  it("returns empty array for completely unmatched query", () => {
    const suggestions = getSuggestions("zzzzz");
    expect(suggestions).toHaveLength(0);
  });

  it("matches substring in name (position-based scoring)", () => {
    const suggestions = getSuggestions("kills");
    const skills = suggestions.find((s) => s.spec.name === "skills");
    expect(skills).toBeDefined();
    expect(skills!.score).toBeGreaterThan(0);
  });

  it("matches substring in alias (lower score)", () => {
    const suggestions = getSuggestions("ear");
    const clear = suggestions.find((s) => s.spec.name === "new");
    expect(clear).toBeDefined();
    expect(clear!.score).toBe(3500);
  });
});
