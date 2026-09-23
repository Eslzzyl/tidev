import { describe, expect, it } from "vitest";
import { hasConnectedProvider } from "./providers";

describe("hasConnectedProvider", () => {
  it("returns false when every provider is disconnected", () => {
    expect(hasConnectedProvider([{ connected: false }, { connected: false }])).toBe(false);
  });

  it("returns true when any provider is connected", () => {
    expect(hasConnectedProvider([{ connected: false }, { connected: true }])).toBe(true);
  });
});
