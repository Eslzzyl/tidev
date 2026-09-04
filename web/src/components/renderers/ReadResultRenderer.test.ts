// @vitest-environment jsdom

import { act, createElement, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { MessageAttachment } from "../../types/api";
import { parseDirectoryReadOutput, ReadResultRenderer, readResultKind } from "./ReadResultRenderer";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.stubGlobal("URL", {
    ...URL,
    createObjectURL: vi.fn(() => "blob:read-result-preview"),
    revokeObjectURL: vi.fn(),
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe("readResultKind", () => {
  it("prefers image attachments over the generic text result", () => {
    const attachments: MessageAttachment[] = [
      {
        type: "image",
        filename: "diagram.png",
        mime: "image/png",
        data: [137, 80, 78, 71],
        file_size: 4,
      },
    ];

    expect(readResultKind(attachments)).toBe("image");
  });

  it("recognizes a directory reference", () => {
    const attachments: MessageAttachment[] = [
      { type: "directory_reference", path: "src", tree: "" },
    ];

    expect(readResultKind(attachments)).toBe("directory");
  });

  it("uses the line renderer when no typed attachment is present", () => {
    expect(readResultKind([])).toBe("text");
  });
});

describe("parseDirectoryReadOutput", () => {
  it("keeps only directory entries and puts directories first", () => {
    const directory = parseDirectoryReadOutput(
      "src/\nmain.ts\ncomponents/\napp.tsx\n\n<system-reminder>ignored</system-reminder>",
      "src",
    );

    expect(directory).toEqual({
      path: "src",
      entries: [
        { name: "components/", isDirectory: true },
        { name: "app.tsx", isDirectory: false },
        { name: "main.ts", isDirectory: false },
      ],
    });
  });

  it("uses the attachment path for an empty directory", () => {
    expect(parseDirectoryReadOutput("(empty)", "assets")).toEqual({
      path: "assets",
      entries: [],
    });
  });
});

describe("ReadResultRenderer", () => {
  it("renders an image preview when the read attachment contains bytes", async () => {
    await act(async () => {
      root.render(
        createElement(ReadResultRenderer, {
          output: "Image read successfully.",
          attachments: [
            {
              type: "image",
              filename: "301374.png",
              mime: "image/png",
              data: [137, 80, 78, 71],
              file_size: 4,
            },
          ],
        }),
      );
    });

    const image = container.querySelector(".tool-read-image img");
    expect(image).not.toBeNull();
    expect(image?.getAttribute("src")).toBe("blob:read-result-preview");
    expect(container.querySelector(".tool-code-lines-fallback")).toBeNull();
    expect(container.querySelector(".tool-read-result-header")).toBeNull();
  });

  it("keeps the active preview URL alive in Strict Mode", async () => {
    let nextUrl = 0;
    vi.stubGlobal("URL", {
      ...URL,
      createObjectURL: vi.fn(() => `blob:read-result-preview-${++nextUrl}`),
      revokeObjectURL: vi.fn(),
    });

    await act(async () => {
      root.render(
        createElement(
          StrictMode,
          null,
          createElement(ReadResultRenderer, {
            output: "Image read successfully.",
            attachments: [
              {
                type: "image",
                filename: "301374.png",
                mime: "image/png",
                data: [137, 80, 78, 71],
                file_size: 4,
              },
            ],
          }),
        ),
      );
    });

    const src = container.querySelector(".tool-read-image img")?.getAttribute("src");
    expect(src).toBeTruthy();
    expect(URL.revokeObjectURL).not.toHaveBeenCalledWith(src);
  });
});
