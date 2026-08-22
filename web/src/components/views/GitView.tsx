import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import {
  GitBranch,
  GitCommitHorizontal,
  ArrowUpFromLine,
  ArrowDownFromLine,
  Archive,
  RotateCcw,
  FileEdit,
  Loader2,
  ChevronDown,
} from "lucide-react";
import { api } from "../../api/client";
import { queryClient } from "../../lib/queryClient";
import {
  useGitCommit,
  useGitPush,
  useGitPull,
  useGitStash,
  useGitBranchCreate,
  useGitBranchDelete,
} from "../../hooks/useQueries";
import type {
  GitStatusResponse,
  GitBranchResponse,
  GitShowResponse,
  GitFileDiffResponse,
  GitGraphResponse,
} from "../../types/api";
import { computeGraphLayout } from "../../lib/gitGraph";
import type { GraphRow } from "../../lib/gitGraph";
import { ChangesPanel } from "./git/GitChangesPanel";
import { GraphHistoryPanel, CommitDetailPanel } from "./git/GitHistoryPanels";
import { BranchesPanel } from "./git/GitBranchesPanel";
import { useTranslation } from "react-i18next";

type GitTab = "changes" | "history" | "branches";

const MIN_PANEL_PCT = 20; // minimum panel width in %

export function GitView() {
  const { t } = useTranslation();
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
  const desktopDetailScrollRef = useRef<HTMLDivElement>(null);
  const mobileDetailScrollRef = useRef<HTMLDivElement>(null);

  // Submodule toggle
  const [showSubmodules, setShowSubmodules] = useState(false);

  // TanStack Query mutations
  const gitCommitMutation = useGitCommit();
  const gitPushMutation = useGitPush();
  const gitPullMutation = useGitPull();
  const gitStashMutation = useGitStash();
  const gitBranchCreateMutation = useGitBranchCreate();
  const gitBranchDeleteMutation = useGitBranchDelete();

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
      const result = await queryClient.fetchQuery({
        queryKey: ["git", "graph", fetchCount],
        queryFn: () => api.gitGraph(fetchCount),
      });
      setGraphData(result);
      setGraphCount(fetchCount);
    } catch (err) {
      setGraphErrorMessage(err instanceof Error ? err.message : t("Failed to load graph data"));
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
      const [s, b] = await Promise.all([
        queryClient.fetchQuery({ queryKey: ["git", "status"], queryFn: api.gitStatus }),
        queryClient.fetchQuery({
          queryKey: ["git", "branches", showSubmodules],
          queryFn: () => api.gitBranches(showSubmodules),
        }),
      ]);
      setStatus(s);
      setBranches(b);
      // Refresh graph data
      await loadGraphData(50);
    } catch (err) {
      setError(err instanceof Error ? err.message : t("Failed to load git data"));
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
      const detail = await queryClient.fetchQuery({
        queryKey: ["git", "show", sha],
        queryFn: () => api.gitShowCommit(sha),
      });
      setSelectedCommit(detail);
      setDetailOpen(true);
    } catch (err) {
      setDetailError(err instanceof Error ? err.message : t("Failed to load commit details"));
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
        const diffs = await queryClient.fetchQuery({
          queryKey: ["git", "diff", selectedCommit.sha, filePath],
          queryFn: () => api.gitShowFileDiff(selectedCommit.sha, filePath),
        });
        if (diffs.length > 0) {
          setFileDiffs((prev) => ({
            ...prev,
            [filePath]: diffs[0],
          }));
        }
        setExpandedFiles((prev) => new Set(prev).add(filePath));
      } catch (err) {
        setDetailError(err instanceof Error ? err.message : t("Failed to load file diff"));
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
      const diffs = await queryClient.fetchQuery({
        queryKey: ["git", "diffs", selectedCommit.sha],
        queryFn: () => api.gitShowAllDiffs(selectedCommit.sha),
      });
      const diffMap: Record<string, GitFileDiffResponse> = {};
      for (const d of diffs) {
        diffMap[d.path] = d;
      }
      setFileDiffs((prev) => ({ ...prev, ...diffMap }));
      setExpandedFiles((prev) => new Set([...prev, ...diffs.map((d) => d.path)]));
    } catch (err) {
      setDetailError(err instanceof Error ? err.message : t("Failed to load all diffs"));
    } finally {
      setLoadingAllDiffs(false);
    }
  }, [selectedCommit, expandedFiles]);

  const toggleAllDiffs = useCallback(async () => {
    if (!selectedCommit) return;
    const allDiffsExpanded =
      selectedCommit.files.length > 0 &&
      selectedCommit.files.every((file) => expandedFiles.has(file.path));
    if (allDiffsExpanded) {
      setExpandedFiles(new Set());
      return;
    }
    await loadAllDiffs();
  }, [selectedCommit, expandedFiles, loadAllDiffs]);

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
        const result = await queryClient.fetchQuery({
          queryKey: ["git", "file-diff", filePath, staged],
          queryFn: () => api.gitDiffFile(filePath, staged),
        });
        setChangeDiffs((prev) => ({ ...prev, [filePath]: result }));
        setExpandedChangeFiles((prev) => new Set(prev).add(filePath));
      } catch (err) {
        setCommitResult(
          t("Error loading diff: {{message}}", {
            message: err instanceof Error ? err.message : t("Unknown error"),
          }),
        );
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
      const result = await gitCommitMutation.mutateAsync(commitMsg.trim());
      setCommitResult(result.message);
      setCommitMsg("");
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        t("Error: {{message}}", {
          message: err instanceof Error ? err.message : t("Commit failed"),
        }),
      );
    } finally {
      setCommitting(false);
    }
  };

  const handlePush = async () => {
    setPushPullLoading(true);
    try {
      const result = await gitPushMutation.mutateAsync({});
      setCommitResult(result.message);
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        t("Push error: {{message}}", {
          message: err instanceof Error ? err.message : t("Push failed"),
        }),
      );
    } finally {
      setPushPullLoading(false);
    }
  };

  const handlePull = async () => {
    setPushPullLoading(true);
    try {
      const result = await gitPullMutation.mutateAsync({});
      setCommitResult(result.message);
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        t("Pull error: {{message}}", {
          message: err instanceof Error ? err.message : t("Pull failed"),
        }),
      );
    } finally {
      setPushPullLoading(false);
    }
  };

  const handleStash = async () => {
    setStashLoading(true);
    try {
      const result = await gitStashMutation.mutateAsync(undefined);
      setCommitResult(result.message);
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        t("Error: {{message}}", {
          message: err instanceof Error ? err.message : t("Stash failed"),
        }),
      );
    } finally {
      setStashLoading(false);
    }
  };

  const handleCreateBranch = async () => {
    if (!newBranchName.trim()) return;
    setCreatingBranch(true);
    try {
      const result = await gitBranchCreateMutation.mutateAsync({
        name: newBranchName.trim(),
        checkout: true,
      });
      setCommitResult(result.message);
      setNewBranchName("");
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        t("Error: {{message}}", {
          message: err instanceof Error ? err.message : t("Failed to create branch"),
        }),
      );
    } finally {
      setCreatingBranch(false);
    }
  };

  const handleDeleteBranch = async (name: string) => {
    try {
      const result = await gitBranchDeleteMutation.mutateAsync(name);
      setCommitResult(result.message);
      await refreshStatus();
    } catch (err) {
      setCommitResult(
        t("Error: {{message}}", {
          message: err instanceof Error ? err.message : t("Failed to delete branch"),
        }),
      );
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
      label: t("Changes"),
      icon: <FileEdit className="h-4 w-4" />,
    },
    {
      id: "history",
      label: t("History"),
      icon: <GitCommitHorizontal className="h-4 w-4" />,
    },
    {
      id: "branches",
      label: t("Branches"),
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
            {t("Retry")}
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
            className="git-icon-button rounded p-1.5 text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
            title={t("Pull")}
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
            className="git-icon-button rounded p-1.5 text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
            title={t("Push")}
          >
            <ArrowUpFromLine className="h-3.5 w-3.5" />
          </button>
          <button
            onClick={handleStash}
            disabled={stashLoading}
            className="git-icon-button rounded p-1.5 text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
            title={t("Stash")}
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
            className="git-icon-button rounded p-1.5 text-neutral-500 hover:bg-neutral-200 dark:hover:bg-neutral-700"
            title={t("Refresh")}
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
            commitResult.startsWith(t("Error")) ||
            commitResult.startsWith(t("Push error")) ||
            commitResult.startsWith(t("Pull error"))
              ? "bg-red-50 text-red-700 dark:bg-red-950/30 dark:text-red-400"
              : "bg-green-50 text-green-700 dark:bg-green-950/30 dark:text-green-400"
          }`}
        >
          {commitResult}
          <button onClick={() => setCommitResult(null)} className="ml-2 underline">
            {t("Dismiss")}
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
              aria-label={t("Resize panels")}
            />

            {/* Right: Commit detail */}
            <div
              ref={desktopDetailScrollRef}
              className="hidden overflow-y-auto md:block"
              style={{
                flex: `${(1 - splitRatio) * 100}%`,
                overscrollBehaviorX: "none",
              }}
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
                    onToggleAllDiffs={toggleAllDiffs}
                    scrollRef={desktopDetailScrollRef}
                  />
                </div>
              ) : (
                <div className="flex h-full items-center justify-center text-sm text-neutral-400">
                  {t("Select a commit to view details")}
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
            aria-label={t("Close detail")}
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
                {t("Commit Detail")}
              </span>
            </div>
            {/* Scrollable content */}
            <div
              ref={mobileDetailScrollRef}
              className="flex-1 overflow-y-auto overscroll-contain"
              style={{ overscrollBehaviorX: "none" }}
            >
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
                  onToggleAllDiffs={toggleAllDiffs}
                  scrollRef={mobileDetailScrollRef}
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
