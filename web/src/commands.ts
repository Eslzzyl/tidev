/** Slash command definitions for the web frontend */

export type CommandAction = "message" | "undo" | "new" | "init" | "redo" | "rename" | "skills";

export interface CommandSpec {
  name: string;
  aliases: string[];
  description: string;
  usage: string;
  action: CommandAction;
}

export interface CommandSuggestion {
  spec: CommandSpec;
  score: number;
}

/** All available slash commands */
export const COMMANDS: CommandSpec[] = [
  {
    name: "message",
    aliases: ["msg"],
    description: "Search current session messages",
    usage: "/message [query]",
    action: "message",
  },
  {
    name: "undo",
    aliases: [],
    description: "Revert the previous user message",
    usage: "/undo",
    action: "undo",
  },
  {
    name: "redo",
    aliases: [],
    description: "Move forward in undo history",
    usage: "/redo",
    action: "redo",
  },
  {
    name: "init",
    aliases: [],
    description: "Analyze project and create AGENTS.md",
    usage: "/init",
    action: "init",
  },
  {
    name: "rename",
    aliases: ["title"],
    description: "Rename the current session",
    usage: "/rename",
    action: "rename",
  },
  {
    name: "new",
    aliases: ["clear"],
    description: "Start a new conversation",
    usage: "/new",
    action: "new",
  },
  {
    name: "skills",
    aliases: ["skill"],
    description: "Browse and load available skills",
    usage: "/skills",
    action: "skills",
  },
];

/**
 * Score a command against a query (mirrors TUI scoring logic).
 * Returns a score or null if no match.
 */
function score(spec: CommandSpec, query: string): number | null {
  const name = spec.name.toLowerCase();
  const aliasMatches = spec.aliases.map((a) => a.toLowerCase());

  let score: number;

  if (query === "") {
    score = 1_000;
  } else if (name === query) {
    score = 10_000;
  } else if (name.startsWith(query)) {
    score = 8_000 - (name.length - query.length) * 10;
  } else if (aliasMatches.some((alias) => alias === query)) {
    score = 9_500;
  } else if (aliasMatches.some((alias) => alias.startsWith(query))) {
    score = 7_500;
  } else {
    const position = name.indexOf(query);
    if (position !== -1) {
      score = 4_500 - position * 20;
    } else if (aliasMatches.some((alias) => alias.includes(query))) {
      score = 3_500;
    } else {
      return null;
    }
  }

  return score;
}

/**
 * Generate suggestions for a query string (text after "/").
 * Returns sorted suggestions by score descending, then alphabetically.
 */
export function getSuggestions(query: string): CommandSuggestion[] {
  const normalized = query.trim().toLowerCase();

  const candidates = COMMANDS.map((spec) => {
    const s = score(spec, normalized);
    return s !== null ? { spec, score: s } : null;
  }).filter((c): c is CommandSuggestion => c !== null);

  candidates.sort((a, b) => {
    if (b.score !== a.score) return b.score - a.score;
    return a.spec.name.localeCompare(b.spec.name);
  });

  return candidates;
}

/**
 * Check if input starts with a slash command fragment (no whitespace after /).
 * Returns the fragment after "/", or null if not in command mode.
 */
export function commandFragment(input: string): string | null {
  const trimmed = input.trimStart();
  if (!trimmed.startsWith("/")) return null;
  const body = trimmed.slice(1);
  if (body.includes(" ")) return null;
  return body;
}
