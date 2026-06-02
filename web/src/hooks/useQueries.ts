import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";

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

  // Models / Tools / Skills
  models: ["models"] as const,
  tools: ["tools"] as const,
  skills: ["skills"] as const,

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
  statsSummary: ["stats", "summary"] as const,
  statsTimeSeries: (granularity?: string) => ["stats", "timeseries", granularity] as const,
  statsModels: ["stats", "models"] as const,
  statsProviders: ["stats", "providers"] as const,
  statsSessions: (limit?: number, offset?: number) =>
    ["stats", "sessions", limit, offset] as const,

  // Filesystem
  fsList: (path?: string) => ["fs", "list", path ?? ""] as const,
  fsRead: (path: string) => ["fs", "read", path] as const,

  // File search
  fileSearch: (query: string) => ["files", "search", query] as const,

  // Terminal
  terminalList: ["terminal", "list"] as const,
  terminalShells: ["terminal", "shells"] as const,
} as const;

// ── Sessions ───────────────────────────────────────────────────────────────

export function useSessions() {
  return useQuery({
    queryKey: queryKeys.sessions,
    queryFn: async () => {
      const { sessions } = await api.listSessions();
      return sessions;
    },
    staleTime: 30_000,
  });
}

export function useSession(id: string | null) {
  return useQuery({
    queryKey: queryKeys.session(id ?? ""),
    queryFn: () => api.getSession(id!),
    enabled: !!id,
    staleTime: 30_000,
  });
}

export function useSessionMessages(id: string | null) {
  return useQuery({
    queryKey: queryKeys.sessionMessages(id ?? ""),
    queryFn: async () => {
      const { messages, todos } = await api.listMessages(id!);
      return { messages, todos: todos ?? [] };
    },
    enabled: !!id,
    staleTime: 30_000,
  });
}

export function useCreateSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: api.createSession,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
    },
  });
}

export function useDeleteSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.deleteSession(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
    },
  });
}

export function useRenameSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) =>
      api.renameSession(id, title),
    onSuccess: (_data, { id }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
      queryClient.invalidateQueries({ queryKey: queryKeys.session(id) });
    },
  });
}

export function useForkSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      sessionId,
      messageId,
      title,
    }: {
      sessionId: string;
      messageId: string;
      title?: string;
    }) => api.forkSession(sessionId, messageId, title),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
    },
  });
}

export function useRevertToMessage() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      sessionId,
      messageId,
    }: {
      sessionId: string;
      messageId: string;
    }) => api.revertToMessage(sessionId, messageId),
    onSuccess: (_data, { sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessionMessages(sessionId),
      });
    },
  });
}

export function useRedoSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (sessionId: string) => api.redoSession(sessionId),
    onSuccess: (_data, sessionId) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessionMessages(sessionId),
      });
    },
  });
}

export function useCompactSession() {
  return useMutation({
    mutationFn: (sessionId: string) => api.compactSession(sessionId),
  });
}

export function useAbortRequest() {
  return useMutation({
    mutationFn: ({
      sessionId,
      requestId,
    }: {
      sessionId: string;
      requestId: number;
    }) => api.abortRequest(sessionId, { request_id: requestId }),
  });
}

export function useSendMessage() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      sessionId,
      data,
    }: {
      sessionId: string;
      data: Parameters<typeof api.sendMessage>[1];
    }) => api.sendMessage(sessionId, data),
    onSuccess: (_data, { sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.sessionMessages(sessionId),
      });
    },
  });
}

export function useSendShellCommand() {
  return useMutation({
    mutationFn: ({
      sessionId,
      command,
    }: {
      sessionId: string;
      command: string;
    }) => api.sendShellCommand(sessionId, command),
  });
}

// ── Workspace ──────────────────────────────────────────────────────────────

export function useWorkspace() {
  return useQuery({
    queryKey: queryKeys.workspace,
    queryFn: api.getWorkspace,
    staleTime: 60_000,
  });
}

// ── Init ───────────────────────────────────────────────────────────────────

export function useInitPrompt() {
  return useQuery({
    queryKey: queryKeys.initPrompt,
    queryFn: async () => {
      const { prompt } = await api.getInitPrompt();
      return prompt;
    },
    staleTime: Infinity,
  });
}

// ── Models / Tools / Skills ────────────────────────────────────────────────

export function useModels() {
  return useQuery({
    queryKey: queryKeys.models,
    queryFn: async () => {
      const { models } = await api.listModels();
      return models;
    },
    staleTime: 60_000,
  });
}

export function useTools() {
  return useQuery({
    queryKey: queryKeys.tools,
    queryFn: async () => {
      const { tools } = await api.listTools();
      return tools;
    },
    staleTime: 60_000,
  });
}

export function useSkills() {
  return useQuery({
    queryKey: queryKeys.skills,
    queryFn: async () => {
      const { skills } = await api.listSkills();
      return skills;
    },
    staleTime: 60_000,
  });
}

// ── Config ─────────────────────────────────────────────────────────────────

export function useDefaultModel() {
  return useQuery({
    queryKey: queryKeys.defaultModel,
    queryFn: api.getDefaultModel,
    staleTime: 60_000,
  });
}

export function useSetDefaultModel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: api.setDefaultModel,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.defaultModel });
    },
  });
}

export function useAgentModels() {
  return useQuery({
    queryKey: queryKeys.agentModels,
    queryFn: api.getAgentModels,
    staleTime: 60_000,
  });
}

export function useSetAgentModel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: api.setAgentModel,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agentModels });
    },
  });
}

export function useMemoryModel() {
  return useQuery({
    queryKey: queryKeys.memoryModel,
    queryFn: api.getMemoryModel,
    staleTime: 60_000,
  });
}

export function useSetMemoryModel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: api.setMemoryModel,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.memoryModel });
    },
  });
}

export function useTerminalShellConfig() {
  return useQuery({
    queryKey: queryKeys.terminalShellConfig,
    queryFn: api.getTerminalShellConfig,
    staleTime: 60_000,
  });
}

export function useSetTerminalShellConfig() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (shell: string) => api.setTerminalShellConfig(shell),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.terminalShellConfig });
    },
  });
}

// ── Providers ──────────────────────────────────────────────────────────────

export function useProviders() {
  return useQuery({
    queryKey: queryKeys.providers,
    queryFn: async () => {
      const { providers } = await api.listProviders();
      return providers;
    },
    staleTime: 60_000,
  });
}

export function useConnectProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      id,
      data,
    }: {
      id: string;
      data: Parameters<typeof api.connectProvider>[1];
    }) => api.connectProvider(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers });
    },
  });
}

export function useDisconnectProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.disconnectProvider(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers });
    },
  });
}

export function useCreateProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: api.createProvider,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers });
    },
  });
}

export function useDeleteProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.deleteProvider(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers });
    },
  });
}

// ── Git ────────────────────────────────────────────────────────────────────

export function useGitStatus() {
  return useQuery({
    queryKey: queryKeys.gitStatus,
    queryFn: api.gitStatus,
    staleTime: 15_000,
  });
}

export function useGitBranches(showSubmodules?: boolean) {
  return useQuery({
    queryKey: queryKeys.gitBranches(showSubmodules),
    queryFn: () => api.gitBranches(showSubmodules),
    staleTime: 30_000,
  });
}

export function useGitGraph(count?: number) {
  return useQuery({
    queryKey: queryKeys.gitGraph(count),
    queryFn: () => api.gitGraph(count),
    staleTime: 30_000,
  });
}

export function useGitShowCommit(sha: string | null) {
  return useQuery({
    queryKey: queryKeys.gitShowCommit(sha ?? ""),
    queryFn: () => api.gitShowCommit(sha!),
    enabled: !!sha,
  });
}

export function useGitShowFileDiff(sha: string | null, path: string | null) {
  return useQuery({
    queryKey: queryKeys.gitShowFileDiff(sha ?? "", path ?? ""),
    queryFn: () => api.gitShowFileDiff(sha!, path!),
    enabled: !!sha && !!path,
  });
}

export function useGitShowAllDiffs(sha: string | null) {
  return useQuery({
    queryKey: queryKeys.gitShowAllDiffs(sha ?? ""),
    queryFn: () => api.gitShowAllDiffs(sha!),
    enabled: !!sha,
  });
}

export function useGitDiffFile(path: string | null, staged?: boolean) {
  return useQuery({
    queryKey: queryKeys.gitDiffFile(path ?? "", staged),
    queryFn: () => api.gitDiffFile(path!, staged),
    enabled: !!path,
  });
}

export function useGitCommit() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (message: string) => api.gitCommit(message),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.gitStatus });
      queryClient.invalidateQueries({ queryKey: ["git"] }); // broad invalidation
    },
  });
}

export function useGitPush() {
  return useMutation({
    mutationFn: ({
      remote,
      branch,
      force,
    }: {
      remote?: string;
      branch?: string;
      force?: boolean;
    }) => api.gitPush(remote, branch, force),
  });
}

export function useGitPull() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ remote, branch }: { remote?: string; branch?: string }) =>
      api.gitPull(remote, branch),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["git"] });
    },
  });
}

export function useGitStash() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (message?: string) => api.gitStash(message),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["git"] });
    },
  });
}

export function useGitBranchCreate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, checkout }: { name: string; checkout?: boolean }) =>
      api.gitBranchCreate(name, checkout),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["git"] });
    },
  });
}

export function useGitBranchDelete() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => api.gitBranchDelete(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["git"] });
    },
  });
}

// ── Stats ──────────────────────────────────────────────────────────────────

export function useStatsSummary() {
  return useQuery({
    queryKey: queryKeys.statsSummary,
    queryFn: api.getStatsSummary,
    staleTime: 60_000,
  });
}

export function useStatsTimeSeries(granularity?: string) {
  return useQuery({
    queryKey: queryKeys.statsTimeSeries(granularity),
    queryFn: () => api.getStatsTimeSeries({ granularity }),
    staleTime: 60_000,
  });
}

export function useStatsModels() {
  return useQuery({
    queryKey: queryKeys.statsModels,
    queryFn: async () => {
      const { entries } = await api.getStatsModels();
      return entries;
    },
    staleTime: 60_000,
  });
}

export function useStatsProviders() {
  return useQuery({
    queryKey: queryKeys.statsProviders,
    queryFn: async () => {
      const { entries } = await api.getStatsProviders();
      return entries;
    },
    staleTime: 60_000,
  });
}

export function useStatsSessions(limit?: number, offset?: number) {
  return useQuery({
    queryKey: queryKeys.statsSessions(limit, offset),
    queryFn: () => api.getStatsSessions({ limit, offset }),
    staleTime: 60_000,
  });
}

// ── Filesystem ─────────────────────────────────────────────────────────────

export function useFsList(path?: string) {
  return useQuery({
    queryKey: queryKeys.fsList(path),
    queryFn: () => api.listDirectory(path),
    staleTime: 30_000,
  });
}

export function useFsRead(path: string | null) {
  return useQuery({
    queryKey: queryKeys.fsRead(path ?? ""),
    queryFn: () => api.readFile(path!),
    enabled: !!path,
    staleTime: 30_000,
  });
}

// ── File search ────────────────────────────────────────────────────────────

export function useFileSearch(query: string) {
  return useQuery({
    queryKey: queryKeys.fileSearch(query),
    queryFn: async () => {
      const { suggestions } = await api.searchFiles(query);
      return suggestions;
    },
    enabled: query.length > 0,
    staleTime: 30_000,
  });
}

// ── Terminal ───────────────────────────────────────────────────────────────

export function useTerminalList() {
  return useQuery({
    queryKey: queryKeys.terminalList,
    queryFn: api.listTerminals,
    staleTime: 10_000,
  });
}

export function useTerminalShells() {
  return useQuery({
    queryKey: queryKeys.terminalShells,
    queryFn: api.listTerminalShells,
    staleTime: 60_000,
  });
}

export function useStartTerminal() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      cols,
      rows,
      shell,
      label,
    }: {
      cols?: number;
      rows?: number;
      shell?: string;
      label?: string;
    }) => api.startTerminal(cols, rows, shell, label),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.terminalList });
    },
  });
}

export function useCloseTerminal() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (sessionId: string) => api.closeTerminal(sessionId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.terminalList });
    },
  });
}

export function useRenameTerminal() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      sessionId,
      label,
    }: {
      sessionId: string;
      label: string;
    }) => api.renameTerminal(sessionId, label),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.terminalList });
    },
  });
}
