// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { MarkdownRenderer } from "./MarkdownRenderer";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("MarkdownRenderer math compatibility", () => {
  it("renders LaTeX display delimiters with KaTeX", () => {
    act(() => {
      root.render(createElement(MarkdownRenderer, { content: String.raw`\[x^2\]` }));
    });

    expect(container.querySelector(".katex-display")).not.toBeNull();
    expect(container.textContent).not.toContain(String.raw`\[x^2\]`);
  });

  it("keeps LaTeX inside fenced code blocks as source text", () => {
    const content = "```latex\n\\[x^2\\]\n```";

    act(() => {
      root.render(createElement(MarkdownRenderer, { content }));
    });

    expect(container.querySelector(".katex")).toBeNull();
    expect(container.textContent).toContain(String.raw`\[x^2\]`);
  });
});
