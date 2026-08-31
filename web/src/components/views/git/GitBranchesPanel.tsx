import { GitBranch, Loader2, Plus, Trash2 } from "lucide-react";
import type { GitBranchResponse } from "../../../types/api";
import { useTranslation } from "react-i18next";
import { Button, IconButton, Input, Switch } from "../../ui";

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
          <Input
            type="text"
            value={newBranchName}
            onChange={(e) => onNewBranchNameChange(e.target.value)}
            placeholder={t("New branch name")}
            className="flex-1"
            onKeyDown={(e) => {
              if (e.key === "Enter" && newBranchName.trim() && !creatingBranch) onCreateBranch();
            }}
          />
          <Button
            onClick={onCreateBranch}
            disabled={!newBranchName.trim() || creatingBranch}
            variant="primary"
            size="sm"
            leadingIcon={
              creatingBranch ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Plus className="h-3.5 w-3.5" />
              )
            }
          >
            {t("Create")}
          </Button>
        </div>
      </div>

      {/* Submodule toggle */}
      <div className="mb-3 flex items-center gap-2">
        <Switch
          checked={showSubmodules}
          onCheckedChange={onToggleSubmodules}
          aria-label={t("Show submodule branches")}
        />
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
              <IconButton
                label={t("Delete branch")}
                size="sm"
                onClick={() => onDeleteBranch(branch.name)}
                title={t("Delete branch")}
              >
                <Trash2 className="h-3 w-3" />
              </IconButton>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
