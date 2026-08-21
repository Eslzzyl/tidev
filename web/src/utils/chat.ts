export function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(date);
}

export function shortPath(value: string): string {
  if (!value) return "";
  const parts = value.split(/[\\/]/).filter(Boolean);
  return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : value;
}

export function formatThinkingLevel(value: string): string {
  const [, level = value] = value.split(":", 2);
  return level.charAt(0).toUpperCase() + level.slice(1);
}
