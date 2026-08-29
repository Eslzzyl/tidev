import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { queryKeys } from "./queryKeys";

// ── Sessions ───────────────────────────────────────────────────────────────

export function useSessions() {
  return useQuery({
    queryKey: queryKeys.sessions,
    queryFn: () => api.listSessions(),
    staleTime: 30_000,
  });
}

export function useSession(id: string | null) {
  const sessionId = id?.trim() || "";
  return useQuery({
    queryKey: queryKeys.session(sessionId),
    queryFn: () => api.getSession(sessionId),
    enabled: !!sessionId,
    staleTime: 30_000,
  });
}

export function useSessionMessages(id: string | null) {
  const sessionId = id?.trim() || "";
  return useQuery({
    queryKey: queryKeys.sessionMessages(sessionId),
    queryFn: async () => {
      const [{ messages }, { todos }] = await Promise.all([
        api.listMessages(sessionId),
        api.getTodos(sessionId),
      ]);
      return { messages, todos };
    },
    enabled: !!sessionId,
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
    mutationFn: ({ id, title }: { id: string; title: string }) => api.renameSession(id, title),
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
    mutationFn: ({ sessionId, messageId }: { sessionId: string; messageId: string }) =>
      api.revertToMessage(sessionId, messageId),
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
    mutationFn: ({ sessionId, requestId }: { sessionId: string; requestId: number }) =>
      api.abortRequest(sessionId, { request_id: requestId }),
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
