/**
 * Git graph layout algorithm.
 *
 * Takes a chronologically ordered list of commits (newest first) with parent
 * SHAs and computes a column-based lane layout for rendering branch topology.
 */

import type { GitCommitItem } from "../types/api";

/** Color palette for branch lanes — rotates if more lanes than colors. */
export const LANE_COLORS = [
  "#4caf50",
  "#2196f3",
  "#ff9800",
  "#e91e63",
  "#9c27b0",
  "#00bcd4",
  "#ff5722",
  "#3f51b5",
  "#8bc34a",
  "#f44336",
  "#009688",
  "#cddc39",
];

export interface GraphRow {
  commit: GitCommitItem;
  /** Which column the commit dot is drawn on. */
  column: number;
  /**
   * Columns that have a vertical line passing through this row.
   * Key = column index, value = occupant SHA (line continues down)
   * or `null` (line terminates at this row).
   */
  lines: Record<number, string | null>;
  /**
   * Horizontal connections between columns at this row.
   * fromCol → toCol (= this commit's column).
   */
  merges: { fromCol: number; toCol: number }[];
  /** Ref labels (branch / tag names) to display next to this commit. */
  refLabels: { column: number; label: string; isHead: boolean }[];
}

export function computeGraphLayout(
  commits: GitCommitItem[],
  headSha: string,
): GraphRow[] {
  if (commits.length === 0) return [];

  // sha → commit & child maps (for bookkeeping)
  const commitMap = new Map<string, GitCommitItem>();
  for (const c of commits) commitMap.set(c.sha, c);
  const childrenOf = new Map<string, string[]>();
  for (const c of commits) {
    for (const p of c.parents ?? []) {
      if (!childrenOf.has(p)) childrenOf.set(p, []);
      childrenOf.get(p)!.push(c.sha);
    }
  }

  // Lane state: lanes[i] = SHA of the commit occupying column i (or null).
  const lanes: (string | null)[] = [];
  // SHA → column index.
  const shaToCol = new Map<string, number>();

  const results: GraphRow[] = [];

  for (const commit of commits) {
    const sha = commit.sha;
    const parents = commit.parents ?? [];
    const refs = commit.refs ?? [];

    // ── 1. Find this commit's column ──────────────────────────────────
    // Only use shaToCol (set by a child processed earlier).
    // Fresh commits (not placed by any child) get a new lane.
    let col = shaToCol.get(sha) ?? -1;
    if (col === -1) {
      col = lanes.indexOf(null);
      if (col === -1) {
        col = lanes.length;
        lanes.push(null);
      }
    }
    while (col >= lanes.length) lanes.push(null);

    // ── 2. Snapshot lanes BEFORE parent update ────────────────────────
    const beforeLines = new Map<number, string>();
    for (let i = 0; i < lanes.length; i++) {
      if (lanes[i] !== null) beforeLines.set(i, lanes[i]!);
    }

    // ── 3. Clean up old mapping if commit moved columns ───────────────
    const oldCol = shaToCol.get(sha);
    if (oldCol !== undefined && oldCol !== col && lanes[oldCol] === sha) {
      lanes[oldCol] = null;
    }

    // ── 4. Place this commit at its column ────────────────────────────
    lanes[col] = sha;
    shaToCol.set(sha, col);

    // ── 5. Update lanes for parents & record merge curves ─────────────
    const merges: GraphRow["merges"] = [];

    if (parents.length === 0) {
      // Root commit — free the lane.
      lanes[col] = null;
      shaToCol.delete(sha);
    } else {
      const firstParent = parents[0];
      const fpCol = shaToCol.get(firstParent);

      if (fpCol !== undefined && fpCol !== col) {
        if (col < fpCol) {
          // Merge: commit on LEFT absorbs parent's lane.
          merges.push({ fromCol: fpCol, toCol: col });
          lanes[col] = firstParent;
          shaToCol.set(firstParent, col);
          lanes[fpCol] = null;
          shaToCol.delete(sha);
        } else {
          // Fork (col > fpCol): parent continues at fpCol, child stays at col.
          // The fork curve is drawn at this commit's row (the child's row).
          merges.push({ fromCol: fpCol, toCol: col });

          // Free the feature lane below this commit.
          lanes[col] = null;
          shaToCol.delete(sha);
        }
      } else {
        // Same lane (or parent not tracked yet).
        if (fpCol === undefined || fpCol === col) {
          lanes[col] = firstParent;
          shaToCol.set(firstParent, col);
        }
      }

      // Additional parents each get their own lane → merge curve.
      for (let i = 1; i < parents.length; i++) {
        const ps = parents[i];
        let pc = shaToCol.get(ps);
        if (pc === undefined) {
          pc = lanes.indexOf(null);
          if (pc === -1) {
            pc = lanes.length;
            lanes.push(null);
          }
          lanes[pc] = ps;
          shaToCol.set(ps, pc);
        }
        if (pc !== col) {
          merges.push({ fromCol: pc, toCol: col });
        }
      }
    }

    // ── 6. Snapshot lanes AFTER parent update ─────────────────────────
    const afterLines = new Map<number, string>();
    for (let i = 0; i < lanes.length; i++) {
      if (lanes[i] !== null) afterLines.set(i, lanes[i]!);
    }

    // ── 7. Compute `lines` — union of before & after ──────────────────
    const lines: Record<number, string | null> = {};
    const allCols = new Set([...beforeLines.keys(), ...afterLines.keys(), col]);
    for (const c of allCols) {
      const before = beforeLines.get(c);
      const after = afterLines.get(c);
      if (before !== undefined && after === undefined) {
        lines[c] = null; // line terminates here
      } else if (after !== undefined) {
        lines[c] = after; // line continues below
      } else {
        lines[c] = null; // edge case: own column only
      }
    }

    // ── 8. Ref labels ─────────────────────────────────────────────────
    const refLabels: GraphRow["refLabels"] = [];
    for (const ref of refs) {
      const isHead = ref === headSha || ref.startsWith("HEAD");
      let label = ref;
      if (label.startsWith("HEAD -> ")) {
        label = label.slice(8);
        refLabels.push({ column: col, label, isHead: true });
        continue;
      }
      if (label.startsWith("tag: ")) label = label.slice(5);
      refLabels.push({ column: col, label, isHead });
    }

    results.push({ commit, column: col, lines, merges, refLabels });
  }

  return results;
}
