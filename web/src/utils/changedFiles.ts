import type { FileDiff, MessageRecord } from "../types/api";

const FILE_STATUS = new Set<FileDiff["status"]>(["added", "modified", "deleted"]);

function stringValue(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function countValue(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : 0;
}

function normalizeFileDiff(value: unknown): FileDiff | null {
  if (!value || typeof value !== "object") return null;
  const item = value as Record<string, unknown>;
  const path = stringValue(item.path) ?? stringValue(item.file) ?? stringValue(item.file_path);
  if (!path) return null;

  const rawStatus = stringValue(item.status);
  const status = FILE_STATUS.has(rawStatus as FileDiff["status"])
    ? (rawStatus as FileDiff["status"])
    : "modified";

  return {
    path,
    status,
    additions: countValue(item.additions),
    deletions: countValue(item.deletions),
  };
}

/**
 * Parse the cumulative snapshot payload stored by tidev-core.
 * The backend historically called the path field `file`, while the web UI
 * uses `path`; accepting both keeps persisted sessions forward-compatible.
 */
export function parseChangedFileSummaries(value: string | null | undefined): FileDiff[] | null {
  if (value === null || value === undefined) return null;
  try {
    const parsed: unknown = JSON.parse(value);
    if (!Array.isArray(parsed)) return null;
    return parsed.flatMap((item) => {
      const normalized = normalizeFileDiff(item);
      return normalized ? [normalized] : [];
    });
  } catch {
    return null;
  }
}

/**
 * Return the latest cumulative snapshot attached to a session message.
 * An explicit empty array is meaningful and clears an older cached summary.
 */
export function latestChangedFiles(records: MessageRecord[]): FileDiff[] {
  for (let index = records.length - 1; index >= 0; index -= 1) {
    const parsed = parseChangedFileSummaries(records[index]?.app_data.file_diffs);
    if (parsed !== null) return parsed;
  }
  return [];
}

export function latestChangedFileSnapshotHash(records: MessageRecord[]): string | undefined {
  for (let index = records.length - 1; index >= 0; index -= 1) {
    const record = records[index];
    if (parseChangedFileSummaries(record?.app_data.file_diffs) !== null) {
      return record?.app_data.snapshot_hash ?? undefined;
    }
  }
  return undefined;
}

export function sortChangedFiles(files: FileDiff[]): FileDiff[] {
  const statusOrder: Record<FileDiff["status"], number> = {
    modified: 0,
    added: 1,
    deleted: 2,
  };
  return [...files].sort(
    (left, right) =>
      statusOrder[left.status] - statusOrder[right.status] || left.path.localeCompare(right.path),
  );
}

export function changedFileTotals(files: FileDiff[]): {
  additions: number;
  deletions: number;
} {
  return files.reduce(
    (totals, file) => ({
      additions: totals.additions + file.additions,
      deletions: totals.deletions + file.deletions,
    }),
    { additions: 0, deletions: 0 },
  );
}
