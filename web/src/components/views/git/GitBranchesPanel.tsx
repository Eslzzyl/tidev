import { GitBranch, Loader2, Plus, Trash2 } from "lucide-react";
import type { GitBranchResponse } from "../../../types/api";
import { useTranslation } from "react-i18next";

export function BranchesPanel({
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
  const { t } = useTranslation();
  if (!branches) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <p className="text-sm text-neutral-500">{t("Not a git repository")}</p>
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
            placeholder={t("New branch name")}
            className="flex-1 rounded border border-neutral-300 bg-white px-3 py-1.5 text-base text-neutral-900 placeholder-neutral-400 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:placeholder-neutral-500"
            onKeyDown={(e) => {
              if (e.key === "Enter" && newBranchName.trim() && !creatingBranch) onCreateBranch();
            }}
          />
          <button
            onClick={onCreateBranch}
            disabled={!newBranchName.trim() || creatingBranch}
            className="git-primary-button flex items-center gap-1 rounded px-3 py-1.5 text-xs font-medium transition-colors hover:bg-neutral-800 dark:hover:bg-neutral-200"
          >
            {creatingBranch ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Plus className="h-3.5 w-3.5" />
            )}
            {t("Create")}
          </button>
        </div>
      </div>

      {/* Submodule toggle */}
      <div className="mb-3 flex items-center gap-2">
        <button
          onClick={onToggleSubmodules}
          aria-label={t("Show submodule branches")}
          aria-pressed={showSubmodules}
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
        <span className="text-xs text-neutral-500">{t("Show submodule branches")}</span>
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
            {branch.current && <span className="text-xs text-neutral-400">{t("current")}</span>}
            {branch.remote && <span className="text-xs text-neutral-400">{branch.remote}</span>}
            {!branch.current && (
              <button
                onClick={() => onDeleteBranch(branch.name)}
                className="rounded p-1 text-neutral-400 hover:bg-neutral-200 hover:text-red-600 dark:hover:bg-neutral-700 dark:hover:text-red-400"
                title={t("Delete branch")}
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
