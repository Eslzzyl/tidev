import i18n from "../i18n";

export function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(i18n.language, { month: "short", day: "numeric" }).format(date);
}

export function shortPath(value: string): string {
  if (!value) return "";
  const parts = value.split(/[\\/]/).filter(Boolean);
  return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : value;
}

const THINKING_LEVEL_LABELS_ZH: Record<string, string> = {
  off: "关",
  minimal: "最低",
  low: "低",
  medium: "中",
  high: "高",
  xhigh: "极高",
  max: "最高",
};

export function formatThinkingLevel(value: string, locale = i18n.language): string {
  const [, level = value] = value.split(":", 2);
  const normalized = level.trim().toLowerCase();
  if (locale.startsWith("zh")) return THINKING_LEVEL_LABELS_ZH[normalized] ?? level;
  return level.charAt(0).toUpperCase() + level.slice(1);
}
