import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { queryKeys } from "./queryKeys";

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
