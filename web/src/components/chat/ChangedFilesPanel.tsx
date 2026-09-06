import { useEffect, useRef, useState } from "react";
import { ChevronDown, ChevronRight, FileEdit, FilePlus, FileX, Loader2, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { FileDiff, SessionFileDiff } from "../../types/api";
import { changedFileTotals, sortChangedFiles } from "../../utils/changedFiles";
import {
  DiffRenderer,
  DiffScrollProvider,
  useDiffCollapseContext,
  DiffCollapseProvider,
} from "../renderers/DiffRenderer";
import { Button, IconButton } from "../ui";

function statusIcon(status: FileDiff["status"]) {
  if (status === "added") return <FilePlus size={14} aria-hidden="true" />;
  if (status === "deleted") return <FileX size={14} aria-hidden="true" />;
  return <FileEdit size={14} aria-hidden="true" />;
}

function statusLabel(status: FileDiff["status"]) {
  switch (status) {
    case "added":
      return "Added";
    case "deleted":
      return "Deleted";
    default:
      return "Modified";
  }
}

function ChangedFileSection({ file }: { file: FileDiff & { diff?: string } }) {
  const { t } = useTranslation();
  const { allExpanded } = useDiffCollapseContext();
  const [expanded, setExpanded] = useState(true);

  useEffect(() => {
    if (allExpanded !== null) {
      const id = requestAnimationFrame(() => setExpanded(allExpanded));
      return () => cancelAnimationFrame(id);
    }
  }, [allExpanded]);

  return (
    <section className="changed-files-diff-section">
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="changed-files-diff-header"
        onClick={() => setExpanded((current) => !current)}
        aria-expanded={expanded}
      >
        {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        <span className={`changed-files-status-icon is-${file.status}`}>
          {statusIcon(file.status)}
        </span>
        <span className="changed-files-diff-path" title={file.path}>
          {file.path}
        </span>
        <span className="changed-files-status-label">{t(statusLabel(file.status))}</span>
        <span className="composer-change-count additions">+{file.additions}</span>
        <span className="composer-change-count deletions">-{file.deletions}</span>
      </Button>
      {expanded ? (
        <div className="changed-files-diff-body">
          {file.diff ? (
            <DiffRenderer diff={file.diff} filepath={file.path} compact />
          ) : (
            <p className="changed-files-empty-diff">
              {t("No diff content (binary or empty file)")}
            </p>
          )}
        </div>
      ) : null}
    </section>
  );
}

function DiffToolbar({ fileCount }: { fileCount: number }) {
  const { t } = useTranslation();
  const { allExpanded, toggleAll } = useDiffCollapseContext();
  const expanded = allExpanded !== false;

  return (
    <div className="changed-files-diff-toolbar">
      <span>{t("Changed Files ({{count}})", { count: fileCount })}</span>
      <Button type="button" variant="ghost" size="sm" onClick={toggleAll}>
        {expanded ? t("Hide all diffs") : t("Show all diffs")}
      </Button>
    </div>
  );
}

export function ChangedFilesPanel({
  summaryFiles,
  files,
  loading,
  error,
  onClose,
}: {
  summaryFiles: FileDiff[];
  files: SessionFileDiff[];
  loading: boolean;
  error: string | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const summaries = sortChangedFiles(files);
  const totals = changedFileTotals(summaryFiles);

  return (
    <aside className="changed-files-panel" aria-label={t("Changed file diff")}>
      <div className="changed-files-panel-header">
        <div className="changed-files-panel-heading">
          <strong>{t("Changed Files")}</strong>
          <span>{t("{{count}} files changed", { count: summaryFiles.length })}</span>
          <span className="composer-change-count additions">+{totals.additions}</span>
          <span className="composer-change-count deletions">-{totals.deletions}</span>
        </div>
        <IconButton
          label={t("Close changed files")}
          size="sm"
          onClick={onClose}
          title={t("Close changed files")}
        >
          <X size={16} />
        </IconButton>
      </div>
      <DiffScrollProvider scrollRef={scrollRef} contentRef={contentRef}>
        <div ref={scrollRef} className="changed-files-panel-scroll">
          <div ref={contentRef} className="changed-files-panel-content">
            {loading ? (
              <div className="changed-files-panel-state">
                <Loader2 size={18} className="animate-spin" />
                <span>{t("Loading changed files…")}</span>
              </div>
            ) : error ? (
              <div className="changed-files-panel-state is-error">{error}</div>
            ) : summaries.length === 0 ? (
              <div className="changed-files-panel-state">{t("No agent changes yet")}</div>
            ) : (
              <DiffCollapseProvider>
                <DiffToolbar fileCount={summaries.length} />
                <div className="changed-files-diff-list">
                  {summaries.map((file) => (
                    <ChangedFileSection file={file} key={file.path} />
                  ))}
                </div>
              </DiffCollapseProvider>
            )}
          </div>
        </div>
      </DiffScrollProvider>
    </aside>
  );
}
