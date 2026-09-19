import { describe, expect, it } from "vitest";

import { normalizeMathDelimiters } from "./normalizeMathDelimiters";

describe("normalizeMathDelimiters", () => {
  it("converts display LaTeX delimiters", () => {
    expect(normalizeMathDelimiters(String.raw`\[x^2 + y^2\]`)).toBe("$$\nx^2 + y^2\n$$");
  });

  it("converts inline LaTeX delimiters", () => {
    expect(normalizeMathDelimiters(String.raw`The value is \(x + 1\).`)).toBe(
      "The value is $x + 1$.",
    );
  });

  it("preserves multiline display formulas", () => {
    const input = String.raw`\[
\begin{aligned}
x &= y
\end{aligned}
\]`;

    expect(normalizeMathDelimiters(input)).toBe(`$$
\\begin{aligned}
x &= y
\\end{aligned}
$$`);
  });

  it("preserves fenced code blocks", () => {
    const input = "```latex\n\\[x^2\\]\n```";

    expect(normalizeMathDelimiters(input)).toBe(input);
  });

  it("preserves inline code spans", () => {
    const input = "Use `\\[x\\]` as a display delimiter.";

    expect(normalizeMathDelimiters(input)).toBe(input);
  });

  it("leaves unmatched delimiters unchanged", () => {
    const input = String.raw`An incomplete formula: \[x^2`;
    expect(normalizeMathDelimiters(input)).toBe(input);
  });

  it("preserves already supported dollar delimiters", () => {
    const input = "Inline $x$ and display $$y$$.";
    expect(normalizeMathDelimiters(input)).toBe(input);
  });
});
