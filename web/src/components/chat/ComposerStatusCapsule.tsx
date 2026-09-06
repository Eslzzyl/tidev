import { Check, Circle, FileEdit, FilePlus, FileX } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { FileDiff, TodoItem } from "../../types/api";
import { changedFileTotals, sortChangedFiles } from "../../utils/changedFiles";

function isTodoCompleted(todo: TodoItem) {
  return todo.status === "completed";
}

function TodoStatusSegment({ todos }: { todos: TodoItem[] }) {
  const { t } = useTranslation();
  const activeIndex = todos.findIndex((todo) => todo.status === "in_progress");
  const currentIndex =
    activeIndex >= 0 ? activeIndex : todos.findIndex((todo) => !isTodoCompleted(todo));
  const displayedIndex = currentIndex >= 0 ? currentIndex : todos.length - 1;

  return (
    <div className="composer-status-segment composer-todo-segment" tabIndex={0} role="group">
      <div className="composer-status-popover composer-todo-card">
        <div className="composer-todo-list">
          {todos.map((todo, index) => {
            const completed = isTodoCompleted(todo);
            const active = index === displayedIndex && !completed;
            const className = [
              "composer-todo-entry",
              completed ? "is-completed" : "",
              active ? "is-active" : "",
            ]
              .filter(Boolean)
              .join(" ");

            return (
              <div className={className} key={`${todo.content}:${index}`}>
                <span className="composer-todo-entry-check" aria-hidden="true">
                  {completed ? <Check size={12} /> : null}
                </span>
                <span>{todo.content}</span>
              </div>
            );
          })}
        </div>
      </div>
      <div
        className="composer-status-trigger composer-todo-trigger"
        aria-label={t("Step {{current}} / {{total}}", {
          current: displayedIndex + 1,
          total: todos.length,
        })}
      >
        <span className="composer-todo-trigger-dot" aria-hidden="true" />
        <span>
          {t("Step {{current}} / {{total}}", {
            current: displayedIndex + 1,
            total: todos.length,
          })}
        </span>
      </div>
    </div>
  );
}

function changedFileIcon(status: FileDiff["status"]) {
  if (status === "added") return <FilePlus size={14} aria-hidden="true" />;
  if (status === "deleted") return <FileX size={14} aria-hidden="true" />;
  return <FileEdit size={14} aria-hidden="true" />;
}

function ChangedFilesStatusSegment({ files, onOpen }: { files: FileDiff[]; onOpen: () => void }) {
  const { t } = useTranslation();
  const sortedFiles = sortChangedFiles(files);
  const totals = changedFileTotals(files);

  return (
    <div className="composer-status-segment composer-changed-files-segment">
      <div className="composer-status-popover composer-changed-files-popover" role="dialog">
        <div className="composer-changed-files-list">
          {sortedFiles.map((file) => (
            <div className="composer-changed-file-row" key={file.path} title={file.path}>
              <span className={`composer-changed-file-icon is-${file.status}`}>
                {changedFileIcon(file.status)}
              </span>
              <span className="composer-changed-file-path">{file.path}</span>
              <span className="composer-change-count additions">+{file.additions}</span>
              <span className="composer-change-count deletions">-{file.deletions}</span>
            </div>
          ))}
        </div>
        <button type="button" className="composer-changed-files-open" onClick={onOpen}>
          {t("Open changed files")}
        </button>
      </div>
      <button
        type="button"
        className="composer-status-trigger composer-changed-files-trigger"
        onClick={onOpen}
        aria-haspopup="dialog"
        aria-label={t("{{count}} files changed", { count: files.length })}
      >
        <span className="composer-changed-files-icon" aria-hidden="true">
          <Circle size={15} />
        </span>
        <span>{t("{{count}} files changed", { count: files.length })}</span>
        <span className="composer-change-count additions">+{totals.additions}</span>
        <span className="composer-change-count deletions">-{totals.deletions}</span>
      </button>
    </div>
  );
}

export function ComposerStatusCapsule({
  todos,
  changedFiles,
  onOpenChangedFiles,
}: {
  todos: TodoItem[];
  changedFiles: FileDiff[];
  onOpenChangedFiles: () => void;
}) {
  if (todos.length === 0 && changedFiles.length === 0) return null;

  return (
    <div className="composer-status-capsule">
      {todos.length > 0 ? <TodoStatusSegment todos={todos} /> : null}
      {todos.length > 0 && changedFiles.length > 0 ? (
        <span className="composer-status-divider" aria-hidden="true" />
      ) : null}
      {changedFiles.length > 0 ? (
        <ChangedFilesStatusSegment files={changedFiles} onOpen={onOpenChangedFiles} />
      ) : null}
    </div>
  );
}
