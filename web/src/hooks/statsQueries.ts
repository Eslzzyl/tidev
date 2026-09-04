import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { queryKeys } from "./queryKeys";

// ── Stats ──────────────────────────────────────────────────────────────────

type StatsQueryParams = { start?: string; end?: string };

export function useStatsOverview(
  granularity?: string,
  params?: StatsQueryParams,
  limit?: number,
  offset?: number,
) {
  return useQuery({
    queryKey: queryKeys.statsOverview(granularity, params?.start, params?.end, limit, offset),
    queryFn: () => api.getStatsOverview({ granularity, ...params, limit, offset }),
    staleTime: 60_000,
  });
}

export function useStatsSummary(params?: StatsQueryParams) {
  return useQuery({
    queryKey: queryKeys.statsSummary(params?.start, params?.end),
    queryFn: () => api.getStatsSummary(params),
    staleTime: 60_000,
  });
}

export function useStatsTimeSeries(granularity?: string, params?: StatsQueryParams) {
  return useQuery({
    queryKey: queryKeys.statsTimeSeries(granularity, params?.start, params?.end),
    queryFn: () => api.getStatsTimeSeries({ granularity, ...params }),
    staleTime: 60_000,
  });
}

export function useStatsActivity() {
  return useQuery({
    queryKey: queryKeys.statsActivity,
    queryFn: api.getStatsActivity,
    staleTime: 60_000,
  });
}

export function useStatsInsights(granularity?: string, params?: StatsQueryParams) {
  return useQuery({
    queryKey: queryKeys.statsInsights(granularity, params?.start, params?.end),
    queryFn: () => api.getStatsInsights({ granularity, ...params }),
    staleTime: 60_000,
  });
}

export function useStatsModels(params?: StatsQueryParams) {
  return useQuery({
    queryKey: queryKeys.statsModels(params?.start, params?.end),
    queryFn: async () => {
      const { entries } = await api.getStatsModels(params);
      return entries;
    },
    staleTime: 60_000,
  });
}

export function useStatsProviders(params?: StatsQueryParams) {
  return useQuery({
    queryKey: queryKeys.statsProviders(params?.start, params?.end),
    queryFn: async () => {
      const { entries } = await api.getStatsProviders(params);
      return entries;
    },
    staleTime: 60_000,
  });
}

export function useStatsSessions(limit?: number, offset?: number, params?: StatsQueryParams) {
  return useQuery({
    queryKey: queryKeys.statsSessions(limit, offset, params?.start, params?.end),
    queryFn: () => api.getStatsSessions({ limit, offset, ...params }),
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
      const { files } = await api.searchFiles(query);
      return files;
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
    mutationFn: ({ sessionId, label }: { sessionId: string; label: string }) =>
      api.renameTerminal(sessionId, label),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.terminalList });
    },
  });
}
