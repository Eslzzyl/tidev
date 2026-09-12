import i18n from "../../../i18n";

// ── Color palettes ───────────────────────────────────────────────────────

export const LIGHT_COLORS = [
  "#2563eb",
  "#16a34a",
  "#d97706",
  "#dc2626",
  "#7c3aed",
  "#0891b2",
  "#ca8a04",
  "#be185d",
];

export const DARK_COLORS = [
  "#60a5fa",
  "#4ade80",
  "#fbbf24",
  "#f87171",
  "#a78bfa",
  "#22d3ee",
  "#fde68a",
  "#f9a8d4",
];

export const CHART_COLORS = ["#2563eb", "#16a34a", "#d97706", "#dc2626", "#7c3aed"];

// ── Helper functions ─────────────────────────────────────────────────────

export function formatNumber(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toLocaleString(i18n.language);
}

export function formatRequestSizeRange(lowerBound: number, upperBound: number | null): string {
  if (upperBound === null) return `${formatNumber(lowerBound)}+`;
  return `${formatNumber(lowerBound)}–${formatNumber(upperBound)}`;
}

export function formatTokenBucket(granularity: string, bucket: string): string {
  const d = new Date(bucket);
  switch (granularity) {
    case "hour":
      return d.toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" });
    case "day":
      return d.toLocaleDateString(i18n.language, { month: "short", day: "numeric" });
    case "week":
      return `W${getWeekNumber(d)}`;
    case "month":
      return d.toLocaleDateString(i18n.language, { month: "short", year: "2-digit" });
    default:
      return bucket;
  }
}

function getWeekNumber(d: Date): number {
  const startOfYear = new Date(d.getFullYear(), 0, 1);
  const diff = d.getTime() - startOfYear.getTime();
  return Math.ceil((diff / 86400000 + startOfYear.getDay() + 1) / 7);
}

export function formatDate(d: string | null): string {
  if (!d) return "—";
  return new Date(d).toLocaleDateString(i18n.language, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export type Granularity = "hour" | "day" | "week" | "month";
export type StatsRange = "24h" | "7d" | "30d" | "all";

export const RANGE_OPTIONS: { value: StatsRange; label: string; durationMs?: number }[] = [
  { value: "24h", label: "24h", durationMs: 24 * 60 * 60 * 1000 },
  { value: "7d", label: "7d", durationMs: 7 * 24 * 60 * 60 * 1000 },
  { value: "30d", label: "30d", durationMs: 30 * 24 * 60 * 60 * 1000 },
  { value: "all", label: "All" },
];
