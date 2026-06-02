import { describe, it, expect } from "vitest";
import { computeGraphLayout } from "./gitGraph";
import type { GitCommitItem } from "../types/api";

function commit(sha: string, parents: string[], refs: string[] = []): GitCommitItem {
  return {
    sha,
    author: "test",
    date: "2024-01-01T00:00:00Z",
    message: `commit ${sha}`,
    parents,
    refs,
  };
}

describe("computeGraphLayout", () => {
  it("should handle an empty list", () => {
    const result = computeGraphLayout([], "abc");
    expect(result).toEqual([]);
  });

  it("should handle a single commit", () => {
    const commits = [commit("a", [])];
    const result = computeGraphLayout(commits, "a");
    expect(result).toHaveLength(1);
    expect(result[0].column).toBe(0);
    expect(result[0].commit.sha).toBe("a");
  });

  it("should handle linear history (same column)", () => {
    // a ← b ← c (c newest)
    const commits = [commit("c", ["b"]), commit("b", ["a"]), commit("a", [])];
    const result = computeGraphLayout(commits, "c");
    expect(result).toHaveLength(3);
    expect(result[0].column).toBe(0); // c
    expect(result[1].column).toBe(0); // b
    expect(result[2].column).toBe(0); // a
    // lines: what continues BELOW each row
    expect(result[0].lines[0]).toBe("b"); // c → b
    expect(result[1].lines[0]).toBe("a"); // b → a
    expect(result[2].lines[0]).toBeNull(); // a terminates
  });

  it("should handle a fork (two children of same parent)", () => {
    //   C (main, child of A)  ← fork curve appears HERE
    //   D (feature, child of A)
    //   A (parent)
    // Commits from git log (newest first): C, D, A
    const commits = [commit("C", ["A"]), commit("D", ["A"]), commit("A", [])];
    const result = computeGraphLayout(commits, "C");

    expect(result).toHaveLength(3);
    // C is on the main line (col 0) — carries the fork curve
    expect(result[0].commit.sha).toBe("C");
    expect(result[0].column).toBe(0);
    expect(result[0].merges).toHaveLength(1);
    expect(result[0].merges[0].fromCol).toBe(0);
    expect(result[0].merges[0].toCol).toBe(1);
    expect(result[0].lines[0]).toBe("A");
    expect(result[0].lines[1]).toBeNull(); // fork lane terminating stub
    // D is on the feature branch (col 1) — no merge at its row
    expect(result[1].commit.sha).toBe("D");
    expect(result[1].column).toBe(1);
    expect(result[1].merges).toHaveLength(0);
    expect(result[1].lines[1]).toBeNull();
    // A is on the main line (col 0)
    expect(result[2].commit.sha).toBe("A");
    expect(result[2].column).toBe(0);
  });

  it("should handle a merge and keep main line continuous", () => {
    //     M  (merge of C and E, parents: [C, E])
    //    / \
    //   C   E
    //    \ /
    //     A
    const commits = [
      commit("M", ["C", "E"]),
      commit("E", ["A"]),
      commit("C", ["A"]),
      commit("A", []),
    ];
    const result = computeGraphLayout(commits, "M");

    expect(result).toHaveLength(4);
    // M is at col 0 (main line), with merge curve from col 1 (E's lane)
    expect(result[0].commit.sha).toBe("M");
    expect(result[0].column).toBe(0);
    expect(result[0].merges).toHaveLength(1);
    expect(result[0].merges[0].fromCol).toBe(1);
    expect(result[0].merges[0].toCol).toBe(0);

    // E is at col 1 (feature branch)
    expect(result[1].commit.sha).toBe("E");
    expect(result[1].column).toBe(1);

    // C is at col 0 (main line), with merge curve from col 1
    expect(result[2].commit.sha).toBe("C");
    expect(result[2].column).toBe(0);
    expect(result[2].merges).toHaveLength(1);
    expect(result[2].merges[0].fromCol).toBe(1);
    expect(result[2].merges[0].toCol).toBe(0);

    // A is at col 0 (main line stays continuous)
    expect(result[3].commit.sha).toBe("A");
    expect(result[3].column).toBe(0);
  });

  it("should assign ref labels", () => {
    const commits = [commit("b", ["a"], ["HEAD -> main"]), commit("a", [])];
    const result = computeGraphLayout(commits, "b");
    expect(result[0].refLabels).toHaveLength(1);
    expect(result[0].refLabels[0].label).toBe("main");
    expect(result[0].refLabels[0].isHead).toBe(true);
  });
});
