/**
 * SVG git graph renderer.
 *
 * Draws branch topology lines, commit nodes, merge/fork curves and ref labels
 * as a single SVG positioned behind the commit list.
 *
 * Key rendering rules:
 * - Each row draws vertical lines from its own TOP (rowTop) to either
 *   the NEXT row's TOP (if active) or its own CENTRE (if terminating).
 *   This keeps segments strictly within row boundaries — NO overlap
 *   between consecutive rows.
 * - Merge curves arc UPWARD (above the horizontal), fork curves arc
 *   DOWNWARD (below the horizontal). Both use cubic beziers with
 *   vertical end-tangents for smooth joins with column lines.
 * - A terminating column that is the SOURCE of a merge/fork curve suppresses
 *   its vertical stub entirely (the curve carries the connection).
 */
import React from "react";
import type { GraphRow } from "../../lib/gitGraph";
import { LANE_COLORS } from "../../lib/gitGraph";

// ── Constants ─────────────────────────────────────────────────────────────

const LANE_W = 18;
const ROW_H = 56;
const DOT_R = 4;
const MERGE_DOT_R = 5.5;
const PAD_Y = 4;
const PAD_LEFT = 8;
const PAD_RIGHT = 4;
const LINE_W = 2;

// ── Geometry helpers ──────────────────────────────────────────────────────

function laneColor(col: number): string {
  return LANE_COLORS[col % LANE_COLORS.length];
}

function laneX(col: number): number {
  return PAD_LEFT + col * LANE_W + LANE_W / 2;
}

/** Top edge of the given row (pixel Y). */
function rowTop(idx: number): number {
  return PAD_Y + idx * ROW_H;
}

/** Centre Y of the given row. */
function rowCtr(idx: number): number {
  return rowTop(idx) + ROW_H / 2;
}

/**
 * Smooth merge curve between two columns at the same row centre.
 *
 * Uses control points positioned ABOVE the endpoints so that the start
 * and end tangents are VERTICAL. This gives a smooth join with the
 * column's vertical line (which also has vertical tangent).
 *
 * The arch height is clamped so the curve stays within the row's
 * upper half, avoiding overlap with the previous row.
 */
function mergePath(fromCol: number, toCol: number, y: number): string {
  const x1 = laneX(fromCol);
  const x2 = laneX(toCol);
  const dx = Math.abs(x2 - x1);
  const arch = Math.min(Math.max(dx * 0.45, 6), ROW_H / 2 - 2);
  return `M ${x1} ${y} C ${x1} ${y - arch}, ${x2} ${y - arch}, ${x2} ${y}`;
}

/**
 * Fork curve — like mergePath but arches DOWNWARD so the curve
 * sits below the commit dot, visually indicating a branch-off.
 */
function forkPath(fromCol: number, toCol: number, y: number): string {
  const x1 = laneX(fromCol);
  const x2 = laneX(toCol);
  const dx = Math.abs(x2 - x1);
  const arch = Math.min(Math.max(dx * 0.45, 6), ROW_H / 2 - 2);
  return `M ${x1} ${y} C ${x1} ${y + arch}, ${x2} ${y + arch}, ${x2} ${y}`;
}

// ── Props / Exports ───────────────────────────────────────────────────────

interface GitGraphSVGProps {
  rows: GraphRow[];
  selectedSha: string | null;
  onSelectCommit: (sha: string) => void;
}

// eslint-disable-next-line react-refresh/only-export-components
export function getGraphWidth(rows: GraphRow[]): number {
  if (rows.length === 0) return PAD_LEFT + PAD_RIGHT + LANE_W;
  const maxCol = rows.reduce((m, r) => {
    const lineCols = Object.keys(r.lines).map(Number);
    const mergeCols = r.merges.flatMap((m) => [m.fromCol, m.toCol]);
    return Math.max(m, r.column, ...lineCols, ...mergeCols);
  }, 0);
  return PAD_LEFT + (maxCol + 1) * LANE_W + PAD_RIGHT;
}

export { ROW_H as GRAPH_ROW_HEIGHT };

// ── Component ─────────────────────────────────────────────────────────────

export const GitGraphSVG = React.memo(function GitGraphSVG({
  rows, selectedSha, onSelectCommit,
}: GitGraphSVGProps) {
  if (rows.length === 0) return null;

  const svgWidth = getGraphWidth(rows);
  const svgHeight = PAD_Y + rows.length * ROW_H + PAD_Y;

  return (
    <svg
      width={svgWidth}
      height={svgHeight}
      className="flex-shrink-0"
      style={{ minWidth: svgWidth }}
    >
      {rows.map((row, i) => {
        const cy = rowCtr(i);
        const col = row.column;
        const cx = laneX(col);
        const isSelected = row.commit.sha === selectedSha;
        const isMerge = (row.commit.parents?.length ?? 0) > 1;
        const dotR = isMerge ? MERGE_DOT_R : DOT_R;
        const dotColor = laneColor(col);

        // Merge-source columns (feature branch entering a merge):
        // fromCol > toCol — the curve arcs from right to left.
        // These get truncated (start at cy) to remove the extension
        // above the merge curve.
        const mergeSrcCols = new Set(
          row.merges.filter((m) => m.fromCol > m.toCol).map((m) => m.fromCol),
        );
        // Fork-target columns (feature branch where fork curve lands):
        // toCol where fromCol < toCol.
        const forkTgtCols = new Set(
          row.merges.filter((m) => m.fromCol < m.toCol).map((m) => m.toCol),
        );

        return (
          <g key={row.commit.sha}>
            {/* ── Vertical lines ────────────────────────────────────── */}
            {Object.entries(row.lines).map(([colStr, nextSha]) => {
              const c = Number(colStr);
              const x = laneX(c);
              const isActive = nextSha !== null;
              const isMergeSrc = mergeSrcCols.has(c);
              const isForkTgt = forkTgtCols.has(c);

              // Merge-source terminating stubs: the merge curve provides
              // the endpoint.  Fork sources & targets keep their stubs.
              if (!isActive && isMergeSrc) return null;

              // Merge-source active lines start at cy to avoid an
              // extension above the merge curve.  Fork-source lines go
              // the full row height — the fork curve branches off at cy.
              const y1 = isMergeSrc ? cy : rowTop(i);
              const y2 = isActive && i + 1 < rows.length
                ? rowTop(i + 1)
                : cy;

              if (y1 >= y2) return null;

              // Fork-target stubs match the fork curve's visual weight.
              const opacity = (isActive || isForkTgt) ? 0.55 : 0.35;

              return (
                <line
                  key={`l-${c}`}
                  x1={x}
                  y1={y1}
                  x2={x}
                  y2={y2}
                  stroke={laneColor(c)}
                  strokeWidth={LINE_W}
                  strokeOpacity={opacity}
                  strokeLinecap="butt"
                />
              );
            })}

            {/* ── Merge / Fork curves ──────────────────────────────── */}
            {row.merges.map((m, mi) => {
              const isFork = m.fromCol < m.toCol;
              const pathD = isFork
                ? forkPath(m.fromCol, m.toCol, cy)
                : mergePath(m.fromCol, m.toCol, cy);
              const rightCol = Math.max(m.fromCol, m.toCol);
              const mc = laneColor(rightCol);
              return (
                <g key={`m-${mi}`}>
                  <path
                    d={pathD}
                    fill="none"
                    stroke={mc}
                    strokeWidth={LINE_W}
                    strokeOpacity={0.5}
                    strokeLinecap="round"
                  />
                  {/* Filler dot at the source end for extra smoothness */}
                  <circle
                    cx={laneX(m.fromCol)}
                    cy={cy}
                    r={LINE_W * 0.7}
                    fill={mc}
                    opacity={0.5}
                  />
                  {/* For forks, also fill the target end */}
                  {isFork && (
                    <circle
                      cx={laneX(m.toCol)}
                      cy={cy}
                      r={LINE_W * 0.7}
                      fill={mc}
                      opacity={0.5}
                    />
                  )}
                </g>
              );
            })}
            {/* ── Merge outer ring ──────────────────────────────────── */}
            {isMerge && (
              <circle
                cx={cx} cy={cy}
                r={dotR + 2}
                fill="none"
                stroke={dotColor}
                strokeWidth={1.5}
                strokeOpacity={0.5}
              />
            )}

            {/* ── Invisible click area ──────────────────────────────── */}
            <circle
              cx={cx} cy={cy}
              r={Math.max(dotR + 4, 9)}
              fill="transparent"
              className="cursor-pointer"
              style={{ pointerEvents: "auto" }}
              onClick={() => onSelectCommit(row.commit.sha)}
            />

            {/* ── Commit dot ────────────────────────────────────────── */}
            <circle
              cx={cx} cy={cy}
              r={dotR}
              fill={dotColor}
              stroke={isSelected ? "#fff" : "none"}
              strokeWidth={isSelected ? 2.5 : 0}
              className="cursor-pointer"
              style={{ pointerEvents: "none" }}
            />
          </g>
        );
      })}
    </svg>
  );
});
