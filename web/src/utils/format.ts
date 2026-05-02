/**
 * Format a date string for display in the session list.
 */
export function formatSessionDate(dateStr: string): string {
  const date = new Date(dateStr);
  const now = new Date();
  const diff = now.getTime() - date.getTime();
  const days = Math.floor(diff / (1000 * 60 * 60 * 24));

  if (days === 0) {
    return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  } else if (days === 1) {
    return 'Yesterday';
  } else if (days < 7) {
    return date.toLocaleDateString([], { weekday: 'short' });
  } else {
    return date.toLocaleDateString([], { month: 'short', day: 'numeric' });
  }
}

/**
 * Format a timestamp for chat message display.
 */
export function formatTime(isoStr: string): string {
  const d = new Date(isoStr);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
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
  if (n < 1_000_000) return (n / 1000).toFixed(1) + 'K';
  if (n < 1_000_000_000) return (n / 1_000_000).toFixed(1) + 'M';
  if (n < 1_000_000_000_000) return (n / 1_000_000_000).toFixed(1) + 'B';
  return (n / 1_000_000_000_000).toFixed(1) + 'T';
}

/**
 * Format workspace path (replace home with ~)
 */
export function formatWorkspace(path: string): string {
  if (!path) return '-';
  if (path.startsWith('/home/') || path.startsWith('/Users/')) {
    const parts = path.split('/');
    if (parts.length >= 3) {
      return '~/' + parts.slice(3).join('/');
    }
  }
  return path;
}
