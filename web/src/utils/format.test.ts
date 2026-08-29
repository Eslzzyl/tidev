import { describe, expect, it } from "vitest";

import { formatThinkingDuration } from "./format";

function translate(key: string, options?: Record<string, unknown>) {
  switch (key) {
    case "{{count}} milliseconds":
      return `${options?.count}ms`;
    case "{{count}} seconds":
      return `${options?.count}s`;
    case "{{minutes}} minutes {{seconds}} seconds":
      return `${options?.minutes}m ${options?.seconds}s`;
    case "{{hours}} hours {{minutes}} minutes {{seconds}} seconds":
      return `${options?.hours}h ${options?.minutes}m ${options?.seconds}s`;
    default:
      return key;
  }
}

describe("formatThinkingDuration", () => {
  it("uses milliseconds before the first second", () => {
    expect(formatThinkingDuration(37, translate)).toBe("37ms");
    expect(formatThinkingDuration(999, translate)).toBe("999ms");
  });

  it("uses tenths of a second before the first minute", () => {
    expect(formatThinkingDuration(1000, translate)).toBe("1.0s");
    expect(formatThinkingDuration(4123, translate)).toBe("4.1s");
    expect(formatThinkingDuration(59999, translate)).toBe("59.9s");
  });

  it("switches to whole seconds at one minute and one hour", () => {
    expect(formatThinkingDuration(60000, translate)).toBe("1m 0s");
    expect(formatThinkingDuration(61000, translate)).toBe("1m 1s");
    expect(formatThinkingDuration(3600000, translate)).toBe("1h 0m 0s");
    expect(formatThinkingDuration(3661000, translate)).toBe("1h 1m 1s");
  });
});
