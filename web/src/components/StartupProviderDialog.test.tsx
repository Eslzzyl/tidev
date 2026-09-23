// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { StartupProviderDialog } from "./StartupProviderDialog";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

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

function clickButton(label: string) {
  const button = Array.from(document.body.querySelectorAll("button")).find(
    (item) => item.textContent === label,
  );
  expect(button).toBeDefined();
  act(() => button?.click());
}

describe("StartupProviderDialog", () => {
  it("opens provider settings from the primary action", () => {
    const onOpenSettings = vi.fn();
    const onDismiss = vi.fn();

    act(() => {
      root.render(createElement(StartupProviderDialog, { onOpenSettings, onDismiss }));
    });
    clickButton("Open provider settings");

    expect(onOpenSettings).toHaveBeenCalledOnce();
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it("dismisses from the secondary action", () => {
    const onOpenSettings = vi.fn();
    const onDismiss = vi.fn();

    act(() => {
      root.render(createElement(StartupProviderDialog, { onOpenSettings, onDismiss }));
    });
    clickButton("Later");

    expect(onDismiss).toHaveBeenCalledOnce();
    expect(onOpenSettings).not.toHaveBeenCalled();
  });
});
