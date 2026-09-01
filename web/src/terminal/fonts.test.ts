import { describe, expect, it } from "vitest";

import { areMonospaceWidths, collectMonospaceFamilies, createTerminalFontInputs } from "./fonts";

describe("terminal fonts", () => {
  it("recognizes equal character widths", () => {
    expect(areMonospaceWidths([10, 10, 10.005])).toBe(true);
    expect(areMonospaceWidths([10, 10.02])).toBe(false);
    expect(areMonospaceWidths([])).toBe(false);
  });

  it("deduplicates and sorts measured monospace families", () => {
    const families = collectMonospaceFamilies(
      [
        { family: "ProggyClean" },
        { family: "Arial" },
        { family: "ProggyClean", style: "Bold" },
        { family: "Noto Color Emoji" },
      ],
      (family) => family === "ProggyClean",
    );

    expect(families).toEqual(["ProggyClean"]);
  });

  it("puts a selected family before the Unicode fallback chain", () => {
    const fonts = createTerminalFontInputs("Cascadia Mono");

    expect(fonts[0]).toEqual({ family: "Cascadia Mono", local: "prefer" });
    expect(fonts.some((font) => typeof font === "object" && "url" in font)).toBe(true);
    expect(
      fonts.some(
        (font) => typeof font === "object" && "family" in font && font.family === "Microsoft YaHei",
      ),
    ).toBe(true);
  });
});
