// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { ThinkingBlock } from "./ThinkingBlock";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: false });
});

describe("ThinkingBlock summary rendering", () => {
  it("renders each summary and ordinary reasoning as separate Markdown blocks", () => {
    act(() => {
      root.render(
        createElement(ThinkingBlock, {
          content: "First summarySecond summaryOrdinary reasoning",
          display: {
            summaries: [
              { summaryIndex: 0, content: "First summary" },
              { summaryIndex: 1, content: "Second summary" },
            ],
            ordinary: "Ordinary reasoning",
          },
          expanded: true,
        }),
      );
    });

    expect(
      Array.from(container.querySelectorAll(".thinking-markdown .markdown-body"), (block) =>
        block.textContent?.trim(),
      ),
    ).toEqual(["First summary", "Second summary", "Ordinary reasoning"]);
  });

  it("renders raw reasoning as a single Markdown block without display metadata", () => {
    act(() => {
      root.render(
        createElement(ThinkingBlock, {
          content: "Persisted reasoning",
          expanded: true,
        }),
      );
    });

    expect(container.querySelectorAll(".thinking-markdown .markdown-body")).toHaveLength(1);
    expect(container.textContent).toContain("Persisted reasoning");
  });

  it("keeps incrementally updated summaries in separate blocks while streaming", () => {
    const props = {
      content: "Summary oneSummary twoOrdinary reasoning",
      display: {
        summaries: [{ summaryIndex: 0, content: "Summary one" }],
        ordinary: "",
      },
      expanded: true,
    };

    act(() => root.render(createElement(ThinkingBlock, props)));
    act(() =>
      root.render(
        createElement(ThinkingBlock, {
          ...props,
          display: {
            summaries: [
              { summaryIndex: 0, content: "Summary one" },
              { summaryIndex: 1, content: "Summary two" },
            ],
            ordinary: "Ordinary reasoning",
          },
        }),
      ),
    );

    expect(
      Array.from(container.querySelectorAll(".thinking-markdown .markdown-body"), (block) =>
        block.textContent?.trim(),
      ),
    ).toEqual(["Summary one", "Summary two", "Ordinary reasoning"]);
  });
});
