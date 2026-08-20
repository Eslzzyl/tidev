/**
 * Simple hash-based router for tidev web.
 *
 * URL Schema:
 * - `#chat` or empty — Chat view (default)
 * - `#files` — Files view
 * - `#settings` — Settings view
 * - `#terminal` — Terminal view
 * - `#git` — Git view
 * - `#chat/session-id` — Chat with specific session
 */

export type MainTab = "chat" | "files" | "settings" | "terminal" | "git" | "stats";

export interface RouteState {
  tab: MainTab;
  sessionId: string | null;
}

const VALID_TABS: MainTab[] = ["chat", "files", "settings", "terminal", "git", "stats"];

function isMainTab(value: string): value is MainTab {
  return (VALID_TABS as readonly string[]).includes(value);
}

/**
 * Parse the current window location hash into a RouteState.
 */
export function parseRoute(): RouteState {
  const hash = window.location.hash.replace(/^#/, "");
  const parts = hash.split("/").filter(Boolean);

  const tabCandidate = parts[0] || "chat";
  const tab = isMainTab(tabCandidate) ? tabCandidate : "chat";
  const sessionId = parts[1] || null;

  return {
    tab,
    sessionId,
  };
}

/**
 * Update the URL hash to reflect the given route state.
 */
export function updateURL(route: RouteState): void {
  const parts: string[] = [route.tab];
  if (route.sessionId) {
    parts.push(route.sessionId);
  }
  const hash = parts.join("/");
  if (window.location.hash !== `#${hash}`) {
    window.location.hash = hash;
  }
}

/**
 * Build a URL string for the given route.
 */
export function buildURL(route: RouteState): string {
  const parts: string[] = [route.tab];
  if (route.sessionId) {
    parts.push(route.sessionId);
  }
  return `#${parts.join("/")}`;
}
