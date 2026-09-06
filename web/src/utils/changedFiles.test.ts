import { describe, expect, it } from "vitest";

import type { MessageRecord } from "../types/api";
import {
  changedFileTotals,
  latestChangedFiles,
  parseChangedFileSummaries,
  sortChangedFiles,
} from "./changedFiles";

function record(fileDiffs?: string): MessageRecord {
  return {
    message: {} as MessageRecord["message"],
    app_data: { file_diffs: fileDiffs },
  };
}

describe("changed file summaries", () => {
  it("normalizes the persisted snapshot shape", () => {
    expect(
      parseChangedFileSummaries(
        JSON.stringify([
          { file: "src/main.rs", additions: 4, deletions: 2, status: "modified" },
          { path: "src/new.rs", additions: 3, deletions: 0, status: "added" },
        ]),
      ),
    ).toEqual([
      { path: "src/main.rs", status: "modified", additions: 4, deletions: 2 },
      { path: "src/new.rs", status: "added", additions: 3, deletions: 0 },
    ]);
  });

  it("uses the latest explicit snapshot, including an empty snapshot", () => {
    expect(
      latestChangedFiles([
        record(JSON.stringify([{ file: "old.rs", additions: 1, deletions: 0 }])),
        record("[]"),
      ]),
    ).toEqual([]);
  });

  it("sorts like the TUI and calculates totals", () => {
    const files = sortChangedFiles([
      { path: "z.rs", status: "deleted", additions: 0, deletions: 2 },
      { path: "b.rs", status: "added", additions: 5, deletions: 0 },
      { path: "a.rs", status: "modified", additions: 2, deletions: 1 },
    ]);
    expect(files.map((file) => file.path)).toEqual(["a.rs", "b.rs", "z.rs"]);
    expect(changedFileTotals(files)).toEqual({ additions: 7, deletions: 3 });
  });
});
