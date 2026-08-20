/**
 * Format a date string for display in the session list.
 */
export function formatSessionDate(dateStr: string): string {
  const date = new Date(dateStr);
  const now = new Date();
  const diff = now.getTime() - date.getTime();
  const days = Math.floor(diff / (1000 * 60 * 60 * 24));

  if (days === 0) {
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  } else if (days === 1) {
    return "Yesterday";
  } else if (days < 7) {
    return date.toLocaleDateString([], { weekday: "short" });
  } else {
    return date.toLocaleDateString([], { month: "short", day: "numeric" });
  }
}

/**
 * Format a date string for chat message display.
 */
export function formatTime(isoStr: string): string {
  const d = new Date(isoStr);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/**
 * Format a git commit date as "YYYY-MM-DD HH:MM:SS ±HHMM".
 */
export function formatGitDate(isoStr: string): string {
  const d = new Date(isoStr);
  const year = d.getFullYear();
  const month = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  const hours = String(d.getHours()).padStart(2, "0");
  const minutes = String(d.getMinutes()).padStart(2, "0");
  const seconds = String(d.getSeconds()).padStart(2, "0");

  const offset = -d.getTimezoneOffset();
  const offsetHours = Math.floor(Math.abs(offset) / 60);
  const offsetMins = Math.abs(offset) % 60;
  const sign = offset >= 0 ? "+" : "-";
  const tz = `${sign}${String(offsetHours).padStart(2, "0")}${String(offsetMins).padStart(2, "0")}`;

  return `${year}-${month}-${day} ${hours}:${minutes}:${seconds} ${tz}`;
}

/**
 * Compute duration string between two ISO timestamps.
 */
export function getDuration(createdAt: string, completedAt: string): string | null {
  const created = new Date(createdAt);
  const completed = new Date(completedAt);
  const diffMs = completed.getTime() - created.getTime();
  if (diffMs < 0) return null;
  const secs = diffMs / 1000;
  if (secs < 60) return `${secs.toFixed(1)}s`;
  const mins = Math.floor(secs / 60);
  const remainSecs = Math.floor(secs % 60);
  return `${mins}m ${remainSecs}s`;
}

/**
 * Format a number with commas.
 */
export function formatNumber(n: number): string {
  return n.toLocaleString();
}

/**
 * Format token count with unit suffix (K, M, B, T).
 */
export function formatToken(n: number): string {
  if (n < 1000) return n.toString();
  if (n < 1_000_000) return (n / 1000).toFixed(1) + "K";
  if (n < 1_000_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n < 1_000_000_000_000) return (n / 1_000_000_000).toFixed(1) + "B";
  return (n / 1_000_000_000_000).toFixed(1) + "T";
}

/**
 * Strip all `<system-reminder>…</system-reminder>` blocks from the
 * given text. These tags are injected into user-message content for LLM
 * prefix cache consistency and must not be visible in the UI.
 */
export function stripSystemReminderTags(text: string): string {
  let result = "";
  let rest = text;
  while (true) {
    const start = rest.indexOf("<system-reminder");
    if (start === -1) {
      result += rest;
      break;
    }
    result += rest.slice(0, start);
    const end = rest.slice(start).indexOf("</system-reminder>");
    if (end === -1) {
      result += rest.slice(start);
      break;
    }
    const afterClose = start + end + "</system-reminder>".length;
    rest = rest.slice(afterClose);
    while (rest.startsWith("\n") || rest.startsWith("\r") || rest.startsWith(" ")) {
      rest = rest.slice(1);
    }
  }
  return result;
}

/**
 * Format workspace path (replace home with ~, strip Windows \\?\\ prefix)
 */
export function formatWorkspace(path: string): string {
  if (!path) return "-";
  const cleaned = path.replace(/^\\\\\?\\/, "");
  const home = getHomeDir();
  if (home && cleaned.startsWith(home)) {
    return "~" + cleaned.slice(home.length);
  }
  return cleaned;
}

function getHomeDir(): string | null {
  try {
    const g = globalThis as unknown as { process?: { env?: Record<string, string> } };
    if (g.process?.env?.HOME) return g.process.env.HOME;
    if (g.process?.env?.USERPROFILE) return g.process.env.USERPROFILE;
    if (g.process?.env?.HOMEDRIVE && g.process?.env?.HOMEPATH) {
      return g.process.env.HOMEDRIVE + g.process.env.HOMEPATH;
    }
  } catch {
    // ignore
  }
  return null;
}
