import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  formatSessionDate,
  formatTime,
  getDuration,
  formatNumber,
  formatToken,
  stripSystemReminderTags,
  formatWorkspace,
} from "./format";

// ─── formatSessionDate ───────────────────────────────────────────

describe("formatSessionDate", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-05-22T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('returns time string for today', () => {
    const today = new Date().toISOString();
    const result = formatSessionDate(today);
    // Should contain hour:minute digits (locale-dependent but always has colon)
    expect(result).toMatch(/\d{1,2}:\d{2}/);
  });

  it('returns "Yesterday" for one day ago', () => {
    const yesterday = new Date(Date.now() - 24 * 60 * 60 * 1000).toISOString();
    expect(formatSessionDate(yesterday)).toBe("Yesterday");
  });

  it('returns weekday name for 2-6 days ago', () => {
    const threeDaysAgo = new Date(
      Date.now() - 3 * 24 * 60 * 60 * 1000,
    ).toISOString();
    const result = formatSessionDate(threeDaysAgo);
    // Should be a short weekday name (e.g. "Mon", "Tue")
    expect(result).not.toBe("Yesterday");
    expect(result).not.toMatch(/\d{1,2}:\d{2}/);
    expect(typeof result).toBe("string");
    expect(result.length).toBeGreaterThan(0);
  });

  it('returns month + day for 7+ days ago', () => {
    const tenDaysAgo = new Date(
      Date.now() - 10 * 24 * 60 * 60 * 1000,
    ).toISOString();
    const result = formatSessionDate(tenDaysAgo);
    // Should contain abbreviated month + day e.g. "May 12"
    expect(result).not.toBe("Yesterday");
    expect(typeof result).toBe("string");
    expect(result.length).toBeGreaterThan(0);
  });
});

// ─── formatTime ──────────────────────────────────────────────────

describe("formatTime", () => {
  it("returns a time string with colon separator", () => {
    const result = formatTime("2026-05-22T14:30:00Z");
    expect(result).toMatch(/\d{1,2}:\d{2}/);
  });
});

// ─── getDuration ─────────────────────────────────────────────────

describe("getDuration", () => {
  it("returns seconds when duration < 60s", () => {
    const start = "2026-05-22T12:00:00Z";
    const end = "2026-05-22T12:00:05.5Z";
    expect(getDuration(start, end)).toBe("5.5s");
  });

  it("returns minutes + seconds when duration >= 60s", () => {
    const start = "2026-05-22T12:00:00Z";
    const end = "2026-05-22T12:02:35Z";
    expect(getDuration(start, end)).toBe("2m 35s");
  });

  it("returns null when completedAt is before createdAt", () => {
    const start = "2026-05-22T12:00:00Z";
    const end = "2026-05-22T11:59:00Z";
    expect(getDuration(start, end)).toBeNull();
  });

  it("returns 0.0s when dates are equal", () => {
    const t = "2026-05-22T12:00:00Z";
    expect(getDuration(t, t)).toBe("0.0s");
  });

  it("handles exact 60s boundary", () => {
    const start = "2026-05-22T12:00:00Z";
    const end = "2026-05-22T12:01:00Z";
    expect(getDuration(start, end)).toBe("1m 0s");
  });
});

// ─── formatNumber ────────────────────────────────────────────────

describe("formatNumber", () => {
  it("formats a number with commas", () => {
    expect(formatNumber(1234567)).toBe("1,234,567");
  });

  it("handles small numbers", () => {
    expect(formatNumber(42)).toBe("42");
  });

  it("handles zero", () => {
    expect(formatNumber(0)).toBe("0");
  });
});

// ─── formatToken ─────────────────────────────────────────────────

describe("formatToken", () => {
  it('returns raw number when < 1000', () => {
    expect(formatToken(0)).toBe("0");
    expect(formatToken(500)).toBe("500");
    expect(formatToken(999)).toBe("999");
  });

  it('appends K suffix for thousands', () => {
    expect(formatToken(1000)).toBe("1.0K");
    expect(formatToken(15234)).toBe("15.2K");
    expect(formatToken(999999)).toBe("1000.0K");
  });

  it('appends M suffix for millions', () => {
    expect(formatToken(1_000_000)).toBe("1.0M");
    expect(formatToken(2_500_000)).toBe("2.5M");
    expect(formatToken(999_999_999)).toBe("1000.0M");
  });

  it('appends B suffix for billions', () => {
    expect(formatToken(1_000_000_000)).toBe("1.0B");
    expect(formatToken(500_000_000_000)).toBe("500.0B");
  });

  it('appends T suffix for trillions', () => {
    expect(formatToken(1_000_000_000_000)).toBe("1.0T");
    expect(formatToken(9_000_000_000_000)).toBe("9.0T");
  });

  it('handles boundary at 999', () => {
    expect(formatToken(999)).toBe("999");
    expect(formatToken(1000)).toBe("1.0K");
  });
});

// ─── stripSystemReminderTags ─────────────────────────────────────

describe("stripSystemReminderTags", () => {
  it("removes a single system-reminder block", () => {
    const input = "hello<system-reminder>secret</system-reminder>world";
    expect(stripSystemReminderTags(input)).toBe("helloworld");
  });

  it("removes trailing whitespace after the closing tag", () => {
    const input =
      "hello<system-reminder>foo</system-reminder>\n\nworld";
    expect(stripSystemReminderTags(input)).toBe("helloworld");
  });

  it("removes trailing spaces after the closing tag", () => {
    const input = "hello<system-reminder>foo</system-reminder>   world";
    expect(stripSystemReminderTags(input)).toBe("helloworld");
  });

  it("handles unclosed tag gracefully", () => {
    const input = "text<system-reminder no close";
    expect(stripSystemReminderTags(input)).toBe("text<system-reminder no close");
  });

  it("returns same string when no tags present", () => {
    expect(stripSystemReminderTags("plain text")).toBe("plain text");
  });

  it("handles empty string", () => {
    expect(stripSystemReminderTags("")).toBe("");
  });

  it("removes multiple system-reminder blocks", () => {
    const input =
      "a<system-reminder>1</system-reminder>b<system-reminder>2</system-reminder>c";
    expect(stripSystemReminderTags(input)).toBe("abc");
  });

  it("handles tag with only whitespace content", () => {
    const input = "before<system-reminder>  </system-reminder>after";
    expect(stripSystemReminderTags(input)).toBe("beforeafter");
  });

  it("handles text that is entirely a system-reminder block", () => {
    expect(stripSystemReminderTags("<system-reminder>full</system-reminder>")).toBe("");
  });

  it("handles \\r\\n line endings after closing tag", () => {
    const input = "a<system-reminder>b</system-reminder>\r\nc";
    expect(stripSystemReminderTags(input)).toBe("ac");
  });
});

// ─── formatWorkspace ─────────────────────────────────────────────

describe("formatWorkspace", () => {
  it('replaces /home/username with ~/username', () => {
    expect(formatWorkspace("/home/user/projects/myapp")).toBe("~/projects/myapp");
  });

  it('replaces /Users/username with ~/username', () => {
    expect(formatWorkspace("/Users/john/Work/rust/tidev")).toBe("~/Work/rust/tidev");
  });

  it('returns path unchanged if not a home path', () => {
    expect(formatWorkspace("/var/www/html")).toBe("/var/www/html");
  });

  it('returns "-" for empty string', () => {
    expect(formatWorkspace("")).toBe("-");
  });

  it('returns path unchanged if home path has fewer than 3 parts', () => {
    expect(formatWorkspace("/Users")).toBe("/Users");
  });
});
