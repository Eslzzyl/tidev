import { useState, useEffect, useCallback } from "react";
import {
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
  ArrowUpFromLine,
  ArrowDownFromLine,
  Archive,
  RotateCcw,
  Plus,
  Trash2,
  Check,
  FileText,
  FilePlus,
  FileEdit,
  FileX,
  Loader2,
} from "lucide-react";
import { api } from "../../api/client";
import type {
  GitStatusResponse,
  GitBranchResponse,
  GitLogResponse,
} from "../../types/api";

type GitTab = "changes" | "history" | "branches";

export function GitView() {
  const [activeTab, setActiveTab] = useState<GitTab>("changes");
  const [status, setStatus] = useState<GitStatusResponse | null>(null);
  const [branches, setBranches] = useState<GitBranchResponse | null>(null);
  const [log, setLog] = useState<GitLogResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [commitMsg, setCommitMsg] = useState("");
  const [committing, setCommitting] = useState(false);
  const [commitResult, setCommitResult] = useState<string | null>(null);
  const [pushPullLoading, setPushPullLoading] = useState(false);
  const [stashLoading, setStashLoading] = useState(false);
  const [newBranchName, setNewBranchName] = useState("");
  const [creatingBranch, setCreatingBranch] = useState(false);

  const refreshStatus = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const [s, b, l] = await Promise.all([
        api.gitStatus(),
        api.gitBranches(),
        api.gitLog(20),
      ]);
      setStatus(s);
      setBranches(b);
      setLog(l);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load git data");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refreshStatus();
  }, [refreshStatus]);

  const handleCommit = async () => {
    if (!commitMsg.trim()) return;
    setCommitting(true);
    setCommitResult(null);
    try {
      const result = await api.gitCommit(commitMsg.trim());
      setCommitResult(result.message);
      setCommitMsg("");
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        `Error: ${err instanceof Error ? err.message : "Commit failed"}`,
      );
    } finally {
      setCommitting(false);
    }
  };

  const handlePush = async () => {
    setPushPullLoading(true);
    try {
      const result = await api.gitPush();
      setCommitResult(result.message);
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        `Push error: ${err instanceof Error ? err.message : "Push failed"}`,
      );
    } finally {
      setPushPullLoading(false);
    }
  };

  const handlePull = async () => {
    setPushPullLoading(true);
    try {
      const result = await api.gitPull();
      setCommitResult(result.message);
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        `Pull error: ${err instanceof Error ? err.message : "Pull failed"}`,
      );
    } finally {
      setPushPullLoading(false);
    }
  };

  const handleStash = async () => {
    setStashLoading(true);
    try {
      const result = await api.gitStash();
      setCommitResult(result.message);
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        `Stash error: ${err instanceof Error ? err.message : "Stash failed"}`,
      );
    } finally {
      setStashLoading(false);
    }
  };

  const handleCreateBranch = async () => {
    if (!newBranchName.trim()) return;
    setCreatingBranch(true);
    try {
      const result = await api.gitBranchCreate(newBranchName.trim(), true);
      setCommitResult(result.message);
      setNewBranchName("");
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        `Branch error: ${err instanceof Error ? err.message : "Branch creation failed"}`,
      );
    } finally {
      setCreatingBranch(false);
    }
  };

  const handleDeleteBranch = async (name: string) => {
    try {
      const result = await api.gitBranchDelete(name);
      setCommitResult(result.message);
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        `Delete error: ${err instanceof Error ? err.message : "Delete failed"}`,
      );
    }
  };

  const tabs: { id: GitTab; label: string; icon: React.ReactNode }[] = [
    {
      id: "changes",
      label: "Changes",
      icon: <FileEdit className="h-4 w-4" />,
    },
    {
      id: "history",
      label: "History",
      icon: <GitCommitHorizontal className="h-4 w-4" />,
    },
    {
      id: "branches",
      label: "Branches",
      icon: <GitBranch className="h-4 w-4" />,
    },
  ];

  if (error) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <div className="text-center">
          <GitBranch className="mx-auto mb-2 h-8 w-8 text-neutral-400" />
          <p className="text-sm text-neutral-500">{error}</p>
          <button
            onClick={refreshStatus}
            className="mt-3 rounded bg-neutral-200 px-3 py-1.5 text-xs font-medium text-neutral-700 hover:bg-neutral-300 dark:bg-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-700"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      {/* Branch info bar */}
      <div className="flex items-center justify-between border-b border-neutral-200 bg-neutral-50 px-4 py-2 dark:border-neutral-800 dark:bg-neutral-900">
        <div className="flex items-center gap-2">
          <GitBranch className="h-4 w-4 text-neutral-500" />
          <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
            {status?.branch || "..."}
          </span>
          {status && (
            <span className="text-xs text-neutral-500">
              {status.sha}
              {(status.ahead > 0 || status.behind > 0) &&
                ` · ↑${status.ahead} ↓${status.behind}`}
            </span>
          )}
        </div>

        <div className="flex items-center gap-1">
          <button
            onClick={handlePull}
            disabled={pushPullLoading}
            className="rounded p-1.5 text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
            title="Pull"
          >
            {pushPullLoading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <ArrowDownFromLine className="h-3.5 w-3.5" />
            )}
          </button>
          <button
            onClick={handlePush}
            disabled={pushPullLoading}
            className="rounded p-1.5 text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
            title="Push"
          >
            <ArrowUpFromLine className="h-3.5 w-3.5" />
          </button>
          <button
            onClick={handleStash}
            disabled={stashLoading}
            className="rounded p-1.5 text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
            title="Stash"
          >
            {stashLoading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Archive className="h-3.5 w-3.5" />
            )}
          </button>
          <button
            onClick={refreshStatus}
            disabled={loading}
            className="rounded p-1.5 text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
            title="Refresh"
          >
            <RotateCcw className={`h-3.5 w-3.5 ${loading ? "animate-spin" : ""}`} />
          </button>
        </div>
      </div>

      {/* Tab bar */}
      <div className="flex border-b border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-950">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`flex items-center gap-1.5 px-4 py-2 text-xs font-medium transition-colors ${
              activeTab === tab.id
                ? "border-b-2 border-neutral-900 text-neutral-900 dark:border-neutral-100 dark:text-neutral-100"
                : "text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300"
            }`}
          >
            {tab.icon}
            {tab.label}
          </button>
        ))}
      </div>

      {/* Result message */}
      {commitResult && (
        <div
          className={`px-4 py-1.5 text-xs ${
            commitResult.startsWith("Error") || commitResult.startsWith("Push error") || commitResult.startsWith("Pull error")
              ? "bg-red-50 text-red-700 dark:bg-red-950/30 dark:text-red-400"
              : "bg-green-50 text-green-700 dark:bg-green-950/30 dark:text-green-400"
          }`}
        >
          {commitResult}
          <button
            onClick={() => setCommitResult(null)}
            className="ml-2 underline"
          >
            Dismiss
          </button>
        </div>
      )}

      {/* Content */}
      <div className="flex-1 overflow-y-auto">
        {loading && !status ? (
          <div className="flex h-full items-center justify-center">
            <Loader2 className="h-5 w-5 animate-spin text-neutral-400" />
          </div>
        ) : activeTab === "changes" ? (
          <ChangesPanel
            status={status}
            commitMsg={commitMsg}
            onCommitMsgChange={setCommitMsg}
            onCommit={handleCommit}
            committing={committing}
          />
        ) : activeTab === "history" ? (
          <HistoryPanel log={log} />
        ) : (
          <BranchesPanel
            branches={branches}
            newBranchName={newBranchName}
            onNewBranchNameChange={setNewBranchName}
            onCreateBranch={handleCreateBranch}
            creatingBranch={creatingBranch}
            onDeleteBranch={handleDeleteBranch}
          />
        )}
      </div>
    </div>
  );
}

// ── Changes Panel ─────────────────────────────────────────────────────────

function ChangesPanel({
  status,
  commitMsg,
  onCommitMsgChange,
  onCommit,
  committing,
}: {
  status: GitStatusResponse | null;
  commitMsg: string;
  onCommitMsgChange: (msg: string) => void;
  onCommit: () => void;
  committing: boolean;
}) {
  const staged = status?.files.filter((f) => f.staged) || [];
  const unstaged = status?.files.filter((f) => !f.staged) || [];

  const fileIcon = (file: { status: string }) => {
    switch (file.status) {
      case "M":
        return <FileEdit className="h-3.5 w-3.5 text-yellow-600" />;
      case "A":
        return <FilePlus className="h-3.5 w-3.5 text-green-600" />;
      case "D":
        return <FileX className="h-3.5 w-3.5 text-red-600" />;
      case "?":
        return <FilePlus className="h-3.5 w-3.5 text-blue-600" />;
      default:
        return <FileText className="h-3.5 w-3.5 text-neutral-500" />;
    }
  };

  const statusLabel = (s: string) => {
    switch (s) {
      case "M":
        return "Modified";
      case "A":
        return "Added";
      case "D":
        return "Deleted";
      case "R":
        return "Renamed";
      case "?":
        return "Untracked";
      default:
        return s;
    }
  };

  return (
    <div className="p-4">
      {/* Commit input */}
      <div className="mb-4">
        <textarea
          value={commitMsg}
          onChange={(e) => onCommitMsgChange(e.target.value)}
          placeholder="Commit message"
          rows={2}
          className="w-full rounded border border-neutral-300 bg-white px-3 py-2 text-sm text-neutral-900 placeholder-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500"
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              if (commitMsg.trim() && !committing) onCommit();
            }
          }}
        />
        <button
          onClick={onCommit}
          disabled={!commitMsg.trim() || committing}
          className="mt-2 flex items-center gap-1.5 rounded bg-neutral-900 px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-neutral-800 disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
        >
          {committing ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Check className="h-3.5 w-3.5" />
          )}
          Commit {staged.length > 0 ? `(${staged.length} file(s))` : ""}
        </button>
      </div>

      {/* File lists */}
      {staged.length > 0 && (
        <FileList title="Staged" files={staged} icon={fileIcon} statusLabel={statusLabel} />
      )}
      {unstaged.length > 0 && (
        <FileList title="Changes" files={unstaged} icon={fileIcon} statusLabel={statusLabel} />
      )}
      {(!status || status.files.length === 0) && (
        <div className="py-8 text-center text-sm text-neutral-500">
          No changes in working tree
        </div>
      )}
    </div>
  );
}

function FileList({
  title,
  files,
  icon,
  statusLabel,
}: {
  title: string;
  files: { path: string; status: string }[];
  icon: (f: { status: string }) => React.ReactNode;
  statusLabel: (s: string) => string;
}) {
  return (
    <div className="mb-4">
      <h3 className="mb-1 text-xs font-medium uppercase text-neutral-500">
        {title} ({files.length})
      </h3>
      <div className="space-y-0.5">
        {files.map((f, i) => (
          <div
            key={i}
            className="flex items-center gap-2 rounded px-2 py-1 text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800"
          >
            {icon(f)}
            <span className="flex-1 truncate text-neutral-700 dark:text-neutral-300">
              {f.path}
            </span>
            <span className="flex-shrink-0 text-neutral-400">
              {statusLabel(f.status)}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── History Panel ─────────────────────────────────────────────────────────

function HistoryPanel({ log }: { log: GitLogResponse | null }) {
  if (!log || log.commits.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <p className="text-sm text-neutral-500">No commits yet</p>
      </div>
    );
  }

  return (
    <div className="p-4">
      <div className="space-y-2">
        {log.commits.map((commit) => (
          <div
            key={commit.sha}
            className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800"
          >
            <div className="mb-1 flex items-center gap-2">
              <GitCommitHorizontal className="h-3.5 w-3.5 text-neutral-400" />
              <span className="font-mono text-xs text-neutral-500">
                {commit.sha.substring(0, 7)}
              </span>
            </div>
            <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
              {commit.message}
            </p>
            <div className="mt-1 flex items-center gap-2 text-xs text-neutral-500">
              <span>{commit.author}</span>
              <span>·</span>
              <span>{new Date(commit.date).toLocaleString()}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── Branches Panel ────────────────────────────────────────────────────────

function BranchesPanel({
  branches,
  newBranchName,
  onNewBranchNameChange,
  onCreateBranch,
  creatingBranch,
  onDeleteBranch,
}: {
  branches: GitBranchResponse | null;
  newBranchName: string;
  onNewBranchNameChange: (name: string) => void;
  onCreateBranch: () => void;
  creatingBranch: boolean;
  onDeleteBranch: (name: string) => void;
}) {
  if (!branches) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <p className="text-sm text-neutral-500">Not a git repository</p>
      </div>
    );
  }

  // Sort: current branch first, rest alphabetically
  const sorted = [
    ...branches.branches.filter((b) => b.current),
    ...branches.branches.filter((b) => !b.current).sort((a, b) => a.name.localeCompare(b.name)),
  ];

  return (
    <div className="p-4">
      {/* Create branch */}
      <div className="mb-4 flex items-center gap-2">
        <input
          type="text"
          value={newBranchName}
          onChange={(e) => onNewBranchNameChange(e.target.value)}
          placeholder="New branch name"
          className="flex-1 rounded border border-neutral-300 bg-white px-3 py-1.5 text-sm text-neutral-900 placeholder-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500"
          onKeyDown={(e) => {
            if (e.key === "Enter" && newBranchName.trim() && !creatingBranch) {
              onCreateBranch();
            }
          }}
        />
        <button
          onClick={onCreateBranch}
          disabled={!newBranchName.trim() || creatingBranch}
          className="flex items-center gap-1 rounded bg-neutral-900 px-3 py-1.5 text-xs font-medium text-white hover:bg-neutral-800 disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
        >
          {creatingBranch ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Plus className="h-3.5 w-3.5" />
          )}
          Create
        </button>
      </div>

      {/* Branch list */}
      <div className="space-y-0.5">
        {sorted.map((branch) => (
          <div
            key={branch.name}
            className={`flex items-center gap-2 rounded px-3 py-2 text-sm ${
              branch.current
                ? "bg-neutral-100 font-medium dark:bg-neutral-800"
                : "hover:bg-neutral-50 dark:hover:bg-neutral-800/50"
            }`}
          >
            <GitBranch className="h-3.5 w-3.5 text-neutral-400" />
            <span
              className={`flex-1 ${
                branch.current
                  ? "text-neutral-900 dark:text-neutral-100"
                  : "text-neutral-600 dark:text-neutral-400"
              }`}
            >
              {branch.name}
            </span>
            {branch.current && (
              <span className="text-xs text-neutral-400">current</span>
            )}
            {branch.remote && (
              <span className="text-xs text-neutral-400">{branch.remote}</span>
            )}
            {!branch.current && (
              <button
                onClick={() => onDeleteBranch(branch.name)}
                className="rounded p-1 text-neutral-400 hover:bg-neutral-200 hover:text-red-600 dark:hover:bg-neutral-700 dark:hover:text-red-400"
                title="Delete branch"
              >
                <Trash2 className="h-3 w-3" />
              </button>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
