import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import {
  GitBranch,
  GitCommitHorizontal,
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
  ChevronDown,
  ChevronRight,
  Copy,
  Tag,
  User,
} from "lucide-react";
import { api } from "../../api/client";
import type {
  GitStatusResponse,
  GitBranchResponse,
  GitShowResponse,
  GitFileDiffResponse,
  GitGraphResponse,
} from "../../types/api";
import { DiffRenderer } from "../renderers/DiffRenderer";
import { formatGitDate } from "../../utils/format";
import { computeGraphLayout } from "../../lib/gitGraph";
import type { GraphRow } from "../../lib/gitGraph";
import { GitGraphSVG, getGraphWidth, GRAPH_ROW_HEIGHT } from "./GitGraph";
import { ContextMenu, type ContextMenuItem } from "../ui/ContextMenu";

type GitTab = "changes" | "history" | "branches";

const MIN_PANEL_PCT = 20; // minimum panel width in %

export function GitView() {
  const [activeTab, setActiveTab] = useState<GitTab>("changes");
  const [status, setStatus] = useState<GitStatusResponse | null>(null);
  const [branches, setBranches] = useState<GitBranchResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [commitMsg, setCommitMsg] = useState("");
  const [committing, setCommitting] = useState(false);
  const [commitResult, setCommitResult] = useState<string | null>(null);
  const [pushPullLoading, setPushPullLoading] = useState(false);
  const [stashLoading, setStashLoading] = useState(false);
  const [newBranchName, setNewBranchName] = useState("");
  const [creatingBranch, setCreatingBranch] = useState(false);

  // Graph view
  const [graphData, setGraphData] = useState<GitGraphResponse | null>(null);
  const [graphLoading, setGraphLoading] = useState(false);
  const [graphCount, setGraphCount] = useState(50);
  const [graphErrorMessage, setGraphErrorMessage] = useState<string | null>(null);

  // Commit detail (History tab)
  const [selectedCommit, setSelectedCommit] = useState<GitShowResponse | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [fileDiffs, setFileDiffs] = useState<Record<string, GitFileDiffResponse>>({});
  const [loadingFileDiff, setLoadingFileDiff] = useState<string | null>(null);
  const [loadingAllDiffs, setLoadingAllDiffs] = useState(false);
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set());

  // Changes diff (Changes tab)
  const [changeDiffs, setChangeDiffs] = useState<Record<string, GitFileDiffResponse>>({});
  const [loadingChangeDiff, setLoadingChangeDiff] = useState<string | null>(null);
  const [expandedChangeFiles, setExpandedChangeFiles] = useState<Set<string>>(new Set());

  // Mobile detail sheet
  const [detailOpen, setDetailOpen] = useState(false);
  const [animateOut, setAnimateOut] = useState(false);

  // Submodule toggle
  const [showSubmodules, setShowSubmodules] = useState(false);

  // Split panel resize
  const splitContainerRef = useRef<HTMLDivElement>(null);
  const [splitRatio, setSplitRatio] = useState(0.4); // left = 40%, right = 60%
  const [isResizingSplit, setIsResizingSplit] = useState(false);
  const resizeStartRef = useRef({ x: 0, ratio: 0 });

  // ── Split panel resize handlers ─────────────────────────────────────

  const handleSplitResizeStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      setIsResizingSplit(true);
      resizeStartRef.current = { x: e.clientX, ratio: splitRatio };
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    },
    [splitRatio],
  );

  useEffect(() => {
    if (!isResizingSplit) return;

    const handleMouseMove = (e: MouseEvent) => {
      const container = splitContainerRef.current;
      if (!container) return;
      const rect = container.getBoundingClientRect();
      const containerWidth = rect.width;
      if (containerWidth === 0) return;

      const dx = e.clientX - resizeStartRef.current.x;
      const newRatio = resizeStartRef.current.ratio + dx / containerWidth;
      // Clamp
      const clamped = Math.min(Math.max(newRatio, MIN_PANEL_PCT / 100), 1 - MIN_PANEL_PCT / 100);
      setSplitRatio(clamped);
    };

    const handleMouseUp = () => {
      setIsResizingSplit(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
    };
  }, [isResizingSplit]);

  // ── Data fetching ───────────────────────────────────────────────────

  const loadGraphData = useCallback(async (count?: number) => {
    const fetchCount = count ?? 50;
    setGraphLoading(true);
    setGraphErrorMessage(null);
    try {
      const result = await api.gitGraph(fetchCount);
      setGraphData(result);
      setGraphCount(fetchCount);
    } catch (err) {
      setGraphErrorMessage(err instanceof Error ? err.message : "Failed to load graph data");
    } finally {
      setGraphLoading(false);
    }
  }, []);

  const loadMoreGraph = useCallback(() => {
    if (graphLoading) return;
    const newCount = graphCount + 50;
    loadGraphData(newCount);
  }, [graphLoading, graphCount, loadGraphData]);

  const refreshStatus = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const [s, b] = await Promise.all([api.gitStatus(), api.gitBranches(showSubmodules)]);
      setStatus(s);
      setBranches(b);
      // Refresh graph data
      await loadGraphData(50);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load git data");
    } finally {
      setLoading(false);
    }
  }, [showSubmodules, loadGraphData]);

  // ── History commit detail ───────────────────────────────────────────

  const selectCommit = useCallback(async (sha: string) => {
    setLoadingDetail(true);
    setDetailError(null);
    setSelectedCommit(null);
    setFileDiffs({});
    setExpandedFiles(new Set());
    try {
      const detail = await api.gitShowCommit(sha);
      setSelectedCommit(detail);
      setDetailOpen(true);
    } catch (err) {
      setDetailError(err instanceof Error ? err.message : "Failed to load commit details");
    } finally {
      setLoadingDetail(false);
    }
  }, []);

  const loadFileDiff = useCallback(
    async (filePath: string) => {
      if (!selectedCommit) return;
      if (fileDiffs[filePath]) {
        setExpandedFiles((prev) => {
          const next = new Set(prev);
          if (next.has(filePath)) next.delete(filePath);
          else next.add(filePath);
          return next;
        });
        return;
      }
      setLoadingFileDiff(filePath);
      try {
        const diffs = await api.gitShowFileDiff(selectedCommit.sha, filePath);
        if (diffs.length > 0) {
          setFileDiffs((prev) => ({
            ...prev,
            [filePath]: diffs[0],
          }));
        }
        setExpandedFiles((prev) => new Set(prev).add(filePath));
      } catch (err) {
        setDetailError(err instanceof Error ? err.message : "Failed to load file diff");
      } finally {
        setLoadingFileDiff(null);
      }
    },
    [selectedCommit, fileDiffs],
  );

  const loadAllDiffs = useCallback(async () => {
    if (!selectedCommit) return;
    setLoadingAllDiffs(true);
    try {
      const diffs = await api.gitShowAllDiffs(selectedCommit.sha);
      const diffMap: Record<string, GitFileDiffResponse> = {};
      for (const d of diffs) {
        diffMap[d.path] = d;
      }
      setFileDiffs((prev) => ({ ...prev, ...diffMap }));
      setExpandedFiles(new Set([...expandedFiles, ...diffs.map((d) => d.path)]));
    } catch (err) {
      setDetailError(err instanceof Error ? err.message : "Failed to load all diffs");
    } finally {
      setLoadingAllDiffs(false);
    }
  }, [selectedCommit, expandedFiles]);

  const closeDetail = useCallback(() => {
    setDetailOpen(false);
    setSelectedCommit(null);
    setFileDiffs({});
    setExpandedFiles(new Set());
    setDetailError(null);
  }, []);

  const handleCloseMobile = useCallback(() => {
    setAnimateOut(true);
    setTimeout(() => {
      closeDetail();
      setAnimateOut(false);
    }, 280);
  }, [closeDetail]);

  // ── Changes file diff ───────────────────────────────────────────────

  const toggleChangeDiff = useCallback(
    async (filePath: string, staged: boolean, status: string) => {
      // Untracked files have no previous version to diff against
      if (status === "?") {
        setExpandedChangeFiles((prev) => {
          const next = new Set(prev);
          if (next.has(filePath)) next.delete(filePath);
          else next.add(filePath);
          return next;
        });
        return;
      }
      if (changeDiffs[filePath]) {
        setExpandedChangeFiles((prev) => {
          const next = new Set(prev);
          if (next.has(filePath)) next.delete(filePath);
          else next.add(filePath);
          return next;
        });
        return;
      }
      setLoadingChangeDiff(filePath);
      try {
        const result = await api.gitDiffFile(filePath, staged);
        setChangeDiffs((prev) => ({ ...prev, [filePath]: result }));
        setExpandedChangeFiles((prev) => new Set(prev).add(filePath));
      } catch (err) {
        setCommitResult(`Error loading diff: ${err instanceof Error ? err.message : "Unknown"}`);
      } finally {
        setLoadingChangeDiff(null);
      }
    },
    [changeDiffs],
  );

  // ── Operations ──────────────────────────────────────────────────────

  useEffect(() => {
    // Calling refreshStatus is an intentional async fire-and-forget for initial load.
    // The setLoading(true) inside is instantaneous, not cascading.
    // eslint-disable-next-line react-hooks/set-state-in-effect
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
      setCommitResult(`Error: ${err instanceof Error ? err.message : "Commit failed"}`);
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
      setCommitResult(`Push error: ${err instanceof Error ? err.message : "Push failed"}`);
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
      setCommitResult(`Pull error: ${err instanceof Error ? err.message : "Pull failed"}`);
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
      setCommitResult(`Error: ${err instanceof Error ? err.message : "Stash failed"}`);
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
      setCommitResult(`Error: ${err instanceof Error ? err.message : "Failed to create branch"}`);
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
      setCommitResult(`Error: ${err instanceof Error ? err.message : "Failed to delete branch"}`);
    }
  };

  const handleToggleSubmodules = () => {
    setShowSubmodules((prev) => !prev);
  };

  // ── Graph computation ─────────────────────────────────────────────────
  const graphRows = useMemo<GraphRow[]>(() => {
    if (!graphData) return [];
    return computeGraphLayout(graphData.commits, graphData.head_sha);
  }, [graphData]);

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
              {(status.ahead > 0 || status.behind > 0) && ` · ↑${status.ahead} ↓${status.behind}`}
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
            commitResult.startsWith("Error") ||
            commitResult.startsWith("Push error") ||
            commitResult.startsWith("Pull error")
              ? "bg-red-50 text-red-700 dark:bg-red-950/30 dark:text-red-400"
              : "bg-green-50 text-green-700 dark:bg-green-950/30 dark:text-green-400"
          }`}
        >
          {commitResult}
          <button onClick={() => setCommitResult(null)} className="ml-2 underline">
            Dismiss
          </button>
        </div>
      )}

      {/* Content */}
      <div className="flex flex-1 overflow-hidden">
        {loading && !status ? (
          <div className="flex h-full w-full items-center justify-center">
            <Loader2 className="h-5 w-5 animate-spin text-neutral-400" />
          </div>
        ) : activeTab === "changes" ? (
          <ChangesPanel
            status={status}
            commitMsg={commitMsg}
            onCommitMsgChange={setCommitMsg}
            onCommit={handleCommit}
            committing={committing}
            changeDiffs={changeDiffs}
            loadingChangeDiff={loadingChangeDiff}
            expandedChangeFiles={expandedChangeFiles}
            onToggleChangeDiff={toggleChangeDiff}
          />
        ) : activeTab === "history" ? (
          <div ref={splitContainerRef} className="flex flex-1 overflow-hidden">
            {/* Left: Graph history list */}
            <div className="overflow-y-auto" style={{ flex: `${splitRatio * 100}%` }}>
              <GraphHistoryPanel
                rows={graphRows}
                graphLoading={graphLoading}
                graphError={graphErrorMessage}
                selectedSha={selectedCommit?.sha ?? null}
                onSelectCommit={selectCommit}
                onRetry={() => loadGraphData(50)}
                onLoadMore={loadMoreGraph}
              />
            </div>

            {/* Resize handle */}
            <div
              onMouseDown={handleSplitResizeStart}
              className={`hidden w-1 cursor-col-resize bg-transparent hover:bg-neutral-300 dark:hover:bg-neutral-700 md:block ${
                isResizingSplit ? "bg-neutral-400 dark:bg-neutral-600" : ""
              }`}
              role="separator"
              aria-label="Resize panels"
            />

            {/* Right: Commit detail */}
            <div
              className="hidden overflow-y-auto md:block"
              style={{ flex: `${(1 - splitRatio) * 100}%` }}
            >
              {selectedCommit ? (
                <div className="p-4">
                  <CommitDetailPanel
                    commit={selectedCommit}
                    fileDiffs={fileDiffs}
                    loadingFileDiff={loadingFileDiff}
                    loadingAllDiffs={loadingAllDiffs}
                    loadingDetail={loadingDetail}
                    detailError={detailError}
                    expandedFiles={expandedFiles}
                    onLoadFileDiff={loadFileDiff}
                    onLoadAllDiffs={loadAllDiffs}
                  />
                </div>
              ) : (
                <div className="flex h-full items-center justify-center text-sm text-neutral-400">
                  Select a commit to view details
                </div>
              )}
            </div>
          </div>
        ) : (
          <div className="flex-1 overflow-y-auto">
            <BranchesPanel
              branches={branches}
              newBranchName={newBranchName}
              onNewBranchNameChange={setNewBranchName}
              onCreateBranch={handleCreateBranch}
              creatingBranch={creatingBranch}
              onDeleteBranch={handleDeleteBranch}
              showSubmodules={showSubmodules}
              onToggleSubmodules={handleToggleSubmodules}
            />
          </div>
        )}
      </div>

      {/* Mobile full-screen overlay for commit detail (History tab) */}
      {activeTab === "history" && (detailOpen || animateOut) && selectedCommit && (
        <>
          {/* Backdrop */}
          <button
            onClick={handleCloseMobile}
            className={`fixed inset-0 z-40 bg-black/30 transition-opacity duration-300 md:hidden ${
              animateOut ? "opacity-0" : ""
            }`}
            aria-label="Close detail"
          />
          {/* Full-screen overlay */}
          <div
            className={`fixed inset-0 z-50 flex flex-col bg-white motion-safe:animate-slide-up-full motion-safe:transition-transform motion-safe:duration-300 motion-safe:ease-smooth dark:bg-neutral-950 md:hidden ${
              animateOut ? "translate-y-full" : ""
            }`}
          >
            {/* Fixed top bar */}
            <div className="flex items-center gap-3 border-b border-neutral-200 px-4 py-3 dark:border-neutral-800">
              <button
                onClick={handleCloseMobile}
                className="rounded p-1 text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800"
              >
                <ChevronDown className="h-5 w-5" />
              </button>
              <span className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
                Commit Detail
              </span>
            </div>
            {/* Scrollable content */}
            <div className="flex-1 overflow-y-auto overscroll-contain">
              <div className="p-4">
                <CommitDetailPanel
                  commit={selectedCommit}
                  fileDiffs={fileDiffs}
                  loadingFileDiff={loadingFileDiff}
                  loadingAllDiffs={loadingAllDiffs}
                  loadingDetail={loadingDetail}
                  detailError={detailError}
                  expandedFiles={expandedFiles}
                  onLoadFileDiff={loadFileDiff}
                  onLoadAllDiffs={loadAllDiffs}
                />
              </div>
            </div>
          </div>
        </>
      )}
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
  changeDiffs,
  loadingChangeDiff,
  expandedChangeFiles,
  onToggleChangeDiff,
}: {
  status: GitStatusResponse | null;
  commitMsg: string;
  onCommitMsgChange: (msg: string) => void;
  onCommit: () => void;
  committing: boolean;
  changeDiffs: Record<string, GitFileDiffResponse>;
  loadingChangeDiff: string | null;
  expandedChangeFiles: Set<string>;
  onToggleChangeDiff: (path: string, staged: boolean, status: string) => void;
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
    <div className="flex-1 overflow-y-auto p-4">
      {/* Commit input */}
      <div className="mb-4">
        <textarea
          value={commitMsg}
          onChange={(e) => onCommitMsgChange(e.target.value)}
          placeholder="Commit message"
          rows={2}
          className="w-full rounded border border-neutral-300 bg-white px-3 py-2 text-base text-neutral-900 placeholder-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500"
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
        <div className="mb-4">
          <h3 className="mb-1 text-xs font-medium uppercase text-neutral-500">
            Staged ({staged.length})
          </h3>
          <div className="space-y-0.5">
            {staged.map((f, i) => (
              <ChangeFileRow
                key={i}
                file={f}
                icon={fileIcon(f)}
                label={statusLabel(f.status)}
                diff={changeDiffs[f.path]}
                isLoading={loadingChangeDiff === f.path}
                isExpanded={expandedChangeFiles.has(f.path)}
                onToggle={() => onToggleChangeDiff(f.path, true, f.status)}
              />
            ))}
          </div>
        </div>
      )}
      {unstaged.length > 0 && (
        <div className="mb-4">
          <h3 className="mb-1 text-xs font-medium uppercase text-neutral-500">
            Changes ({unstaged.length})
          </h3>
          <div className="space-y-0.5">
            {unstaged.map((f, i) => (
              <ChangeFileRow
                key={i}
                file={f}
                icon={fileIcon(f)}
                label={statusLabel(f.status)}
                diff={changeDiffs[f.path]}
                isLoading={loadingChangeDiff === f.path}
                isExpanded={expandedChangeFiles.has(f.path)}
                onToggle={() => onToggleChangeDiff(f.path, false, f.status)}
              />
            ))}
          </div>
        </div>
      )}
      {(!status || status.files.length === 0) && (
        <div className="py-8 text-center text-sm text-neutral-500">No changes in working tree</div>
      )}
    </div>
  );
}

function ChangeFileRow({
  file,
  icon,
  label,
  diff,
  isLoading,
  isExpanded,
  onToggle,
}: {
  file: { path: string; status: string };
  icon: React.ReactNode;
  label: string;
  diff: GitFileDiffResponse | undefined;
  isLoading: boolean;
  isExpanded: boolean;
  onToggle: () => void;
}) {
  return (
    <div>
      <button
        onClick={onToggle}
        className="flex w-full items-center gap-2 rounded px-2 py-1 text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800"
      >
        {icon}
        <span className="flex-1 truncate text-left text-neutral-700 dark:text-neutral-300">
          {file.path}
        </span>
        <span className="flex-shrink-0 text-neutral-400">{label}</span>
        {isLoading && <Loader2 className="h-3 w-3 animate-spin text-neutral-400" />}
        <ChevronRight
          className={`h-3 w-3 text-neutral-400 transition-transform ${
            isExpanded ? "rotate-90" : ""
          }`}
        />
      </button>
      {/* Diff content — smooth height transition */}
      <div
        className="motion-safe:transition-all motion-safe:duration-300 motion-safe:ease-smooth grid"
        style={{
          gridTemplateRows: isExpanded ? "1fr" : "0fr",
          opacity: isExpanded ? 1 : 0,
        }}
      >
        <div className="min-h-0 overflow-hidden">
          <div className="ml-4 border-l-2 border-neutral-200 pl-2 dark:border-neutral-700">
            {file.status === "?" ? (
              <p className="py-2 text-xs text-neutral-400">
                New file — no previous version to diff against
              </p>
            ) : diff ? (
              diff.diff ? (
                <DiffRenderer diff={diff.diff} filepath={file.path} />
              ) : (
                <p className="py-2 text-xs text-neutral-400">
                  No diff content (binary or empty file)
                </p>
              )
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Graph History Panel ────────────────────────────────────────────────────

function buildCommitContextMenuItems(row: GraphRow): ContextMenuItem[] {
  const copy = (text: string) => {
    navigator.clipboard.writeText(text).catch(() => {});
  };

  const items: ContextMenuItem[] = [
    {
      label: "Copy SHA (short)",
      icon: <GitCommitHorizontal className="h-3.5 w-3.5" />,
      onClick: () => copy(row.commit.sha.substring(0, 7)),
    },
    {
      label: "Copy SHA (full)",
      icon: <FileText className="h-3.5 w-3.5" />,
      onClick: () => copy(row.commit.sha),
    },
  ];

  // Extract tag names from refs
  const tags = (row.commit.refs ?? [])
    .filter((r) => r.startsWith("tag: "))
    .map((r) => r.replace(/^tag: /, ""));
  if (tags.length > 0) {
    items.push({ type: "separator" });
    for (const tag of tags) {
      items.push({
        label: `Copy Tag (${tag})`,
        icon: <Tag className="h-3.5 w-3.5" />,
        onClick: () => copy(tag),
      });
    }
  }

  items.push({ type: "separator" });
  items.push(
    {
      label: "Copy Author",
      icon: <User className="h-3.5 w-3.5" />,
      onClick: () => copy(row.commit.author),
    },
    {
      label: "Copy Title",
      icon: <Copy className="h-3.5 w-3.5" />,
      onClick: () => copy(row.commit.message.split("\n")[0]),
    },
    {
      label: "Copy Full Message",
      icon: <FileText className="h-3.5 w-3.5" />,
      onClick: () => copy(row.commit.message),
    },
  );

  return items;
}

function GraphHistoryPanel({
  rows,
  graphLoading,
  graphError,
  selectedSha,
  onSelectCommit,
  onRetry,
  onLoadMore,
}: {
  rows: GraphRow[];
  graphLoading: boolean;
  graphError: string | null;
  selectedSha: string | null;
  onSelectCommit: (sha: string) => void;
  onRetry: () => void;
  onLoadMore: () => void;
}) {
  const sentinelRef = useRef<HTMLDivElement>(null);

  // Context menu state
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    row: GraphRow;
  } | null>(null);

  // Long-press detection for touch devices
  const longPressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const isLongPress = useRef(false);

  // IntersectionObserver for infinite scroll
  useEffect(() => {
    const el = sentinelRef.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting && !graphLoading) {
          onLoadMore();
        }
      },
      { rootMargin: "300px" },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [graphLoading, onLoadMore]);

  const handleContextMenu = useCallback((e: React.MouseEvent, row: GraphRow) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY, row });
  }, []);

  const handleTouchStart = useCallback((e: React.TouchEvent, row: GraphRow) => {
    isLongPress.current = false;
    const touch = e.touches[0];
    longPressTimer.current = setTimeout(() => {
      isLongPress.current = true;
      setContextMenu({ x: touch.clientX, y: touch.clientY, row });
    }, 500);
  }, []);

  const handleTouchMove = useCallback(() => {
    if (longPressTimer.current) {
      clearTimeout(longPressTimer.current);
      longPressTimer.current = null;
    }
  }, []);

  const handleTouchEnd = useCallback(() => {
    if (longPressTimer.current) {
      clearTimeout(longPressTimer.current);
      longPressTimer.current = null;
    }
  }, []);

  const handleClick = useCallback(
    (sha: string) => {
      if (isLongPress.current) {
        isLongPress.current = false;
        return;
      }
      onSelectCommit(sha);
    },
    [onSelectCommit],
  );

  if (graphLoading && rows.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <Loader2 className="h-5 w-5 animate-spin text-neutral-400" />
      </div>
    );
  }

  if (graphError && rows.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <div className="text-center">
          <p className="text-sm text-red-600 dark:text-red-400">{graphError}</p>
          <button
            onClick={onRetry}
            className="mt-3 rounded bg-neutral-200 px-3 py-1.5 text-xs font-medium text-neutral-700 hover:bg-neutral-300 dark:bg-neutral-800 dark:text-neutral-300 dark:hover:bg-neutral-700"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  if (rows.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <p className="text-sm text-neutral-500">No commits yet</p>
      </div>
    );
  }

  const graphWidth = getGraphWidth(rows);

  return (
    <div className="relative py-2">
      {/* SVG graph layer — positioned absolutely behind the cards */}
      <div className="absolute left-0 top-0 pointer-events-none" style={{ width: graphWidth }}>
        <GitGraphSVG rows={rows} selectedSha={selectedSha} onSelectCommit={onSelectCommit} />
      </div>

      {/* Commit cards — overlaid on top with padding for the graph */}
      <div className="space-y-0" style={{ paddingLeft: graphWidth + 8 }}>
        {rows.map((row) => {
          const isSelected = row.commit.sha === selectedSha;
          return (
            <button
              key={row.commit.sha}
              onClick={() => handleClick(row.commit.sha)}
              onContextMenu={(e) => handleContextMenu(e, row)}
              onTouchStart={(e) => handleTouchStart(e, row)}
              onTouchMove={handleTouchMove}
              onTouchEnd={handleTouchEnd}
              className={`w-full rounded-lg px-2 py-1.5 text-left transition-colors ${
                isSelected
                  ? "bg-neutral-100 dark:bg-neutral-800"
                  : "hover:bg-neutral-50 dark:hover:bg-neutral-900"
              }`}
              style={{ height: GRAPH_ROW_HEIGHT }}
            >
              {row.refLabels.length > 0 && (
                <div className="flex items-center gap-2">
                  {row.refLabels.slice(0, 3).map((rl, ri) => (
                    <span
                      key={ri}
                      className={`inline-block rounded px-1.5 py-[1px] text-[10px] font-medium leading-tight text-white ${
                        rl.isHead ? "bg-green-500" : "bg-indigo-500"
                      }`}
                    >
                      {rl.label}
                    </span>
                  ))}
                </div>
              )}
              <p className="truncate text-sm font-medium text-neutral-900 dark:text-neutral-100">
                {row.commit.message}
              </p>
              <div className="flex items-center gap-2 text-[11px] text-neutral-500">
                <span className="font-mono text-[11px] text-neutral-500">
                  {row.commit.sha.substring(0, 7)}
                </span>
                <span>·</span>
                <span>{row.commit.author}</span>
                <span>·</span>
                <span>{formatGitDate(row.commit.date)}</span>
              </div>
            </button>
          );
        })}
      </div>

      {/* Sentinel for infinite scroll */}
      <div ref={sentinelRef} className="h-4" />

      {graphLoading && (
        <div className="flex items-center justify-center py-4">
          <Loader2 className="h-4 w-4 animate-spin text-neutral-400" />
          <span className="ml-2 text-xs text-neutral-500">Loading graph...</span>
        </div>
      )}

      {/* Context menu */}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          items={buildCommitContextMenuItems(contextMenu.row)}
          onClose={() => setContextMenu(null)}
        />
      )}
    </div>
  );
}

// ── History Panel ─────────────────────────────────────────────────────────

// ── Commit Detail Panel ───────────────────────────────────────────────────

function CommitDetailPanel({
  commit,
  fileDiffs,
  loadingFileDiff,
  loadingAllDiffs,
  loadingDetail,
  detailError,
  expandedFiles,
  onLoadFileDiff,
  onLoadAllDiffs,
}: {
  commit: GitShowResponse;
  fileDiffs: Record<string, GitFileDiffResponse>;
  loadingFileDiff: string | null;
  loadingAllDiffs: boolean;
  loadingDetail: boolean;
  detailError: string | null;
  expandedFiles: Set<string>;
  onLoadFileDiff: (path: string) => void;
  onLoadAllDiffs: () => void;
}) {
  const statusIcon = (s: string) => {
    switch (s) {
      case "A":
        return <FilePlus className="h-3.5 w-3.5 text-green-600" />;
      case "D":
        return <FileX className="h-3.5 w-3.5 text-red-600" />;
      case "M":
        return <FileEdit className="h-3.5 w-3.5 text-yellow-600" />;
      default:
        return <FileText className="h-3.5 w-3.5 text-neutral-500" />;
    }
  };

  if (loadingDetail) {
    return (
      <div className="flex items-center justify-center p-8">
        <Loader2 className="h-5 w-5 animate-spin text-neutral-400" />
      </div>
    );
  }

  if (detailError) {
    return (
      <div className="p-4">
        <p className="text-sm text-red-600 dark:text-red-400">{detailError}</p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* Commit header */}
      <div>
        <div className="flex items-center gap-2">
          <GitCommitHorizontal className="h-4 w-4 text-neutral-400" />
          <span className="font-mono text-xs text-neutral-500">{commit.sha.substring(0, 7)}</span>
        </div>
        <p className="mt-1 text-sm font-medium text-neutral-900 dark:text-neutral-100">
          {commit.message}
        </p>
        <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-neutral-500">
          <span>{commit.author}</span>
          <span>·</span>
          <span>{new Date(commit.date).toLocaleString()}</span>
        </div>
        <div className="mt-1 flex items-center gap-2 text-xs">
          <span className="text-green-600">+{commit.total_additions}</span>
          <span className="text-red-600">-{commit.total_deletions}</span>
          <span className="text-neutral-400">{commit.files.length} file(s)</span>
        </div>
      </div>

      {/* Divider */}
      <div className="border-t border-neutral-200 dark:border-neutral-800" />

      {/* File list */}
      <div>
        <div className="mb-2 flex items-center justify-between">
          <h3 className="text-xs font-medium uppercase text-neutral-500">
            Changed Files ({commit.files.length})
          </h3>
          <button
            onClick={onLoadAllDiffs}
            disabled={loadingAllDiffs}
            className="flex items-center gap-1 rounded px-2 py-1 text-[10px] text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800 disabled:opacity-50"
          >
            {loadingAllDiffs ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <ChevronDown className="h-3 w-3" />
            )}
            Show all diffs
          </button>
        </div>
        <div className="space-y-1">
          {commit.files.map((file) => (
            <div key={file.path}>
              <button
                onClick={() => onLoadFileDiff(file.path)}
                className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800"
              >
                {statusIcon(file.status)}
                <span className="flex-1 truncate text-left text-neutral-700 dark:text-neutral-300">
                  {file.path}
                </span>
                <span className="flex-shrink-0 text-[10px] text-green-600">+{file.additions}</span>
                <span className="flex-shrink-0 text-[10px] text-red-600">-{file.deletions}</span>
                {loadingFileDiff === file.path && (
                  <Loader2 className="h-3 w-3 animate-spin text-neutral-400" />
                )}
                <ChevronRight
                  className={`h-3 w-3 text-neutral-400 transition-transform ${
                    expandedFiles.has(file.path) ? "rotate-90" : ""
                  }`}
                />
              </button>

              {/* Inline diff for this file — smooth height transition */}
              <div
                className="motion-safe:transition-all motion-safe:duration-300 motion-safe:ease-smooth grid"
                style={{
                  gridTemplateRows:
                    expandedFiles.has(file.path) && fileDiffs[file.path] ? "1fr" : "0fr",
                  opacity: expandedFiles.has(file.path) && fileDiffs[file.path] ? 1 : 0,
                }}
              >
                <div className="min-h-0 overflow-hidden">
                  {fileDiffs[file.path] && (
                    <div className="ml-4 border-l-2 border-neutral-200 pl-2 dark:border-neutral-700">
                      {fileDiffs[file.path].diff ? (
                        <DiffRenderer diff={fileDiffs[file.path].diff} filepath={file.path} />
                      ) : (
                        <p className="py-2 text-xs text-neutral-400">
                          No diff content (binary or empty file)
                        </p>
                      )}
                    </div>
                  )}
                </div>
              </div>
            </div>
          ))}
        </div>
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
  showSubmodules,
  onToggleSubmodules,
}: {
  branches: GitBranchResponse | null;
  newBranchName: string;
  onNewBranchNameChange: (name: string) => void;
  onCreateBranch: () => void;
  creatingBranch: boolean;
  onDeleteBranch: (name: string) => void;
  showSubmodules: boolean;
  onToggleSubmodules: () => void;
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
      <div className="mb-4">
        <div className="flex gap-2">
          <input
            type="text"
            value={newBranchName}
            onChange={(e) => onNewBranchNameChange(e.target.value)}
            placeholder="New branch name"
            className="flex-1 rounded border border-neutral-300 bg-white px-3 py-1.5 text-base text-neutral-900 placeholder-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500"
            onKeyDown={(e) => {
              if (e.key === "Enter" && newBranchName.trim() && !creatingBranch) onCreateBranch();
            }}
          />
          <button
            onClick={onCreateBranch}
            disabled={!newBranchName.trim() || creatingBranch}
            className="flex items-center gap-1 rounded bg-neutral-900 px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-neutral-800 disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
          >
            {creatingBranch ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Plus className="h-3.5 w-3.5" />
            )}
            Create
          </button>
        </div>
      </div>

      {/* Submodule toggle */}
      <div className="mb-3 flex items-center gap-2">
        <button
          onClick={onToggleSubmodules}
          className={`relative inline-flex h-4 w-7 items-center rounded-full transition-colors ${
            showSubmodules ? "bg-neutral-500" : "bg-neutral-300 dark:bg-neutral-600"
          }`}
        >
          <span
            className={`inline-block h-3 w-3 transform rounded-full bg-white transition-transform ${
              showSubmodules ? "translate-x-3.5" : "translate-x-0.5"
            }`}
          />
        </button>
        <span className="text-xs text-neutral-500">Show submodule branches</span>
      </div>

      {/* Branch list */}
      <div className="space-y-1">
        {sorted.map((branch, i) => (
          <div
            key={i}
            className="flex items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800"
          >
            <GitBranch className="h-3.5 w-3.5 text-neutral-500" />
            <span className="flex-1 font-medium text-neutral-900 dark:text-neutral-100">
              {branch.name}
            </span>
            {branch.current && <span className="text-xs text-neutral-400">current</span>}
            {branch.remote && <span className="text-xs text-neutral-400">{branch.remote}</span>}
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
