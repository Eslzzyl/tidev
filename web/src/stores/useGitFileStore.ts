import { create } from "zustand";
import { api } from "../api/client";
import { queryClient } from "../lib/queryClient";

export interface GitFileStatus {
  status: string;
  staged: boolean;
}

/** Aggregated status for display (for both files and propagated directories) */
export interface GitDisplayStatus {
  /** Whether there are any changes (unstaged or staged or untracked) */
  hasChanges: boolean;
  /** Has unstaged changes */
  hasUnstaged: boolean;
  /** Has staged changes */
  hasStaged: boolean;
  /** Is untracked */
  isUntracked: boolean;
  /** The underlying raw status character (for tooltip) */
  rawStatus: string;
}

interface GitFileStore {
  /** Path → Git status mapping (files only, from git status) */
  statusMap: Record<string, GitFileStatus>;
  /** Path → Display status (files + directories, propagated) */
  displayMap: Record<string, GitDisplayStatus>;
  /** Whether the data is being loaded */
  loading: boolean;
  /** Error message if fetch failed */
  error: string | null;
  /** Current branch name */
  branch: string | null;
  /** Fetch git status from backend */
  refresh: () => Promise<void>;
}

/**
 * Propagate file git statuses up the directory tree.
 * For example, if src/main.rs is modified, both src/ and the root
 * will also show as having changes.
 */
function propagateStatuses(
  fileMap: Record<string, GitFileStatus>,
): Record<string, GitDisplayStatus> {
  const result: Record<string, GitDisplayStatus> = {};

  for (const [filePath, status] of Object.entries(fileMap)) {
    // Add the file itself
    addOrMerge(result, filePath, status);

    // Walk up the directory tree and propagate
    const parts = filePath.split("/");
    for (let i = parts.length - 1; i > 0; i--) {
      const dirPath = parts.slice(0, i).join("/");
      addOrMerge(result, dirPath, status);
    }

    // Also add root level (empty string represents root)
    addOrMerge(result, "", status);
  }

  return result;
}

function addOrMerge(map: Record<string, GitDisplayStatus>, path: string, status: GitFileStatus) {
  const existing = map[path];
  const isUntracked = status.status === "?" || status.status === "!";
  const rawStatus = status.status[0]?.toUpperCase() || "?";

  if (!existing) {
    map[path] = {
      hasChanges: true,
      hasUnstaged: !status.staged,
      hasStaged: status.staged,
      isUntracked,
      rawStatus,
    };
  } else {
    // Merge: unstaged takes priority over staged
    map[path] = {
      hasChanges: true,
      hasUnstaged: existing.hasUnstaged || !status.staged,
      hasStaged: existing.hasStaged || status.staged,
      isUntracked: existing.isUntracked || isUntracked,
      rawStatus: existing.rawStatus,
    };
  }
}

export const useGitFileStore = create<GitFileStore>((set) => ({
  statusMap: {},
  displayMap: {},
  loading: false,
  error: null,
  branch: null,

  refresh: async () => {
    set({ loading: true, error: null });
    try {
      const result = await queryClient.fetchQuery({
        queryKey: ["git", "status"],
        queryFn: api.gitStatus,
      });
      const fileMap: Record<string, GitFileStatus> = {};
      for (const file of result.files) {
        fileMap[file.path] = { status: file.status, staged: file.staged };
      }
      const displayMap = propagateStatuses(fileMap);
      set({
        statusMap: fileMap,
        displayMap,
        branch: result.branch,
        loading: false,
      });
    } catch (err) {
      set({
        loading: false,
        error: err instanceof Error ? err.message : "Failed to get git status",
      });
    }
  },
}));
