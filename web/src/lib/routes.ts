export type MainFeature = "chat" | "files" | "terminal" | "git" | "stats";

export type GitSubTab = "changes" | "history" | "branches";

export type SettingsCategory =
  | "appearance"
  | "editor"
  | "interaction"
  | "terminal"
  | "security"
  | "mcp"
  | "skills"
  | "about";

export const routes = {
  root: () => "/",
  chat: (sessionId?: string | null) => (sessionId ? `/chat/${sessionId}` : "/chat"),
  files: (filePath?: string | null) =>
    filePath ? `/files?path=${encodeURIComponent(filePath)}` : "/files",
  terminal: (tabId?: string | null) => (tabId ? `/terminal/${tabId}` : "/terminal"),
  git: (tab: GitSubTab = "changes", sha?: string | null) => {
    if (tab === "history" && sha) {
      return `/git/history/${sha}`;
    }
    return `/git/${tab}`;
  },
  stats: (range?: string, granularity?: string) => {
    const params = new URLSearchParams();
    if (range) params.set("range", range);
    if (granularity) params.set("granularity", granularity);
    const query = params.toString();
    return query ? `/stats?${query}` : "/stats";
  },
  settings: (category?: SettingsCategory | string | null) =>
    category ? `/settings/${category}` : "/settings",
};

/**
 * Determine the active main navigation feature from a pathname.
 */
export function getActiveFeature(pathname: string): MainFeature {
  if (pathname.startsWith("/files")) return "files";
  if (pathname.startsWith("/terminal")) return "terminal";
  if (pathname.startsWith("/git")) return "git";
  if (pathname.startsWith("/stats")) return "stats";
  return "chat";
}
