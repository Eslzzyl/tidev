import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { queryKeys } from "./queryKeys";

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
    queryFn: api.listModels,
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

export function useSubagentConfig() {
  return useQuery({
    queryKey: queryKeys.subagentConfig,
    queryFn: api.getSubagentConfig,
    staleTime: 60_000,
  });
}

export function useSetSubagentConfig() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: api.setSubagentConfig,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.subagentConfig });
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
    mutationFn: ({ id, data }: { id: string; data: Parameters<typeof api.connectProvider>[1] }) =>
      api.connectProvider(id, data),
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

// ── MCP Servers ───────────────────────────────────────────────────────────

export function useMcpServers() {
  return useQuery({
    queryKey: queryKeys.mcpServers,
    queryFn: api.listMcpServers,
    staleTime: 5_000,
    refetchInterval: 5_000,
  });
}

export function useUpsertMcpServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: api.upsertMcpServer,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers });
      queryClient.invalidateQueries({ queryKey: queryKeys.tools });
    },
  });
}

export function useDeleteMcpServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => api.deleteMcpServer(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers });
      queryClient.invalidateQueries({ queryKey: queryKeys.tools });
    },
  });
}

export function useConnectMcpServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => api.connectMcpServer(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers });
      queryClient.invalidateQueries({ queryKey: queryKeys.tools });
    },
  });
}

export function useDisconnectMcpServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => api.disconnectMcpServer(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers });
      queryClient.invalidateQueries({ queryKey: queryKeys.tools });
    },
  });
}

export function useRefreshMcpServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => api.refreshMcpServer(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers });
      queryClient.invalidateQueries({ queryKey: queryKeys.tools });
    },
  });
}

export function useSkillsQuery() {
  return useQuery({
    queryKey: queryKeys.skills,
    queryFn: api.listSkills,
    staleTime: 10_000,
  });
}

export function useSkillDetailQuery(name: string | null) {
  return useQuery({
    queryKey: queryKeys.skill(name ?? ""),
    queryFn: () => (name ? api.getSkill(name) : Promise.reject(new Error("No skill name"))),
    enabled: !!name,
    staleTime: 30_000,
  });
}

export function useSkillFileQuery(name: string | null, path?: string) {
  return useQuery({
    queryKey: queryKeys.skillFile(name ?? "", path),
    queryFn: () =>
      name ? api.getSkillFile(name, path) : Promise.reject(new Error("No skill name")),
    enabled: !!name && !!path,
    staleTime: 30_000,
  });
}
