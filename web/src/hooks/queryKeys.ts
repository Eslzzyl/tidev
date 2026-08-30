// ── Query keys ─────────────────────────────────────────────────────────────

export const queryKeys = {
  // Session
  sessions: ["sessions"] as const,
  session: (id: string) => ["session", id] as const,
  sessionMessages: (id: string) => ["session", id, "messages"] as const,

  // Workspace
  workspace: ["workspace"] as const,

  // Init
  initPrompt: ["init", "prompt"] as const,

  // Models / Tools / Skills / MCP
  models: ["models"] as const,
  tools: ["tools"] as const,
  skills: ["skills"] as const,
  mcpServers: ["mcp", "servers"] as const,

  // Config
  defaultModel: ["config", "default-model"] as const,
  agentModels: ["config", "agent-models"] as const,
  memoryModel: ["config", "memory-model"] as const,
  terminalShellConfig: ["config", "terminal-shell"] as const,
  modelThinkingLevel: (providerId: string, modelId: string) =>
    ["config", "model-thinking-level", providerId, modelId] as const,

  // Providers
  providers: ["providers"] as const,

  // Git
  gitStatus: ["git", "status"] as const,
  gitBranches: (showSubmodules?: boolean) => ["git", "branches", showSubmodules] as const,
  gitGraph: (count?: number) => ["git", "graph", count] as const,
  gitShowCommit: (sha: string) => ["git", "show", sha] as const,
  gitShowFileDiff: (sha: string, path: string) => ["git", "diff", sha, path] as const,
  gitShowAllDiffs: (sha: string) => ["git", "diffs", sha] as const,
  gitDiffFile: (path: string, staged?: boolean) => ["git", "file-diff", path, staged] as const,

  // Stats
  statsOverview: (
    granularity?: string,
    start?: string,
    end?: string,
    limit?: number,
    offset?: number,
  ) => ["stats", "overview", granularity, start, end, limit, offset] as const,
  statsSummary: (start?: string, end?: string) => ["stats", "summary", start, end] as const,
  statsTimeSeries: (granularity?: string, start?: string, end?: string) =>
    ["stats", "timeseries", granularity, start, end] as const,
  statsModels: (start?: string, end?: string) => ["stats", "models", start, end] as const,
  statsProviders: (start?: string, end?: string) => ["stats", "providers", start, end] as const,
  statsSessions: (limit?: number, offset?: number, start?: string, end?: string) =>
    ["stats", "sessions", limit, offset, start, end] as const,

  // Filesystem
  fsList: (path?: string) => ["fs", "list", path ?? ""] as const,
  fsRead: (path: string) => ["fs", "read", path] as const,

  // File search
  fileSearch: (query: string) => ["files", "search", query] as const,

  // Terminal
  terminalList: ["terminal", "list"] as const,
  terminalShells: ["terminal", "shells"] as const,
} as const;
