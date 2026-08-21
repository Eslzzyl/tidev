import { Check, ChevronRight, FileEdit, FilePlus, FileText, FileX, Loader2 } from "lucide-react";
import type { GitFileDiffResponse, GitStatusResponse } from "../../../types/api";
import { DiffRenderer } from "../../renderers/DiffRenderer";

export function ChangesPanel({
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
          className="git-commit-button mt-2 flex items-center gap-1.5 rounded bg-neutral-900 px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-neutral-800 disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
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

export function ChangeFileRow({
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
