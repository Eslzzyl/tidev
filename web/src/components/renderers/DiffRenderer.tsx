import {
  useState,
  useRef,
  useEffect,
  useMemo,
  createContext,
  useContext,
  useCallback,
} from "react";
import { ChevronDown, ChevronRight, Expand, Minus } from "lucide-react";
import type { HLJSApi } from "highlight.js";
import { useTranslation } from "react-i18next";

interface Props {
  diff: string;
  filepath: string;
  /** When true, render in a more compact inline style for chat messages */
  compact?: boolean;
}

const WIDE_LAYOUT_THRESHOLD = 768;
const LARGE_FILE_THRESHOLD = 100; // lines - degrade highlighting for files larger than this

interface RawChange {
  type: "context" | "add" | "del";
  content: string;
  oldLine: number | null;
  newLine: number | null;
}

interface AlignedRow {
  left: {
    type: "context" | "del" | "empty";
    content: string;
    lineNum: number | null;
  };
  right: {
    type: "context" | "add" | "empty";
    content: string;
    lineNum: number | null;
  };
}

interface AlignedHunk {
  oldStart: number;
  newStart: number;
  rows: AlignedRow[];
}

function detectLanguage(fp: string): string {
  const ext = fp.split(".").pop()?.toLowerCase() || "";
  const langMap: Record<string, string> = {
    rs: "rust",
    ts: "typescript",
    tsx: "tsx",
    js: "javascript",
    jsx: "jsx",
    py: "python",
    go: "go",
    rb: "ruby",
    java: "java",
    kt: "kotlin",
    scala: "scala",
    swift: "swift",
    c: "c",
    h: "c",
    cpp: "cpp",
    hpp: "cpp",
    cc: "cpp",
    hh: "cpp",
    cxx: "cpp",
    cs: "csharp",
    php: "php",
    html: "html",
    css: "css",
    scss: "scss",
    sass: "sass",
    less: "less",
    sql: "sql",
    sh: "bash",
    bash: "bash",
    zsh: "bash",
    yaml: "yaml",
    yml: "yaml",
    toml: "toml",
    json: "json",
    xml: "xml",
    md: "markdown",
    mdx: "markdown",
    svelte: "svelte",
    vue: "vue",
    lua: "lua",
    dart: "dart",
    r: "r",
    zig: "zig",
    nim: "nim",
  };
  return langMap[ext] || "";
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/**
 * Parse a unified diff string into content-aligned hunks.
 *
 * Uses the same approach as OpenChamber's parseDiffToLines:
 * 1. Split changes into leftSide (context + del) and rightSide (context + add)
 * 2. Find alignment points by matching context lines (same content on both sides)
 * 3. Walk both sides simultaneously, using alignment points as anchors
 */
function parseDiffAligned(diffText: string): AlignedHunk[] {
  const hunks: AlignedHunk[] = [];
  const lines = diffText.split("\n");
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Skip headers
    if (
      line.startsWith("Index:") ||
      line.startsWith("===") ||
      line.startsWith("---") ||
      line.startsWith("+++")
    ) {
      i++;
      continue;
    }

    // Parse hunk header
    if (line.startsWith("@@")) {
      const match = line.match(/@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      const oldStart = match ? parseInt(match[1], 10) : 0;
      const newStart = match ? parseInt(match[2], 10) : 0;

      // Collect raw changes for this hunk
      const changes: RawChange[] = [];
      let oldLineNum = oldStart;
      let newLineNum = newStart;
      let j = i + 1;

      while (
        j < lines.length &&
        !lines[j].startsWith("@@") &&
        !lines[j].startsWith("Index:") &&
        !lines[j].startsWith("===")
      ) {
        const cl = lines[j];
        if (cl.startsWith("+")) {
          changes.push({
            type: "add",
            content: cl.slice(1),
            oldLine: null,
            newLine: newLineNum,
          });
          newLineNum++;
        } else if (cl.startsWith("-")) {
          changes.push({
            type: "del",
            content: cl.slice(1),
            oldLine: oldLineNum,
            newLine: null,
          });
          oldLineNum++;
        } else if (cl.startsWith(" ")) {
          changes.push({
            type: "context",
            content: cl.slice(1),
            oldLine: oldLineNum,
            newLine: newLineNum,
          });
          oldLineNum++;
          newLineNum++;
        }
        j++;
      }
      i = j;

      // --- Content-based alignment ---
      // Split into left side (context + del) and right side (context + add)
      const leftSide: {
        type: "context" | "del";
        lineNum: number;
        content: string;
      }[] = [];
      const rightSide: {
        type: "context" | "add";
        lineNum: number;
        content: string;
      }[] = [];

      for (const ch of changes) {
        if (ch.type === "context") {
          leftSide.push({
            type: "context",
            lineNum: ch.oldLine!,
            content: ch.content,
          });
          rightSide.push({
            type: "context",
            lineNum: ch.newLine!,
            content: ch.content,
          });
        } else if (ch.type === "del") {
          leftSide.push({
            type: "del",
            lineNum: ch.oldLine!,
            content: ch.content,
          });
        } else if (ch.type === "add") {
          rightSide.push({
            type: "add",
            lineNum: ch.newLine!,
            content: ch.content,
          });
        }
      }

      // Find alignment points: context lines with the same content on both sides
      const alignmentPoints: { leftIdx: number; rightIdx: number }[] = [];
      for (let li = 0; li < leftSide.length; li++) {
        const l = leftSide[li];
        if (l.type === "context") {
          const foundRi = rightSide.findIndex(
            (r, rIdx) =>
              r.type === "context" &&
              r.content === l.content &&
              !alignmentPoints.some((ap) => ap.rightIdx === rIdx),
          );
          if (foundRi >= 0) {
            alignmentPoints.push({ leftIdx: li, rightIdx: foundRi });
          }
        }
      }
      alignmentPoints.sort((a, b) => a.leftIdx - b.leftIdx);

      // Walk both sides using alignment points as anchors
      const rows: AlignedRow[] = [];
      let leftIdx = 0;
      let rightIdx = 0;
      let alignIdx = 0;

      while (leftIdx < leftSide.length || rightIdx < rightSide.length) {
        const nextAlign = alignIdx < alignmentPoints.length ? alignmentPoints[alignIdx] : null;

        if (nextAlign && leftIdx === nextAlign.leftIdx && rightIdx === nextAlign.rightIdx) {
          // Alignment point: both sides have this context line
          rows.push({
            left: {
              type: "context",
              content: leftSide[leftIdx].content,
              lineNum: leftSide[leftIdx].lineNum,
            },
            right: {
              type: "context",
              content: rightSide[rightIdx].content,
              lineNum: rightSide[rightIdx].lineNum,
            },
          });
          leftIdx++;
          rightIdx++;
          alignIdx++;
        } else {
          const needLeft = leftIdx < leftSide.length && (!nextAlign || leftIdx < nextAlign.leftIdx);
          const needRight =
            rightIdx < rightSide.length && (!nextAlign || rightIdx < nextAlign.rightIdx);

          if (needLeft && needRight) {
            // Both sides have content before next alignment — pair them 1:1
            rows.push({
              left: {
                type: leftSide[leftIdx].type,
                content: leftSide[leftIdx].content,
                lineNum: leftSide[leftIdx].lineNum,
              },
              right: {
                type: rightSide[rightIdx].type,
                content: rightSide[rightIdx].content,
                lineNum: rightSide[rightIdx].lineNum,
              },
            });
            leftIdx++;
            rightIdx++;
          } else if (needLeft) {
            // Only left has content — empty right
            rows.push({
              left: {
                type: leftSide[leftIdx].type,
                content: leftSide[leftIdx].content,
                lineNum: leftSide[leftIdx].lineNum,
              },
              right: { type: "empty", content: "", lineNum: null },
            });
            leftIdx++;
          } else if (needRight) {
            // Only right has content — empty left
            rows.push({
              left: { type: "empty", content: "", lineNum: null },
              right: {
                type: rightSide[rightIdx].type,
                content: rightSide[rightIdx].content,
                lineNum: rightSide[rightIdx].lineNum,
              },
            });
            rightIdx++;
          } else {
            // Should not reach here, but break to avoid infinite loop
            break;
          }
        }
      }

      hunks.push({ oldStart, newStart, rows });
    } else {
      i++;
    }
  }

  return hunks;
}

export function DiffRenderer({ diff, filepath, compact = false }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [isWide, setIsWide] = useState(
    typeof window !== "undefined" && window.innerWidth >= WIDE_LAYOUT_THRESHOLD,
  );
  const [hljs, setHljs] = useState<HLJSApi | null>(null);

  // Dynamically load highlight.js on first render
  useEffect(() => {
    import("highlight.js").then((mod) => setHljs(mod.default));
  }, []);

  const language = useMemo(() => detectLanguage(filepath), [filepath]);
  const hunks = useMemo(() => parseDiffAligned(diff), [diff]);
  const totalLines = useMemo(() => hunks.reduce((sum, h) => sum + h.rows.length, 0), [hunks]);
  const useFullHighlight = totalLines <= LARGE_FILE_THRESHOLD;

  const highlightLine = useCallback(
    (line: string, lang: string, fullHighlight: boolean): string => {
      if (!lang || !fullHighlight || !hljs) return escapeHtml(line);
      try {
        const result = hljs.highlight(line, {
          language: lang,
          ignoreIllegals: true,
        });
        return result.value;
      } catch {
        return escapeHtml(line);
      }
    },
    [hljs],
  );

  useEffect(() => {
    const checkWidth = () => setIsWide(window.innerWidth >= WIDE_LAYOUT_THRESHOLD);
    window.addEventListener("resize", checkWidth);
    return () => window.removeEventListener("resize", checkWidth);
  }, []);

  const renderInlineLine = (row: AlignedRow) => {
    // For inline mode, prefer the side with content (del or add over empty)
    const hasDel = row.left.type === "del";
    const hasAdd = row.right.type === "add";
    const isContext = row.left.type === "context";

    if (isContext) {
      return (
        <div className="flex min-h-[22px] font-mono text-xs leading-[22px]">
          <span className="w-8 shrink-0 select-none text-right text-neutral-400">
            {row.left.lineNum}
          </span>
          <span className="w-4 shrink-0 select-none text-center text-neutral-400"> </span>
          <span className="flex-1 whitespace-pre-wrap break-all pl-1 text-neutral-800 dark:text-neutral-200">
            {highlightLine(row.left.content, language, useFullHighlight)}
          </span>
        </div>
      );
    }

    if (hasDel) {
      return (
        <div className="flex min-h-[22px] font-mono text-xs leading-[22px] bg-red-50 dark:bg-red-950/40">
          <span className="w-8 shrink-0 select-none text-right text-neutral-400">
            {row.left.lineNum}
          </span>
          <span className="w-4 shrink-0 select-none text-center text-red-500">-</span>
          <span className="flex-1 whitespace-pre-wrap break-all pl-1 text-neutral-800 dark:text-neutral-200">
            {highlightLine(row.left.content, language, useFullHighlight)}
          </span>
        </div>
      );
    }

    if (hasAdd) {
      return (
        <div className="flex min-h-[22px] font-mono text-xs leading-[22px] bg-green-50 dark:bg-green-950/40">
          <span className="w-8 shrink-0 select-none text-right text-neutral-400">
            {row.right.lineNum}
          </span>
          <span className="w-4 shrink-0 select-none text-center text-green-600">+</span>
          <span className="flex-1 whitespace-pre-wrap break-all pl-1 text-neutral-800 dark:text-neutral-200">
            {highlightLine(row.right.content, language, useFullHighlight)}
          </span>
        </div>
      );
    }

    return null;
  };

  const renderTwoColumn = (rows: AlignedRow[]) => (
    <div>
      {rows.map((row, idx) => (
        <div
          key={idx}
          className="flex border-b border-neutral-100 last:border-b-0 dark:border-neutral-800"
        >
          {/* Left side (old) */}
          <div
            className={`flex min-h-[22px] flex-1 min-w-0 font-mono text-xs leading-[22px] border-r border-neutral-200 dark:border-neutral-700 ${
              row.left.type === "del" ? "bg-red-50 dark:bg-red-950/40" : ""
            }`}
          >
            <span className="w-8 shrink-0 select-none text-right text-neutral-400">
              {row.left.lineNum ?? ""}
            </span>
            <span
              className={`w-4 shrink-0 select-none text-center ${
                row.left.type === "del" ? "text-red-500" : "text-neutral-400"
              }`}
            >
              {row.left.type === "del" ? "-" : row.left.type === "empty" ? "" : " "}
            </span>
            <span className="flex-1 whitespace-pre-wrap break-all pl-1 text-neutral-800 dark:text-neutral-200">
              {highlightLine(row.left.content, language, useFullHighlight)}
            </span>
          </div>

          {/* Right side (new) */}
          <div
            className={`flex min-h-[22px] flex-1 min-w-0 font-mono text-xs leading-[22px] ${
              row.right.type === "add" ? "bg-green-50 dark:bg-green-950/40" : ""
            }`}
          >
            <span className="w-8 shrink-0 select-none text-right text-neutral-400">
              {row.right.lineNum ?? ""}
            </span>
            <span
              className={`w-4 shrink-0 select-none text-center ${
                row.right.type === "add" ? "text-green-600" : "text-neutral-400"
              }`}
            >
              {row.right.type === "add" ? "+" : row.right.type === "empty" ? "" : " "}
            </span>
            <span className="flex-1 whitespace-pre-wrap break-all pl-1 text-neutral-800 dark:text-neutral-200">
              {highlightLine(row.right.content, language, useFullHighlight)}
            </span>
          </div>
        </div>
      ))}
    </div>
  );

  if (!diff) return null;

  return (
    <div
      ref={containerRef}
      className={`rounded-lg border border-neutral-200 dark:border-neutral-800 ${
        compact ? "text-xs" : ""
      }`}
    >
      {isWide
        ? hunks.map((hunk, idx) => (
            <div
              key={idx}
              className="border-b border-neutral-200 last:border-b-0 dark:border-neutral-800"
            >
              {renderTwoColumn(hunk.rows)}
            </div>
          ))
        : hunks.map((hunk, idx) => (
            <div
              key={idx}
              className="border-b border-neutral-200 last:border-b-0 dark:border-neutral-800"
            >
              {hunk.rows.map((row, rowIdx) => (
                <div key={rowIdx}>{renderInlineLine(row)}</div>
              ))}
            </div>
          ))}
    </div>
  );
}

/**
 * Collapsible diff file wrapper - shows file header with expand/collapse.
 */
export function CollapsibleDiffFile({
  diff,
  filepath,
  defaultExpanded = true,
  hideCollapseToggle = false,
}: Props & { defaultExpanded?: boolean; hideCollapseToggle?: boolean }) {
  const { t } = useTranslation();
  const { allExpanded } = useDiffCollapseContext();
  const [localExpanded, setLocalExpanded] = useState(defaultExpanded);

  // Sync with global collapse/expand all
  useEffect(() => {
    if (allExpanded !== null) {
      const id = requestAnimationFrame(() => setLocalExpanded(allExpanded));
      return () => cancelAnimationFrame(id);
    }
  }, [allExpanded]);

  const isExpanded = hideCollapseToggle ? true : localExpanded;
  const totalLines = useMemo(() => diff.split("\n").length, [diff]);

  return (
    <div className="overflow-hidden rounded-lg border border-neutral-200 dark:border-neutral-800">
      {/* File header */}
      <button
        onClick={() => !hideCollapseToggle && setLocalExpanded(!isExpanded)}
        className="flex w-full items-center gap-2 border-b border-neutral-200 bg-neutral-50 px-3 py-2 text-left text-xs font-medium text-neutral-700 transition-colors hover:bg-neutral-100 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-300 dark:hover:bg-neutral-800"
      >
        {hideCollapseToggle ? null : isExpanded ? (
          <ChevronDown className="h-3.5 w-3.5 text-neutral-400" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 text-neutral-400" />
        )}
        <span className="flex-1 truncate">{filepath}</span>
        <span className="text-[10px] text-neutral-400">
          {t("{{count}} lines", { count: totalLines })}
        </span>
      </button>

      {/* Diff content */}
      {isExpanded && <DiffRenderer diff={diff} filepath={filepath} compact />}
    </div>
  );
}

// Context for "expand/collapse all" control
interface DiffCollapseContextType {
  allExpanded: boolean | null;
  toggleAll: () => void;
}

const DiffCollapseContext = createContext<DiffCollapseContextType>({
  allExpanded: null,
  toggleAll: () => {},
});

// eslint-disable-next-line react-refresh/only-export-components
export function useDiffCollapseContext() {
  return useContext(DiffCollapseContext);
}

/**
 * Provider that manages "expand/collapse all" for multiple diff files.
 */
export function DiffCollapseProvider({ children }: { children: React.ReactNode }) {
  const { t } = useTranslation();
  const [allExpanded, setAllExpanded] = useState<boolean | null>(null);

  const toggleAll = useCallback(() => {
    setAllExpanded((prev) => (prev === null ? false : !prev));
  }, []);

  return (
    <DiffCollapseContext.Provider value={{ allExpanded, toggleAll }}>
      <div>
        <div className="mb-2 flex items-center justify-between">
          <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400">
            {t("File Changes")}
          </span>
          <button
            onClick={toggleAll}
            className="flex items-center gap-1 rounded px-2 py-1 text-[10px] text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800"
          >
            {allExpanded === false ? (
              <>
                <Expand className="h-3 w-3" /> {t("Expand all")}
              </>
            ) : (
              <>
                <Minus className="h-3 w-3" /> {t("Collapse all")}
              </>
            )}
          </button>
        </div>
        {children}
      </div>
    </DiffCollapseContext.Provider>
  );
}
