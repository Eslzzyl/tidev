import { useCallback, useEffect, useRef, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  Copy,
  FileEdit,
  FilePlus,
  FileText,
  FileX,
  GitCommitHorizontal,
  Loader2,
  Tag,
  User,
} from "lucide-react";
import type { GitFileDiffResponse, GitShowResponse } from "../../../types/api";
import { DiffRenderer } from "../../renderers/DiffRenderer";
import { ContextMenu, type ContextMenuItem } from "../../ui/ContextMenu";
import { formatGitDate } from "../../../utils/format";
import { GitGraphSVG, getGraphWidth, GRAPH_ROW_HEIGHT } from "../GitGraph";
import type { GraphRow } from "../../../lib/gitGraph";

export function buildCommitContextMenuItems(row: GraphRow): ContextMenuItem[] {
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

export function GraphHistoryPanel({
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

export function CommitDetailPanel({
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
