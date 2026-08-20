import { QueryClient } from "@tanstack/react-query";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      /** Data is considered fresh for 30s — no refetch within that window. */
      staleTime: 30 * 1000,
      /** Keep unused data in cache for 5 minutes. */
      gcTime: 5 * 60 * 1000,
      /** Retry twice on failure, with exponential backoff capped at 10s. */
      retry: 2,
      retryDelay: (attempt) => Math.min(1000 * 2 ** attempt, 10000),
      /** Refetch when the user switches back to the tab (stale data only). */
      refetchOnWindowFocus: true,
    },
  },
});
