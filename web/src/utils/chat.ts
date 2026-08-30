import i18n from "../i18n";
import type { ThinkingLevelValue } from "../types/api";

export function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(i18n.language, { month: "short", day: "numeric" }).format(date);
}

export function formatSessionActivity(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  if (!sameDay) return formatDate(value);
  return new Intl.DateTimeFormat(i18n.language, { hour: "2-digit", minute: "2-digit" }).format(
    date,
  );
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

function serializeThinkingLevel(value: ThinkingLevelValue): string {
  if (typeof value === "string") return value;
  const [provider, level] = Object.entries(value)[0] ?? [];
  if (!provider) return "";
  return typeof level === "string" ? `${provider}:${level}` : "";
}

export function isThinkingLevelEnabled(value: ThinkingLevelValue): boolean {
  const serialized = serializeThinkingLevel(value);
  const [, level = serialized] = serialized.split(":", 2);
  return !["", "none", "off"].includes(level.trim().toLowerCase());
}

export function formatThinkingLevel(value: ThinkingLevelValue, locale = i18n.language): string {
  const serialized = serializeThinkingLevel(value);
  const [, level = serialized] = serialized.split(":", 2);
  const normalized = level
    .trim()
    .toLowerCase()
    .replace(/^x[-_]high$/, "xhigh");
  if (locale.startsWith("zh")) return THINKING_LEVEL_LABELS_ZH[normalized] ?? level;
  return level.charAt(0).toUpperCase() + level.slice(1);
}
