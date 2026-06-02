import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";

// ── Query keys ─────────────────────────────────────────────────────────────

export const queryKeys = {
  sessions: ["sessions"] as const,
  session: (id: string) => ["session", id] as const,
  sessionMessages: (id: string) => ["session", id, "messages"] as const,
  workspace: ["workspace"] as const,
} as const;

// ── Sessions list ───────────────────────────────────────────────────────────

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

// ── Single session ──────────────────────────────────────────────────────────

export function useSession(id: string | null) {
  return useQuery({
    queryKey: queryKeys.session(id ?? ""),
    queryFn: () => api.getSession(id!),
    enabled: !!id,
    staleTime: 30_000,
  });
}

// ── Session messages ────────────────────────────────────────────────────────

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

// ── Workspace ───────────────────────────────────────────────────────────────

export function useWorkspace() {
  return useQuery({
    queryKey: queryKeys.workspace,
    queryFn: api.getWorkspace,
    staleTime: 60_000,
  });
}

// ── Mutations ───────────────────────────────────────────────────────────────

export function useRenameSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) =>
      api.renameSession(id, title),
    onSuccess: (_data, { id }) => {
      // Invalidate both the list and the individual session cache
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
      queryClient.invalidateQueries({ queryKey: queryKeys.session(id) });
    },
  });
}
