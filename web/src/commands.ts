/** Slash command definitions for the web frontend */

export type CommandAction =
  | "message"
  | "undo"
  | "new"
  | "init"
  | "redo"
  | "rename"
  | "skills"
  | "connect"
  | "compact"
  | "fork"
  | "shell";

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
    usage: "/init [focus or constraints]",
    action: "init",
  },
  {
    name: "rename",
    aliases: ["title"],
    description: "Rename the current session",
    usage: "/rename <title>",
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
  {
    name: "connect",
    aliases: [],
    description: "Connect to LLM providers",
    usage: "/connect",
    action: "connect",
  },
  {
    name: "compact",
    aliases: [],
    description: "Compact conversation context",
    usage: "/compact",
    action: "compact",
  },
  {
    name: "fork",
    aliases: [],
    description: "Fork session from a message",
    usage: "/fork",
    action: "fork",
  },
  {
    name: "shell",
    aliases: ["bash", "!"],
    description: "Run a shell command",
    usage: "/shell <command>  or  !<command>",
    action: "shell",
  },
];

function score(spec: CommandSpec, query: string): number | null {
  const name = spec.name.toLowerCase();
  const aliasMatches = spec.aliases.map((a) => a.toLowerCase());

  let s: number;

  if (query === "") {
    s = 1_000;
  } else if (name === query) {
    s = 10_000;
  } else if (name.startsWith(query)) {
    s = 8_000 - (name.length - query.length) * 10;
  } else if (aliasMatches.some((alias) => alias === query)) {
    s = 9_500;
  } else if (aliasMatches.some((alias) => alias.startsWith(query))) {
    s = 7_500;
  } else {
    const position = name.indexOf(query);
    if (position !== -1) {
      s = 4_500 - position * 20;
    } else if (aliasMatches.some((alias) => alias.includes(query))) {
      s = 3_500;
    } else {
      return null;
    }
  }

  return s;
}

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

export function commandFragment(input: string): string | null {
  const trimmed = input.trimStart();
  if (!trimmed.startsWith("/")) return null;
  const body = trimmed.slice(1);
  if (body.includes(" ")) return null;
  return body;
}

export function isSlashCommand(input: string): boolean {
  return commandFragment(input) !== null || isShellBang(input);
}

export function isShellBang(input: string): boolean {
  return input.trimStart().startsWith("!");
}

export function parseSlashCommand(input: string): { command: string; args: string } | null {
  const trimmed = input.trim();
  if (trimmed.startsWith("!")) {
    return { command: "shell", args: trimmed.slice(1).trim() };
  }
  if (!trimmed.startsWith("/")) return null;
  const withoutSlash = trimmed.slice(1);
  const spaceIdx = withoutSlash.indexOf(" ");
  if (spaceIdx === -1) {
    return { command: withoutSlash.toLowerCase(), args: "" };
  }
  return {
    command: withoutSlash.slice(0, spaceIdx).toLowerCase(),
    args: withoutSlash.slice(spaceIdx + 1).trim(),
  };
}
